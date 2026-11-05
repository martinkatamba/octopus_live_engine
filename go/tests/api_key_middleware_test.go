package tests

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg"
)

// newKeyedEngine loads a blueprint whose ApiKeyAuthMiddleware entry is keysBlock.
func newKeyedEngine(t *testing.T, keysBlock string) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	dir := t.TempDir()
	bp := "workflow_name: keyed\nversion: \"1.0.0\"\nmiddlewares:\n  - name: ApiKeyAuthMiddleware\n" + keysBlock +
		"    excludes: [/health]\napi:\n  - route: /api/v1.0.0/keyed/ping\n    method: GET\n    steps:\n      - id: ping\n        activity: Ping\n        is_output: true\n"
	if err := os.WriteFile(filepath.Join(dir, "keyed.yaml"), []byte(bp), 0o644); err != nil {
		t.Fatal(err)
	}
	engine := pkg.NewOctopusLiveEngine("")
	engine.RegisterActivity("Ping", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"pong": true}, nil
	})
	if err := engine.LoadBlueprintsFromDirectory(dir); err != nil {
		t.Fatal(err)
	}
	router := gin.New()
	engine.MountRoutes(router)
	return router
}

// Only the blueprint's keys pass, in the configured header; with none configured every request is refused.
func TestApiKeyMiddlewareAcceptsOnlyConfiguredKeys(t *testing.T) {
	cases := map[string]struct {
		block    string
		accepted string
	}{
		"under config":  {"    config:\n      valid_keys: [partner-key-1]\n      header_name: X-Partner-Key\n", "partner-key-1"},
		"top level":     {"    valid_keys: [partner-key-1]\n    header_name: X-Partner-Key\n", "partner-key-1"},
		"none declared": {"", ""},
	}
	for name, tc := range cases {
		t.Run(name, func(t *testing.T) {
			r := newKeyedEngine(t, tc.block)
			url := "/api/v1.0.0/keyed/ping"
			header := "X-API-Key"
			if tc.accepted != "" {
				header = "X-Partner-Key"
			}
			for _, old := range []string{"default-api-key", "secret-key", "katz-secret-key-12345"} {
				if w := do(r, "GET", url, "", "", map[string]string{header: old}); w.Code != 401 {
					t.Fatalf("%s accepted the old built-in key %q: %d", header, old, w.Code)
				}
			}
			if w := do(r, "GET", url, "", "", nil); w.Code != 401 {
				t.Fatalf("no key: got %d, want 401", w.Code)
			}
			if tc.accepted != "" {
				if w := do(r, "GET", url, "", "", map[string]string{"X-Partner-Key": tc.accepted}); w.Code != 200 {
					t.Fatalf("configured key: got %d, want 200: %s", w.Code, w.Body.String())
				}
				if w := do(r, "GET", url, "", "", map[string]string{"X-API-Key": tc.accepted}); w.Code != 401 {
					t.Fatalf("configured key in the default header: got %d, want 401", w.Code)
				}
			}
			if w := do(r, "GET", "/health", "", "", nil); w.Code != 200 {
				t.Fatalf("/health: got %d, want 200", w.Code)
			}
		})
	}
}
