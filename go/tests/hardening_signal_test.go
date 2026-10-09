package tests

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
)

type payFlow struct {
	engine              *pkg.OctopusLiveEngine
	router              *gin.Engine
	confirmed, expired  atomic.Int32
	released, delivered atomic.Int32
	seenOrder           atomic.Value
}

// newPayFlow: initiate -> respond -> wait for the provider -> settle.
func newPayFlow(t *testing.T, timeout int, failAfterResume bool) *payFlow {
	t.Helper()
	gin.SetMode(gin.TestMode)
	f := &payFlow{engine: pkg.NewOctopusLiveEngine("")}
	e := f.engine
	e.RegisterActivity("Initiate", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"payment_ref": "ref-1", "order_id": "order-42"}, nil
	})
	e.RegisterActivity("Release", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		f.released.Add(1)
		return nil, nil
	})
	e.RegisterActivity("Respond", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"message": "check your phone", "payment_ref": in["payment_ref"]}, nil
	})
	e.RegisterActivity("Confirm", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		f.confirmed.Add(1)
		f.seenOrder.Store(in["order_id"])
		return map[string]interface{}{"paid": true}, nil
	})
	e.RegisterActivity("Expire", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		f.expired.Add(1)
		return map[string]interface{}{"paid": false}, nil
	})
	e.RegisterActivity("Deliver", func(ctx context.Context, in map[string]interface{}) (map[string]interface{}, error) {
		f.delivered.Add(1)
		if failAfterResume {
			return nil, errors.New("courier unavailable")
		}
		return nil, nil
	})
	if err := e.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "pay", Version: "1.0.0", API: models.APIEndpoints{{
		Route: "/api/v1.0.0/pay/start", Method: "POST", Steps: []models.WorkflowStep{
			{ID: "initiate", Activity: "Initiate", Compensation: "Release"},
			{ID: "respond", Activity: "Respond", IsOutput: true},
			{ID: "wait_for_provider", AwaitSignal: "payment_result", TimeoutSeconds: timeout},
			{ID: "settle", Condition: &models.StepCondition{Variable: "signal.data.status", Operator: "==", Value: "successful"},
				ThenSteps: []models.WorkflowStep{{ID: "confirm", Activity: "Confirm"}, {ID: "deliver", Activity: "Deliver"}},
				ElseSteps: []models.WorkflowStep{{ID: "expire", Activity: "Expire"}}},
		},
	}}}); err != nil {
		t.Fatal(err)
	}
	f.router = gin.New()
	e.MountRoutes(f.router)
	return f
}

func (f *payFlow) start(t *testing.T) string {
	t.Helper()
	w := do(f.router, "POST", "/api/v1.0.0/pay/start", `{}`, "", nil)
	if w.Code != http.StatusOK || w.Header().Get("X-Execution-Status") != "AWAITING_SIGNAL" {
		t.Fatalf("start: %d %s %s", w.Code, w.Header().Get("X-Execution-Status"), w.Body.String())
	}
	var out map[string]interface{}
	_ = json.Unmarshal(w.Body.Bytes(), &out)
	if out["message"] != "check your phone" {
		t.Fatalf("output before pausing: %v", out)
	}
	return w.Header().Get("X-Job-ID")
}

func TestAwaitSignalResumesOnce(t *testing.T) {
	f := newPayFlow(t, 0, false)
	ctx := context.Background()
	jobID := f.start(t)
	if st, _ := f.engine.GetExecutionState(ctx, jobID); st.Status != models.StatusAwaitingSignal {
		t.Fatalf("stored status = %s", st.Status)
	}
	if _, err := f.engine.Signal(ctx, jobID, "something_else", nil, nil); !errors.Is(err, services.ErrSignalMismatch) {
		t.Fatalf("wrong signal: %v", err)
	}
	// Signal data can't overwrite the flow's own values.
	st, err := f.engine.Signal(ctx, jobID, "payment_result", map[string]interface{}{"status": "successful", "order_id": "evil"}, nil)
	if err != nil || st.Status != models.StatusCompleted || f.confirmed.Load() != 1 {
		t.Fatalf("resume: err=%v status=%s confirmed=%d", err, st.Status, f.confirmed.Load())
	}
	if f.seenOrder.Load() != "order-42" {
		t.Fatalf("signal data overrode order_id: %v", f.seenOrder.Load())
	}
	if _, err := f.engine.Signal(ctx, jobID, "payment_result", map[string]interface{}{"status": "successful"}, nil); !errors.Is(err, services.ErrNoSuspension) {
		t.Fatalf("second signal resumed again: %v", err)
	}
	if f.confirmed.Load() != 1 {
		t.Fatalf("confirmed %d times", f.confirmed.Load())
	}
	stored, _ := f.engine.GetExecutionState(ctx, jobID)
	var sawSignal bool
	for _, tr := range stored.Traces {
		sawSignal = sawSignal || tr.Status == "SIGNAL_RECEIVED"
	}
	if stored.Status != models.StatusCompleted || !sawSignal {
		t.Fatalf("history after resume: %s signal=%v", stored.Status, sawSignal)
	}
}

func TestAwaitSignalTimeout(t *testing.T) {
	f := newPayFlow(t, 1, false)
	jobID := f.start(t)
	waitUntil(t, "timeout branch", 4*time.Second, func() bool {
		st, _ := f.engine.GetExecutionState(context.Background(), jobID)
		return st != nil && st.Status == models.StatusCompleted
	})
	if f.expired.Load() != 1 || f.confirmed.Load() != 0 {
		t.Fatalf("timeout: expired=%d confirmed=%d", f.expired.Load(), f.confirmed.Load())
	}
	if _, err := f.engine.Signal(context.Background(), jobID, "payment_result", nil, nil); !errors.Is(err, services.ErrNoSuspension) {
		t.Fatalf("late signal: %v", err)
	}
}

func TestCompensationSurvivesSuspension(t *testing.T) {
	f := newPayFlow(t, 0, true)
	jobID := f.start(t)
	st, err := f.engine.Signal(context.Background(), jobID, "payment_result", map[string]interface{}{"status": "successful"}, nil)
	if err == nil || st.Status != models.StatusRolledBack || f.released.Load() != 1 {
		t.Fatalf("rollback after resume: err=%v status=%s released=%d", err, st.Status, f.released.Load())
	}
}

func TestSignalHTTPRoute(t *testing.T) {
	f := newPayFlow(t, 0, false)
	jobID := f.start(t)
	path := "/api/v1.0.0/pay/signal/" + jobID
	if w := do(f.router, "POST", path, `{"signal":"payment_result","status":"successful"}`, "203.0.113.9:1", nil); w.Code != http.StatusForbidden {
		t.Fatalf("remote signal without auth: %d", w.Code)
	}
	if w := do(f.router, "POST", path, `{"signal":"payment_result","status":"successful"}`, "127.0.0.1:1", nil); w.Code != http.StatusOK || f.confirmed.Load() != 1 {
		t.Fatalf("local signal: %d %s", w.Code, w.Body.String())
	}
	if w := do(f.router, "POST", path, `{}`, "127.0.0.1:1", nil); w.Code != http.StatusNotFound {
		t.Fatalf("signal after resume: %d", w.Code)
	}
}

func TestUnsupportedBlueprintKeysRejected(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")
	if err := engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "dyn", Steps: []models.WorkflowStep{
		{ID: "x", DynamicBranching: map[string]interface{}{"field": "kind"}}}}); err == nil {
		t.Error("dynamic_branching accepted")
	}
	if err := engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "roles", AllowedInitiatorRoles: []string{"admin"}}); err == nil {
		t.Error("allowed_initiator_roles accepted")
	}
	if err := engine.RegisterBlueprint(&models.WorkflowBlueprint{WorkflowName: "asyncwait", Steps: []models.WorkflowStep{
		{ID: "w", AwaitSignal: "x", Async: true}}}); err == nil {
		t.Error("async await_signal accepted")
	}
}
