package tests

import (
	"context"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/adapters/outbound"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
)

func waitUntil(t *testing.T, what string, timeout time.Duration, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", what)
}

func traceStatus(t *testing.T, engine *pkg.OctopusLiveEngine, jobID, stepID string) string {
	st, err := engine.GetExecutionState(context.Background(), jobID)
	if err != nil {
		return ""
	}
	for _, tr := range st.Traces {
		if tr.StepID == stepID {
			return tr.Status
		}
	}
	return ""
}

// R4: timeout_seconds stops waiting for a hung activity.
func TestStepTimeoutEnforced(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	engine.RegisterActivity("Hang", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		select {
		case <-time.After(5 * time.Second):
		case <-ctx.Done():
		}
		return nil, nil
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "slow", Version: "1.0.0",
		Steps: []models.WorkflowStep{{ID: "hang", Activity: "Hang", TimeoutSeconds: 1}}})
	start := time.Now()
	st, err := engine.ExecuteSync(context.Background(), "slow", map[string]interface{}{})
	if err == nil || time.Since(start) > 3*time.Second {
		t.Fatalf("timeout not enforced: err=%v after %v", err, time.Since(start))
	}
	if st.ErrorCode == nil || *st.ErrorCode != 504 {
		t.Fatalf("error code = %v", st.ErrorCode)
	}
}

// R4: retry_policy retries transient errors but never rejections (4xx).
func TestStepRetryPolicy(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	var flaky, rejected atomic.Int32
	engine.RegisterActivity("Flaky", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		if flaky.Add(1) < 3 {
			return nil, errors.New("temporary outage")
		}
		return map[string]interface{}{"ok": true}, nil
	})
	engine.RegisterActivity("Reject", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		rejected.Add(1)
		return nil, services.NewValidationException("bad input")
	})
	retry := map[string]interface{}{"max_attempts": 3, "initial_interval_ms": 5}
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "retry", Version: "1.0.0",
		Steps: []models.WorkflowStep{{ID: "flaky", Activity: "Flaky", RetryPolicy: retry}}})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "reject", Version: "1.0.0",
		Steps: []models.WorkflowStep{{ID: "reject", Activity: "Reject", RetryPolicy: retry}}})

	st, err := engine.ExecuteSync(context.Background(), "retry", map[string]interface{}{})
	if err != nil || st.Traces[0].Attempts != 3 {
		t.Fatalf("flaky: err=%v attempts=%d", err, st.Traces[0].Attempts)
	}
	if _, err := engine.ExecuteSync(context.Background(), "reject", map[string]interface{}{}); err == nil || rejected.Load() != 1 {
		t.Fatalf("validation error retried %d times", rejected.Load())
	}
	// Without a policy, sync steps run once.
	var once atomic.Int32
	engine.RegisterActivity("Once", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		once.Add(1)
		return nil, errors.New("down")
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "once", Version: "1.0.0", Steps: []models.WorkflowStep{{ID: "o", Activity: "Once"}}})
	_, _ = engine.ExecuteSync(context.Background(), "once", map[string]interface{}{})
	if once.Load() != 1 {
		t.Fatalf("default attempts = %d", once.Load())
	}
}

// R3: async outcomes are recorded; retry_policy applies to async steps too.
func TestAsyncOutcomeRecorded(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	var calls atomic.Int32
	engine.RegisterActivity("Background", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"sent": true}, nil
	})
	engine.RegisterActivity("BackgroundFail", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		calls.Add(1)
		return nil, errors.New("provider unreachable")
	})
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "bg", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "ok", Activity: "Background", Async: true},
		{ID: "fail", Activity: "BackgroundFail", Async: true, RetryPolicy: map[string]interface{}{"max_attempts": 2, "initial_interval_ms": 5}},
	}})
	st, err := engine.ExecuteSync(context.Background(), "bg", map[string]interface{}{})
	if err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "async outcomes", 3*time.Second, func() bool {
		return traceStatus(t, engine, st.JobID, "ok") == "ASYNC_COMPLETED" && traceStatus(t, engine, st.JobID, "fail") == "ASYNC_FAILED"
	})
	if calls.Load() != 2 {
		t.Fatalf("async retry_policy: %d attempts, want 2", calls.Load())
	}
}

// R3: on a real Temporal server, max_attempts: 1 means exactly one attempt,
// the outcome reaches the history, and StopWorker stops without signals.
func TestTemporalHonoursStepRetryPolicy(t *testing.T) {
	addr := os.Getenv("TEMPORAL_ADDRESS")
	if addr == "" {
		addr = "127.0.0.1:7233"
	}
	if c, err := net.DialTimeout("tcp", addr, 500*time.Millisecond); err != nil {
		t.Skipf("Temporal not reachable at %s", addr)
	} else {
		c.Close()
	}
	engine := pkg.NewOctopusLiveEngine("")
	var calls atomic.Int32
	engine.RegisterActivity("SendOnce", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		calls.Add(1)
		return nil, errors.New("provider declined")
	})
	disp := outbound.NewTemporalAsyncActivityDispatcher(addr, "default", "octopus-hardening-test-"+time.Now().Format("150405.000"))
	disp.FallbackInMemory = false
	engine.SetAsyncDispatcher(disp)
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "durable", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "send", Activity: "SendOnce", Async: true, TimeoutSeconds: 5, RetryPolicy: map[string]interface{}{"max_attempts": 1}},
	}})
	if err := engine.StartEmbeddedWorker(); err != nil {
		t.Fatal(err)
	}
	st, err := engine.ExecuteSync(context.Background(), "durable", map[string]interface{}{})
	if err != nil {
		t.Fatal(err)
	}
	waitUntil(t, "temporal outcome", 15*time.Second, func() bool { return traceStatus(t, engine, st.JobID, "send") == "ASYNC_FAILED" })
	time.Sleep(1500 * time.Millisecond) // a retry would have happened by now
	if calls.Load() != 1 {
		t.Fatalf("activity ran %d times with max_attempts: 1", calls.Load())
	}
	done := make(chan struct{})
	go func() { engine.StopEmbeddedWorker(); close(done) }()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatal("StopWorker hung")
	}
}

// R5: routes and job lookup use the blueprint's version consistently.
func TestVersionPrefixConsistent(t *testing.T) {
	for _, v := range []string{"2.1.0", "v2.1.0"} {
		engine := pkg.NewOctopusLiveEngine("")
		engine.SetOptions(models.EngineOptions{Tooling: models.ToolingOpen})
		engine.RegisterActivity("Noop", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
			return map[string]interface{}{"ok": true}, nil
		})
		_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "ver", Version: v, Steps: []models.WorkflowStep{{ID: "n", Activity: "Noop"}}})
		r := gin.New()
		engine.MountRoutes(r)
		w := do(r, "POST", "/api/v2.1.0/ver/execute", `{}`, "", nil)
		if w.Code != http.StatusOK {
			t.Fatalf("version %q: execute route %d", v, w.Code)
		}
		jobID := w.Header().Get("X-Job-ID")
		w = do(r, "GET", "/api/visualizer/resolve-job?job_id="+jobID, "", "", nil)
		if !strings.Contains(w.Body.String(), `"prefix":"/api/v2.1.0/ver"`) {
			t.Fatalf("version %q: resolve-job %s", v, w.Body.String())
		}
	}
}

// R6: middlewares mount in a stable order (blueprint registration, then
// declaration order), not map iteration order.
func TestMiddlewareOrderDeterministic(t *testing.T) {
	var first string
	for run := 0; run < 15; run++ {
		var mu sync.Mutex
		var order []string
		engine := pkg.NewOctopusLiveEngine("")
		for _, name := range []string{"A", "B", "C", "D"} {
			n := name
			engine.RegisterCustomMiddleware("MW"+n, func(cfg models.MiddlewareConfig, ex ports.IActivityExecutor, d ...ports.IAsyncActivityDispatcher) gin.HandlerFunc {
				return func(c *gin.Context) { mu.Lock(); order = append(order, n); mu.Unlock(); c.Next() }
			})
		}
		for i, pair := range [][]string{{"MWA", "MWB"}, {"MWC"}, {"MWD"}} {
			bp := &models.WorkflowBlueprint{WorkflowName: "wf" + string(rune('a'+i)), Version: "1.0.0"}
			for _, m := range pair {
				bp.Middlewares = append(bp.Middlewares, models.MiddlewareConfig{Name: m})
			}
			_ = engine.RegisterBlueprint(bp)
		}
		r := gin.New()
		engine.MountRoutes(r)
		do(r, "GET", "/health", "", "", nil)
		got := strings.Join(order, "")
		if run == 0 {
			first = got
		}
		if got != first || got != "ABCD" {
			t.Fatalf("run %d: order %q (first %q)", run, got, first)
		}
	}
}

// R7: a failing loop item fails the workflow (and runs compensations), unless
// continue_on_error is set.
func TestLoopErrorsSurface(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	var undone atomic.Bool
	engine.RegisterActivity("Reserve", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"items": []string{"a", "bad", "c"}}, nil
	})
	engine.RegisterActivity("Release", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		undone.Store(true)
		return nil, nil
	})
	engine.RegisterActivity("Ship", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		if in["item"] == "bad" {
			return nil, errors.New("cannot ship bad")
		}
		return map[string]interface{}{"shipped": in["item"]}, nil
	})
	loop := map[string]interface{}{"items_field": "items", "item_variable": "item"}
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "loop", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "reserve", Activity: "Reserve", Compensation: "Release"},
		{ID: "ship", Activity: "Ship", Loop: loop},
	}})
	st, err := engine.ExecuteSync(context.Background(), "loop", map[string]interface{}{})
	if err == nil || st.Status != models.StatusRolledBack || !undone.Load() {
		t.Fatalf("loop failure: err=%v status=%s compensated=%v", err, st.Status, undone.Load())
	}
	tolerant := map[string]interface{}{"items_field": "items", "item_variable": "item", "continue_on_error": true, "max_iterations": 10.0}
	_ = engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "loop2", Version: "1.0.0", Steps: []models.WorkflowStep{
		{ID: "reserve", Activity: "Reserve"}, {ID: "ship", Activity: "Ship", Loop: tolerant},
	}})
	if st, err := engine.ExecuteSync(context.Background(), "loop2", map[string]interface{}{}); err != nil || st.Status != models.StatusCompleted {
		t.Fatalf("continue_on_error: err=%v status=%s", err, st.Status)
	}
}

// R10: unreadable or invalid blueprint files are errors, not silently skipped.
func TestInvalidBlueprintFilesFail(t *testing.T) {
	dir := t.TempDir()
	_ = os.WriteFile(filepath.Join(dir, "good.yaml"), []byte("workflow_name: good\nversion: \"1.0.0\"\nsteps: []\n"), 0o644)
	_ = os.WriteFile(filepath.Join(dir, "typo.yaml"), []byte("workflow_name: typo\nsteps:\n  - id: a\n   activity: X\n"), 0o644)
	engine := pkg.NewOctopusLiveEngine("")
	if err := engine.LoadBlueprintsFromDirectory(dir); err == nil || !strings.Contains(err.Error(), "typo.yaml") {
		t.Fatalf("invalid YAML not reported: %v", err)
	}
}
