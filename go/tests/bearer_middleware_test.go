package tests

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg"
)

func hs256(t *testing.T, claims map[string]interface{}, secret, alg string) string {
	t.Helper()
	enc := func(v interface{}) string {
		b, _ := json.Marshal(v)
		return base64.RawURLEncoding.EncodeToString(b)
	}
	in := enc(map[string]string{"alg": alg, "typ": "JWT"}) + "." + enc(claims)
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(in))
	return in + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

func newJWTEngine(t *testing.T, secretBlock string) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	dir := t.TempDir()
	bp := "workflow_name: jwtwf\nversion: \"1.0.0\"\nmiddlewares:\n  - name: BearerAuthAndClaimsMiddleware\n" + secretBlock +
		"    excludes: [/health]\napi:\n  - route: /api/v1.0.0/jwtwf/me\n    method: GET\n    steps:\n      - id: me\n        activity: Me\n        is_output: true\n"
	if err := os.WriteFile(filepath.Join(dir, "jwtwf.yaml"), []byte(bp), 0o644); err != nil {
		t.Fatal(err)
	}
	engine := pkg.NewOctopusLiveEngine("")
	engine.RegisterActivity("Me", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"ok": true}, nil
	})
	if err := engine.LoadBlueprintsFromDirectory(dir); err != nil {
		t.Fatal(err)
	}
	router := gin.New()
	engine.MountRoutes(router)
	return router
}

// Tokens are verified with the configured HS256 secret; with no validator and no secret every token is refused.
func TestBearerMiddlewareVerifiesHS256OrRefuses(t *testing.T) {
	cases := map[string]struct{ block, env string }{
		"config secret":    {"    config:\n      jwt_secret: blueprint-secret-1\n", ""},
		"top-level secret": {"    jwt_secret: blueprint-secret-1\n", ""},
		"env secret":       {"", "blueprint-secret-1"},
		"no secret":        {"", ""},
	}
	for name, tc := range cases {
		t.Run(name, func(t *testing.T) {
			t.Setenv("OCTOPUS_JWT_SECRET", tc.env)
			r := newJWTEngine(t, tc.block)
			url := "/api/v1.0.0/jwtwf/me"
			later := float64(time.Now().Add(10 * time.Minute).Unix())
			bearer := func(tok string) map[string]string { return map[string]string{"Authorization": "Bearer " + tok} }

			refused := []string{
				"anything-at-all",
				hs256(t, map[string]interface{}{"sub": "mallory", "exp": later}, "wrong-secret", "HS256"),
				hs256(t, map[string]interface{}{"sub": "alice", "exp": float64(time.Now().Add(-time.Minute).Unix())}, "blueprint-secret-1", "HS256"),
				hs256(t, map[string]interface{}{"sub": "alice", "exp": later}, "blueprint-secret-1", "none"),
				hs256(t, map[string]interface{}{"sub": "alice"}, "octopus-live-engine-secret-key-2026", "HS256"),
			}
			for _, tok := range refused {
				if w := do(r, "GET", url, "", "", bearer(tok)); w.Code != 401 {
					t.Fatalf("token %q: got %d, want 401", tok, w.Code)
				}
			}
			want := 401
			if tc.block != "" || tc.env != "" {
				want = 200
			}
			good := hs256(t, map[string]interface{}{"sub": "alice", "exp": later}, "blueprint-secret-1", "HS256")
			if w := do(r, "GET", url, "", "", bearer(good)); w.Code != want {
				t.Fatalf("valid token: got %d, want %d: %s", w.Code, want, w.Body.String())
			}
			if w := do(r, "GET", "/health", "", "", nil); w.Code != 200 {
				t.Fatalf("/health: got %d", w.Code)
			}
		})
	}
}
