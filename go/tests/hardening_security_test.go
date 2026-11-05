package tests

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
)

// newSecurityEngine builds an engine with an "acct" workflow:
//
//	GET  /api/v1.0.0/acct/me      echoes the user_id the activity sees
//	POST /api/v1.0.0/acct/fail    fails with an internal error
//	POST /api/v1.0.0/acct/echo    echoes a JSON field
func newSecurityEngine(t *testing.T, apiKey string, opts models.EngineOptions, mw ...gin.HandlerFunc) *gin.Engine {
	t.Helper()
	gin.SetMode(gin.TestMode)
	engine := pkg.NewOctopusLiveEngine(apiKey)
	engine.SetOptions(opts)
	engine.RegisterActivity("Me", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"user_id": in["user_id"], "claims": in["_auth_claims"]}, nil
	})
	engine.RegisterActivity("Boom", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return nil, errors.New("pq: connection to db-prod-7 failed, password=hunter2")
	})
	engine.RegisterActivity("Echo", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"value": in["value"]}, nil
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{
		WorkflowName: "acct", Version: "1.0.0",
		API: models.APIEndpoints{
			{Route: "/api/v1.0.0/acct/me", Method: "GET", Steps: []models.WorkflowStep{{ID: "me", Activity: "Me", IsOutput: true}}},
			{Route: "/api/v1.0.0/acct/fail", Method: "POST", Steps: []models.WorkflowStep{{ID: "boom", Activity: "Boom"}}},
			{Route: "/api/v1.0.0/acct/echo", Method: "POST", Steps: []models.WorkflowStep{{ID: "echo", Activity: "Echo", IsOutput: true}}},
		},
	})
	for _, m := range mw {
		engine.Use(m)
	}
	router := gin.New()
	engine.MountRoutes(router)
	return router
}

func do(r *gin.Engine, method, path, body, remote string, hdr map[string]string) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	if remote != "" {
		req.RemoteAddr = remote
	}
	if body != "" {
		req.Header.Set("Content-Type", "application/json")
	}
	for k, v := range hdr {
		req.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	return w
}

func injectUser(user string) gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Set("workflow_injections", map[string]interface{}{"user_id": user})
		c.Next()
	}
}

// S1: values injected by the server (e.g. the authenticated user) must win
// over request parameters, and reserved keys can't be supplied by clients.
func TestInjectedIdentityCannotBeOverridden(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{}, injectUser("alice"))
	w := do(r, "GET", "/api/v1.0.0/acct/me?user_id=mallory", "", "", nil)
	var out map[string]interface{}
	_ = json.Unmarshal(w.Body.Bytes(), &out)
	if out["user_id"] != "alice" {
		t.Fatalf("activity saw user_id=%v, want alice (client override)", out["user_id"])
	}

	r = newSecurityEngine(t, "", models.EngineOptions{})
	w = do(r, "GET", `/api/v1.0.0/acct/me?_auth_claims=admin`, "", "", nil)
	_ = json.Unmarshal(w.Body.Bytes(), &out)
	if out["claims"] != nil {
		t.Fatalf("client-supplied _auth_claims reached the activity: %v", out["claims"])
	}
}

// S3: no hard-coded keys; keys only in the X-API-Key header by default.
func TestNoBackdoorAPIKeys(t *testing.T) {
	r := newSecurityEngine(t, "real-key", models.EngineOptions{})
	for _, k := range []string{"katz-secret-key-12345", "secret-key", "default-api-key"} {
		if w := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":1}`, "", map[string]string{"X-API-Key": k}); w.Code != http.StatusUnauthorized {
			t.Errorf("backdoor key %q accepted: %d", k, w.Code)
		}
	}
	if w := do(r, "POST", "/api/v1.0.0/acct/echo?api_key=real-key", `{"value":1}`, "", nil); w.Code != http.StatusUnauthorized {
		t.Errorf("api_key in query accepted by default: %d", w.Code)
	}
	if w := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":1}`, "", map[string]string{"X-API-Key": "real-key"}); w.Code != http.StatusOK {
		t.Errorf("valid header key rejected: %d", w.Code)
	}
	r = newSecurityEngine(t, "real-key", models.EngineOptions{AllowAPIKeyInQuery: true})
	if w := do(r, "POST", "/api/v1.0.0/acct/echo?api_key=real-key", `{"value":1}`, "", nil); w.Code != http.StatusOK {
		t.Errorf("opt-in query key rejected: %d", w.Code)
	}
}

// S4: tooling (history, visualizer, docs) is not public by default.
func TestToolingAccess(t *testing.T) {
	tooling := []string{"/api/jobs", "/api/visualizer/jobs", "/api/visualizer/domains", "/visualizer", "/docs", "/openapi.json", "/api/v1.0.0/acct/jobs"}
	r := newSecurityEngine(t, "", models.EngineOptions{})
	for _, p := range tooling {
		if w := do(r, "GET", p, "", "203.0.113.5:4000", nil); w.Code != http.StatusForbidden {
			t.Errorf("remote %s: %d, want 403", p, w.Code)
		}
		if w := do(r, "GET", p, "", "127.0.0.1:4000", nil); w.Code != http.StatusOK {
			t.Errorf("local %s: %d, want 200", p, w.Code)
		}
	}
	r = newSecurityEngine(t, "k", models.EngineOptions{})
	if w := do(r, "GET", "/api/jobs", "", "203.0.113.5:4000", map[string]string{"X-API-Key": "k"}); w.Code != http.StatusOK {
		t.Errorf("remote with key: %d", w.Code)
	}
	if w := do(r, "GET", "/api/jobs", "", "127.0.0.1:4000", nil); w.Code != http.StatusUnauthorized {
		t.Errorf("local without key when a key is set: %d", w.Code)
	}
	r = newSecurityEngine(t, "", models.EngineOptions{Tooling: models.ToolingOpen})
	if w := do(r, "GET", "/api/jobs", "", "203.0.113.5:4000", nil); w.Code != http.StatusOK {
		t.Errorf("open tooling: %d", w.Code)
	}
	r = newSecurityEngine(t, "", models.EngineOptions{Tooling: models.ToolingDisabled})
	if w := do(r, "GET", "/visualizer", "", "127.0.0.1:4000", nil); w.Code != http.StatusNotFound {
		t.Errorf("disabled tooling: %d", w.Code)
	}
}

// S4: declarative middlewares are no longer forced to skip the visualizer.
func TestMiddlewareCanProtectVisualizer(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{}, func(c *gin.Context) {
		if c.GetHeader("Authorization") == "" {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "UserNotAuthenticated"})
			return
		}
		c.Next()
	})
	if w := do(r, "GET", "/visualizer", "", "127.0.0.1:4000", nil); w.Code != http.StatusUnauthorized {
		t.Errorf("visualizer bypassed auth middleware: %d", w.Code)
	}
}

// S5: client job ids are ignored by default (they are idempotency keys).
func TestClientJobIDsIgnoredByDefault(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{})
	w1 := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":"first"}`, "", map[string]string{"X-Job-ID": "shared-id"})
	w2 := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":"second"}`, "", map[string]string{"X-Job-ID": "shared-id"})
	if id := w1.Header().Get("X-Job-ID"); id == "shared-id" || !strings.HasPrefix(id, "job_") {
		t.Errorf("client job id used: %q", id)
	}
	if !strings.Contains(w2.Body.String(), "second") {
		t.Errorf("second caller got a replayed response: %s", w2.Body.String())
	}
	// Opt-in keeps client ids; replay protection still scopes them per route.
	r = newSecurityEngine(t, "", models.EngineOptions{AllowClientJobIDs: true})
	w := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":"x"}`, "", map[string]string{"X-Job-ID": "tx-42"})
	if w.Header().Get("X-Job-ID") != "tx-42" {
		t.Errorf("opt-in client job id not used: %q", w.Header().Get("X-Job-ID"))
	}
	if w := do(r, "GET", "/api/v1.0.0/acct/me", "", "", map[string]string{"X-Job-ID": "tx-42"}); strings.Contains(w.Body.String(), `"value"`) {
		t.Errorf("job id reused across routes replayed another response: %s", w.Body.String())
	}
	if w := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value":"x"}`, "", map[string]string{"X-Job-ID": "<script>"}); w.Header().Get("X-Job-ID") == "<script>" {
		t.Error("unsafe client job id accepted")
	}
}

// S6: X-Forwarded-For is only trusted from configured proxies.
func TestForwardedForNotTrustedByDefault(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{})
	if w := do(r, "GET", "/api/jobs", "", "203.0.113.5:4000", map[string]string{"X-Forwarded-For": "127.0.0.1"}); w.Code != http.StatusForbidden {
		t.Errorf("spoofed X-Forwarded-For treated as local: %d", w.Code)
	}
	r = newSecurityEngine(t, "", models.EngineOptions{TrustedProxies: []string{"10.0.0.0/8"}})
	if w := do(r, "GET", "/api/jobs", "", "10.1.2.3:4000", map[string]string{"X-Forwarded-For": "127.0.0.1"}); w.Code != http.StatusOK {
		t.Errorf("trusted proxy's client IP ignored: %d", w.Code)
	}
}

// S8: unexpected errors are not sent to clients unless enabled.
func TestInternalErrorsNotLeaked(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{})
	w := do(r, "POST", "/api/v1.0.0/acct/fail", `{}`, "", nil)
	if w.Code != http.StatusInternalServerError || strings.Contains(w.Body.String(), "hunter2") || strings.Contains(w.Body.String(), "db-prod") {
		t.Fatalf("internal error leaked: %d %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "job_") {
		t.Errorf("generic error should carry the job id: %s", w.Body.String())
	}
	r = newSecurityEngine(t, "", models.EngineOptions{ExposeErrorDetails: true})
	if w := do(r, "POST", "/api/v1.0.0/acct/fail", `{}`, "", nil); !strings.Contains(w.Body.String(), "db-prod") {
		t.Errorf("opt-in error details missing: %s", w.Body.String())
	}
}

// R9: malformed JSON is rejected instead of running with empty input.
func TestMalformedJSONRejected(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{})
	if w := do(r, "POST", "/api/v1.0.0/acct/echo", `{"value": `, "", nil); w.Code != http.StatusBadRequest {
		t.Fatalf("malformed JSON: %d %s", w.Code, w.Body.String())
	}
	if w := do(r, "POST", "/api/v1.0.0/acct/echo", ``, "", nil); w.Code != http.StatusOK {
		t.Fatalf("empty body should be allowed: %d", w.Code)
	}
}

// S2: the visualizer must not build JavaScript from execution data (job ids,
// routes, workflow names) and must not default or read API keys from the URL.
// Verified in a browser: a job id of x');document.title='XSS-PWNED';// ran
// script on the old jobs list and is inert now.
func TestVisualizerDoesNotBuildScriptFromData(t *testing.T) {
	r := newSecurityEngine(t, "", models.EngineOptions{})
	html := do(r, "GET", "/visualizer", "", "127.0.0.1:4000", nil).Body.String()
	for _, bad := range []string{`onclick="selectJobFromModal(\'' + jId`, "katz-secret-key", "urlParams.get('api_key')", `'"</strong> Not Found` + "'"} {
		if strings.Contains(html, bad) {
			t.Errorf("visualizer still contains %q", bad)
		}
	}
	for _, good := range []string{"data-inspect-job=", "escapeHtml(targetJobId)"} {
		if !strings.Contains(html, good) {
			t.Errorf("visualizer missing %q", good)
		}
	}
}
