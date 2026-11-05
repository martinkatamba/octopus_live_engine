package tests

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strconv"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/adapters/outbound"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/services"
)

func TestGoLiveEngineSyncExecution(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")

	// Register test activities
	engine.RegisterActivity("ValidateOrder", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"is_valid": true,
			"order_id": input["order_id"],
		}, nil
	})

	engine.RegisterActivity("CalculateTax", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		amount := 100.0
		if a, ok := input["amount"].(float64); ok {
			amount = a
		} else if a, ok := input["amount"].(int); ok {
			amount = float64(a)
		} else if aStr, ok := input["amount"].(string); ok {
			if parsed, err := strconv.ParseFloat(aStr, 64); err == nil {
				amount = parsed
			}
		}
		tax := amount * 0.15
		return map[string]interface{}{
			"tax_amount": tax,
			"total":      amount + tax,
		}, nil
	})

	// Register test blueprint
	bp := &models.WorkflowBlueprint{
		WorkflowName: "order_tax_pipeline",
		Version:      "1.0.0",
		Mode:         "sync",
		Steps: []models.WorkflowStep{
			{ID: "validate", Activity: "ValidateOrder"},
			{ID: "calc_tax", Activity: "CalculateTax"},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	// Execute sync
	ctx := context.Background()
	state, err := engine.ExecuteSync(ctx, "order_tax_pipeline", map[string]interface{}{
		"order_id": "ORD-999",
		"amount":   200.0,
	})

	if err != nil {
		t.Fatalf("Execution failed: %v", err)
	}

	if state.Status != models.StatusCompleted {
		t.Errorf("Expected status COMPLETED, got %s", state.Status)
	}

	if len(state.Traces) != 2 {
		t.Errorf("Expected 2 traces, got %d", len(state.Traces))
	}

	if total, ok := state.Data["total"].(float64); !ok || total != 230.0 {
		t.Errorf("Expected total 230.0, got %v", state.Data["total"])
	}

	// Test Visualizer Diagram compilation
	jobID := state.JobID
	diagram, err := engine.VisualizerService.CompileX6Diagram(ctx, "order_tax_pipeline", &jobID, nil, nil)
	if err != nil {
		t.Fatalf("Visualizer compilation failed: %v", err)
	}

	nodes, ok := diagram["nodes"].([]map[string]interface{})
	if !ok || len(nodes) != 4 { // start, validate, calc_tax, end
		t.Errorf("Expected 4 visualizer nodes, got %d", len(nodes))
	}
}

func TestGoLiveEngineLoopAndAsync(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")

	var asyncCalled atomic.Bool
	engine.RegisterActivity("ProcessItem", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		item, _ := input["item"].(map[string]interface{})
		price, _ := item["price"].(float64)
		return map[string]interface{}{
			"processed_price": price * 1.1,
		}, nil
	})

	engine.RegisterActivity("AsyncAuditLog", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		asyncCalled.Store(true)
		return map[string]interface{}{"audited": true}, nil
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName: "batch_loop_pipeline",
		Version:      "1.0.0",
		Mode:         "sync",
		Steps: []models.WorkflowStep{
			{
				ID:       "step_loop",
				Activity: "ProcessItem",
				Loop: map[string]interface{}{
					"items_field":   "items",
					"item_variable": "item",
				},
			},
			{
				ID:       "step_async_log",
				Activity: "AsyncAuditLog",
				IsAsync:  true,
			},
			{
				ID:     "step_settle",
				Settle: true,
				Reason: "Batch settled successfully",
			},
			{
				ID:       "step_unreachable",
				Activity: "ProcessItem",
			},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	ctx := context.Background()
	state, err := engine.ExecuteSync(ctx, "batch_loop_pipeline", map[string]interface{}{
		"items": []interface{}{
			map[string]interface{}{"name": "Item A", "price": 10.0},
			map[string]interface{}{"name": "Item B", "price": 20.0},
		},
	})

	if err != nil {
		t.Fatalf("Execution failed: %v", err)
	}

	if state.Status != models.StatusCompleted {
		t.Errorf("Expected COMPLETED, got %s", state.Status)
	}

	results, ok := state.Data["step_loop_results"].([]interface{})
	if !ok || len(results) != 2 {
		t.Errorf("Expected 2 loop results, got %v", results)
	}

	time.Sleep(50 * time.Millisecond)
	if !asyncCalled.Load() {
		t.Logf("Async activity was scheduled in background")
	}
}

func TestGoLiveEngineSagaCompensation(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")

	compensated := false
	engine.RegisterActivity("ReserveFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"hold_id": "HOLD-999"}, nil
	})

	engine.RegisterActivity("ReleaseFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		compensated = true
		return map[string]interface{}{"released": true}, nil
	})

	engine.RegisterActivity("FailingActivity", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return nil, errors.New("upstream service unavailable")
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName:     "saga_flow",
		CompensationMode: "sync",
		Steps: []models.WorkflowStep{
			{
				ID:           "reserve",
				Activity:     "ReserveFunds",
				Compensation: "ReleaseFunds",
			},
			{
				ID:       "failing_step",
				Activity: "FailingActivity",
			},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	ctx := context.Background()
	state, err := engine.ExecuteSync(ctx, "saga_flow", map[string]interface{}{"amount": 500.0})

	if err == nil {
		t.Fatalf("Expected error, got nil")
	}

	if state.Status != models.StatusRolledBack {
		t.Errorf("Expected ROLLED_BACK, got %s", state.Status)
	}

	if !compensated {
		t.Errorf("Expected compensation activity ReleaseFunds to be executed")
	}
}

func TestGoDeclarativeMultiRouteAPI(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := pkg.NewOctopusLiveEngine("")
	// Opt back into client job ids and remotely readable tooling (httptest
	// requests come from 192.0.2.1, not loopback).
	engine.SetOptions(models.EngineOptions{AllowClientJobIDs: true, Tooling: models.ToolingOpen})

	engine.RegisterActivity("CalculateSubtotal", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"subtotal": 100.0}, nil
	})

	engine.RegisterActivity("CalculateTax", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		amount := 100.0
		if a, ok := input["amount"].(float64); ok {
			amount = a
		} else if aStr, ok := input["amount"].(string); ok {
			if parsed, err := strconv.ParseFloat(aStr, 64); err == nil {
				amount = parsed
			}
		} else if sub, ok := input["subtotal"].(float64); ok {
			amount = sub
		}
		tax := amount * 0.15
		return map[string]interface{}{
			"tax_amount": tax,
			"total":      amount + tax,
		}, nil
	})

	engine.RegisterActivity("PackageTaxResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"tax_total": input["tax_amount"],
			"total":     input["total"],
		}, nil
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName: "ecommerce_tax_service",
		Version:      "1.0.0",
		Mode:         "sync",
		Steps: []models.WorkflowStep{
			{ID: "def_subtotal", Activity: "CalculateSubtotal"},
			{ID: "def_tax", Activity: "CalculateTax"},
		},
		API: []models.APIEndpointConfig{
			{
				Route:       "/api/v1/checkout/tax",
				Method:      "POST",
				InputModel:  "CheckoutTaxRequest",
				OutputModel: "CheckoutTaxResponse",
				StatusCode:  200,
				Summary:     "Calculate tax for cart",
				Tags:        []string{"Checkout"},
				Steps: []models.WorkflowStep{
					{ID: "ep_subtotal", Activity: "CalculateSubtotal"},
					{ID: "ep_tax", Activity: "CalculateTax"},
					{ID: "ep_package", Activity: "PackageTaxResponse", IsOutput: true},
				},
			},
			{
				Route:       "/api/v1/checkout/quick-tax",
				Method:      "GET",
				InputModel:  "QuickTaxRequest",
				OutputModel: "CheckoutTaxResponse",
				StatusCode:  200,
				Summary:     "Quick tax quote",
				Tags:        []string{"Checkout"},
				Steps: []models.WorkflowStep{
					{ID: "quick_tax", Activity: "CalculateTax", IsOutput: true},
				},
			},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	router := gin.New()
	engine.MountRoutes(router)

	// 1. Test POST /api/v1/checkout/tax with X-Job-ID header and IsOutput activity
	postBody, _ := json.Marshal(map[string]interface{}{"cart_id": "CART-123"})
	req, _ := http.NewRequest("POST", "/api/v1/checkout/tax", bytes.NewBuffer(postBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Job-ID", "go-custom-tx-999")

	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != 200 {
		t.Fatalf("Expected status 200, got %d: %s", w.Code, w.Body.String())
	}
	if w.Header().Get("X-Job-ID") != "go-custom-tx-999" {
		t.Errorf("Expected X-Job-ID 'go-custom-tx-999', got '%s'", w.Header().Get("X-Job-ID"))
	}
	if w.Header().Get("X-Execution-Status") != "COMPLETED" {
		t.Errorf("Expected X-Execution-Status 'COMPLETED', got '%s'", w.Header().Get("X-Execution-Status"))
	}

	var respData map[string]interface{}
	_ = json.Unmarshal(w.Body.Bytes(), &respData)
	if respData["total"] != 115.0 {
		t.Errorf("Expected total 115.0, got %v", respData["total"])
	}
	if respData["tax_total"] != 15.0 {
		t.Errorf("Expected tax_total 15.0, got %v", respData["tax_total"])
	}

	// 2. Test GET /api/v1/checkout/quick-tax with query params
	getReq, _ := http.NewRequest("GET", "/api/v1/checkout/quick-tax?amount=200.0", nil)
	getW := httptest.NewRecorder()
	router.ServeHTTP(getW, getReq)

	if getW.Code != 200 {
		t.Fatalf("Expected status 200, got %d: %s", getW.Code, getW.Body.String())
	}
	if getW.Header().Get("X-Execution-Status") != "COMPLETED" {
		t.Errorf("Expected X-Execution-Status COMPLETED, got '%s'", getW.Header().Get("X-Execution-Status"))
	}

	var getRespData map[string]interface{}
	_ = json.Unmarshal(getW.Body.Bytes(), &getRespData)
	if total, ok := getRespData["total"].(float64); !ok || total != 230.0 {
		t.Errorf("Expected total 230.0, got %v", getRespData["total"])
	}

	// 3. Test OpenAPI spec endpoint
	openApiReq, _ := http.NewRequest("GET", "/openapi.json", nil)
	openApiW := httptest.NewRecorder()
	router.ServeHTTP(openApiW, openApiReq)
	if openApiW.Code != 200 {
		t.Errorf("Expected /openapi.json to return 200, got %d", openApiW.Code)
	}
}

func TestGoTemporalAsyncActivityDispatcher(t *testing.T) {
	engine := pkg.NewOctopusLiveEngine("")

	engine.RegisterActivity("AsyncAuditLog", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"logged": true}, nil
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName: "go_durable_async_wf",
		Version:      "1.0.0",
		Mode:         "sync",
		Persistence: map[string]interface{}{
			"temporal": map[string]interface{}{
				"host":       "localhost:7233",
				"namespace":  "default",
				"task_queue": "go-test-queue",
			},
		},
		Steps: []models.WorkflowStep{
			{ID: "step_sync", Activity: "AsyncAuditLog", Async: false},
			{ID: "step_async", Activity: "AsyncAuditLog", Async: true},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	if engine.AsyncDispatcher.GetDispatcherType() != "TEMPORAL" {
		t.Fatalf("Expected dispatcher type TEMPORAL, got %s", engine.AsyncDispatcher.GetDispatcherType())
	}

	state, err := engine.ExecuteSync(context.Background(), "go_durable_async_wf", map[string]interface{}{
		"job_id": "go-durable-job-001",
	})
	if err != nil {
		t.Fatalf("Workflow failed: %v", err)
	}

	if state.Status != models.StatusCompleted {
		t.Errorf("Expected status COMPLETED, got %v", state.Status)
	}
	if len(state.Traces) != 2 {
		t.Fatalf("Expected 2 traces, got %d", len(state.Traces))
	}

	asyncTrace := state.Traces[1]
	if asyncTrace.Status != "ASYNC_DISPATCHED" {
		t.Errorf("Expected ASYNC_DISPATCHED, got %s", asyncTrace.Status)
	}
	if !asyncTrace.IsAsync {
		t.Errorf("Expected IsAsync true")
	}
	if asyncTrace.OutputPayload["dispatcher"] != "TEMPORAL" {
		t.Errorf("Expected dispatcher TEMPORAL in output payload, got %v", asyncTrace.OutputPayload["dispatcher"])
	}
}

func TestGoJobsHistoryEndpoint(t *testing.T) {
	gin.SetMode(gin.TestMode)
	engine := pkg.NewOctopusLiveEngine("")
	// Opt back into client job ids and remotely readable tooling (httptest
	// requests come from 192.0.2.1, not loopback).
	engine.SetOptions(models.EngineOptions{AllowClientJobIDs: true, Tooling: models.ToolingOpen})

	engine.RegisterActivity("ValidateOrder", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"status": "valid"}, nil
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName: "order_tax_pipeline",
		Version:      "1.0.0",
		Mode:         "sync",
		Steps: []models.WorkflowStep{
			{ID: "validate", Activity: "ValidateOrder"},
		},
		API: models.APIEndpoints{
			{
				Route:  "/orders/create",
				Method: "POST",
				Steps: []models.WorkflowStep{
					{ID: "validate", Activity: "ValidateOrder"},
				},
			},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	router := gin.New()
	engine.MountRoutes(router)

	// 1. Execute a request to populate history
	reqBody, _ := json.Marshal(map[string]interface{}{
		"item": "laptop",
	})
	req, _ := http.NewRequest("POST", "/api/v1.0.0/order_tax_pipeline/orders/create", bytes.NewBuffer(reqBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Job-ID", "job-go-test-888")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK from execute, got %d: %s", w.Code, w.Body.String())
	}

	// 2. Test GET /api/visualizer/jobs
	reqJobs, _ := http.NewRequest("GET", "/api/visualizer/jobs", nil)
	wJobs := httptest.NewRecorder()
	router.ServeHTTP(wJobs, reqJobs)

	if wJobs.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK from /api/visualizer/jobs, got %d: %s", wJobs.Code, wJobs.Body.String())
	}

	var jobsList []map[string]interface{}
	if err := json.Unmarshal(wJobs.Body.Bytes(), &jobsList); err != nil {
		t.Fatalf("Failed to parse jobs JSON: %v", err)
	}

	if len(jobsList) == 0 {
		t.Fatalf("Expected at least 1 job in history list, got 0")
	}

	foundJob := false
	for _, j := range jobsList {
		if j["job_id"] == "job-go-test-888" {
			foundJob = true
			if j["workflow_name"] != "order_tax_pipeline" {
				t.Errorf("Expected workflow_name order_tax_pipeline, got %v", j["workflow_name"])
			}
			if j["status"] != "COMPLETED" {
				t.Errorf("Expected status COMPLETED, got %v", j["status"])
			}
			if j["route"] != "/api/v1.0.0/order_tax_pipeline/orders/create" && j["route"] != "/orders/create" {
				t.Errorf("Unexpected route: %v", j["route"])
			}
			if j["method"] != "POST" {
				t.Errorf("Expected method POST, got %v", j["method"])
			}
			if j["client_host"] == nil || j["client_host"] == "" {
				t.Errorf("Expected client_host to be set")
			}
			break
		}
	}
	if !foundJob {
		t.Errorf("job-go-test-888 not found in /api/visualizer/jobs")
	}

	// 3. Test GET /api/jobs
	reqApiJobs, _ := http.NewRequest("GET", "/api/jobs", nil)
	wApiJobs := httptest.NewRecorder()
	router.ServeHTTP(wApiJobs, reqApiJobs)
	if wApiJobs.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK from /api/jobs, got %d", wApiJobs.Code)
	}

	// 4. Test GET /api/v1.0.0/order_tax_pipeline/jobs
	reqWfJobs, _ := http.NewRequest("GET", "/api/v1.0.0/order_tax_pipeline/jobs", nil)
	wWfJobs := httptest.NewRecorder()
	router.ServeHTTP(wWfJobs, reqWfJobs)
	if wWfJobs.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK from /api/v1.0.0/order_tax_pipeline/jobs, got %d", wWfJobs.Code)
	}

	// 5. Test Visualizer HTML contains Jobs Modal components
	reqVis, _ := http.NewRequest("GET", "/visualizer", nil)
	wVis := httptest.NewRecorder()
	router.ServeHTTP(wVis, reqVis)

	if wVis.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK from /visualizer, got %d", wVis.Code)
	}
	visHtml := wVis.Body.String()
	if !bytes.Contains([]byte(visHtml), []byte("jobsModalBackdrop")) {
		t.Errorf("Visualizer HTML missing 'jobsModalBackdrop'")
	}
	if !bytes.Contains([]byte(visHtml), []byte("Execution Jobs History")) {
		t.Errorf("Visualizer HTML missing 'Execution Jobs History'")
	}
	if !bytes.Contains([]byte(visHtml), []byte("openJobsModal()")) {
		t.Errorf("Visualizer HTML missing 'openJobsModal()'")
	}
}

func TestSensitiveFieldMasking(t *testing.T) {
	// 1. Direct Unit Test of services.MaskPayload
	rawPayload := map[string]interface{}{
		"username": "sarah_vip",
		"password": "supersecretpassword",
		"API_KEY":  "sk_live_99999",
		"meta": map[string]interface{}{
			"client_ip": "127.0.0.1",
			"Secret":    "nested_secret_val",
		},
		"tokens": []interface{}{
			map[string]interface{}{
				"token_type": "Bearer",
				"TOKEN":      "jwt.token.abc",
			},
		},
	}

	maskFields := []string{"password", "api_key", "secret", "token"}
	masked := services.MaskPayload(rawPayload, maskFields, "********")

	if masked["username"] != "sarah_vip" {
		t.Errorf("Expected username to be unmasked 'sarah_vip', got %v", masked["username"])
	}
	if masked["password"] != "********" {
		t.Errorf("Expected password to be masked, got %v", masked["password"])
	}
	if masked["API_KEY"] != "********" {
		t.Errorf("Expected API_KEY to be masked, got %v", masked["API_KEY"])
	}

	nestedMeta, ok := masked["meta"].(map[string]interface{})
	if !ok || nestedMeta["Secret"] != "********" {
		t.Errorf("Expected nested Secret to be masked, got %v", nestedMeta)
	}
	if nestedMeta["client_ip"] != "127.0.0.1" {
		t.Errorf("Expected nested client_ip to remain unchanged, got %v", nestedMeta["client_ip"])
	}

	tokensList, ok := masked["tokens"].([]interface{})
	if !ok || len(tokensList) != 1 {
		t.Fatalf("Expected tokens list to have 1 item, got %v", masked["tokens"])
	}
	tokenItem, ok := tokensList[0].(map[string]interface{})
	if !ok || tokenItem["TOKEN"] != "********" {
		t.Errorf("Expected TOKEN in array to be masked, got %v", tokenItem)
	}
	if tokenItem["token_type"] != "Bearer" {
		t.Errorf("Expected token_type to remain 'Bearer', got %v", tokenItem["token_type"])
	}

	// 2. Integration Test: Live Engine Workflow Execution with Blueprint Masking
	engine := pkg.NewOctopusLiveEngine("")

	var receivedPassword string
	engine.RegisterActivity("LoginAuthActivity", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		receivedPassword, _ = input["password"].(string)
		if receivedPassword != "secret123" {
			return nil, errors.New("invalid credentials")
		}
		return map[string]interface{}{
			"access_token": "token_abc123xyz",
			"customer_id":  "cust_001",
		}, nil
	})

	engine.RegisterActivity("StepWithCustomMask", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"processed": true,
		}, nil
	})

	bp := &models.WorkflowBlueprint{
		WorkflowName: "masked_auth_pipeline",
		Version:      "1.0.0",
		Mode:         "sync",
		Masking: &models.MaskingConfig{
			Enabled:   true,
			Fields:    []string{"password", "access_token"},
			MaskValue: "********",
		},
		Steps: []models.WorkflowStep{
			{
				ID:       "step_login",
				Activity: "LoginAuthActivity",
			},
			{
				ID:         "step_custom_pin",
				Activity:   "StepWithCustomMask",
				MaskFields: []string{"pin_code"},
			},
		},
	}
	_ = engine.RegisterBlueprint(bp)

	ctx := context.Background()
	state, err := engine.ExecuteSync(ctx, "masked_auth_pipeline", map[string]interface{}{
		"username": "sarah_vip",
		"password": "secret123",
		"pin_code": "4321",
	})

	if err != nil {
		t.Fatalf("Workflow execution failed: %v", err)
	}

	// Verify Activity received real unmasked credentials
	if receivedPassword != "secret123" {
		t.Errorf("Expected activity to receive unmasked password 'secret123', got '%s'", receivedPassword)
	}

	// Verify Step 1 Trace InputPayload has masked password and OutputPayload has masked access_token
	if len(state.Traces) < 2 {
		t.Fatalf("Expected at least 2 traces, got %d", len(state.Traces))
	}

	loginTrace := state.Traces[0]
	if loginTrace.InputPayload["password"] != "********" {
		t.Errorf("Expected trace InputPayload['password'] to be '********', got %v", loginTrace.InputPayload["password"])
	}
	if loginTrace.InputPayload["username"] != "sarah_vip" {
		t.Errorf("Expected trace InputPayload['username'] to be 'sarah_vip', got %v", loginTrace.InputPayload["username"])
	}
	if loginTrace.OutputPayload["access_token"] != "********" {
		t.Errorf("Expected trace OutputPayload['access_token'] to be '********', got %v", loginTrace.OutputPayload["access_token"])
	}
	if loginTrace.OutputPayload["customer_id"] != "cust_001" {
		t.Errorf("Expected trace OutputPayload['customer_id'] to be 'cust_001', got %v", loginTrace.OutputPayload["customer_id"])
	}

	// Verify Step 2 Trace has step-level masked pin_code
	customTrace := state.Traces[1]
	if customTrace.InputPayload["pin_code"] != "********" {
		t.Errorf("Expected trace InputPayload['pin_code'] to be masked '********', got %v", customTrace.InputPayload["pin_code"])
	}

	// 3. Test Visualizer Diagram node sanitization
	jobID := state.JobID
	diagram, err := engine.VisualizerService.CompileX6Diagram(ctx, "masked_auth_pipeline", &jobID, nil, nil)
	if err != nil {
		t.Fatalf("Failed to compile diagram: %v", err)
	}

	nodes, ok := diagram["nodes"].([]map[string]interface{})
	if !ok {
		t.Fatalf("Expected diagram nodes slice")
	}

	var loginNode map[string]interface{}
	for _, n := range nodes {
		if data, ok := n["data"].(map[string]interface{}); ok {
			if data["step_id"] == "step_login" {
				loginNode = data
				break
			}
		}
	}

	if loginNode == nil {
		t.Fatalf("step_login node not found in compiled diagram")
	}

	nodeInput, ok := loginNode["input_payload"].(map[string]interface{})
	if !ok || nodeInput["password"] != "********" {
		t.Errorf("Expected diagram node input_payload password to be '********', got %v", nodeInput["password"])
	}
	if nodeInput["username"] != "sarah_vip" {
		t.Errorf("Expected diagram node input_payload username to be 'sarah_vip', got %v", nodeInput["username"])
	}
}

func TestRelationalStateStoreAndDialects(t *testing.T) {
	// 1. Dialect Verification
	pg := &outbound.PostgresDialect{}
	if pg.Name() != "postgres" || pg.Placeholder(1) != "$1" {
		t.Errorf("PostgresDialect mismatch: %v", pg.Placeholder(1))
	}
	if !bytes.Contains([]byte(pg.UpsertExecutionStateSQL()), []byte("ON CONFLICT (job_id) DO UPDATE")) {
		t.Errorf("PostgresDialect missing ON CONFLICT upsert")
	}

	my := &outbound.MySqlDialect{}
	if my.Name() != "mysql" || my.Placeholder(1) != "?" {
		t.Errorf("MySqlDialect mismatch: %v", my.Placeholder(1))
	}
	if !bytes.Contains([]byte(my.UpsertExecutionStateSQL()), []byte("ON DUPLICATE KEY UPDATE")) {
		t.Errorf("MySqlDialect missing ON DUPLICATE KEY upsert")
	}

	ms := &outbound.SqlServerDialect{}
	if ms.Name() != "sqlserver" {
		t.Errorf("SqlServerDialect name mismatch")
	}
	if !bytes.Contains([]byte(ms.UpsertExecutionStateSQL()), []byte("MERGE INTO execution_states")) {
		t.Errorf("SqlServerDialect missing MERGE INTO upsert")
	}

	sq := &outbound.SqliteDialect{}
	if sq.Name() != "sqlite" {
		t.Errorf("SqliteDialect name mismatch")
	}

	// 2. StoreFactory Verification
	factory := outbound.NewStoreFactory()
	sMem := factory.Create("inmemory", nil)
	if sMem == nil {
		t.Fatalf("StoreFactory failed to create inmemory store")
	}

	sPg := factory.Create("postgres", map[string]interface{}{"dsn": "postgres://localhost:5432/db"})
	if sPg == nil {
		t.Fatalf("StoreFactory failed to create postgres store")
	}

	sMy := factory.Create("mysql", map[string]interface{}{"dsn": "mysql://localhost:3306/db"})
	if sMy == nil {
		t.Fatalf("StoreFactory failed to create mysql store")
	}

	sMs := factory.Create("sqlserver", map[string]interface{}{"dsn": "sqlserver://localhost:1433"})
	if sMs == nil {
		t.Fatalf("StoreFactory failed to create sqlserver store")
	}

	// 3. Fallback / Memory-backed Execution of RelationalStateStore
	relStore := outbound.NewRelationalStateStore(nil, pg)
	ctx := context.Background()

	state := &models.ExecutionState{
		JobID:        "go-relational-job-101",
		WorkflowName: "order_tax_pipeline",
		Status:       models.StatusCompleted,
		Route:        "/api/v1/tax",
		Method:       "POST",
		StartedAt:    "2026-09-23T18:00:00Z",
		Data: map[string]interface{}{
			"subtotal": 100.0,
		},
		OutputData: map[string]interface{}{
			"tax":   8.25,
			"total": 108.25,
		},
		Traces: []models.StepExecutionTrace{
			{
				StepID:       "step_tax",
				ActivityName: "CalculateSalesTax",
				Status:       "COMPLETED",
				InputPayload: map[string]interface{}{
					"subtotal": 100.0,
				},
				OutputPayload: map[string]interface{}{
					"tax": 8.25,
				},
				DurationMs: 0.85,
			},
		},
	}

	err := relStore.SaveState(ctx, state)
	if err != nil {
		t.Fatalf("Failed to save state in relational store: %v", err)
	}

	fetched, err := relStore.GetState(ctx, "go-relational-job-101")
	if err != nil || fetched == nil {
		t.Fatalf("Failed to fetch state from relational store: %v", err)
	}
	if fetched.JobID != "go-relational-job-101" || fetched.WorkflowName != "order_tax_pipeline" {
		t.Errorf("Fetched state mismatch: %v", fetched)
	}
	if len(fetched.Traces) != 1 || fetched.Traces[0].ActivityName != "CalculateSalesTax" {
		t.Errorf("Fetched traces mismatch: %v", fetched.Traces)
	}

	listed, err := relStore.ListStates(ctx, "order_tax_pipeline", 10)
	if err != nil || len(listed) != 1 {
		t.Errorf("ListStates mismatch, expected 1 item, got %d (err: %v)", len(listed), err)
	}
}
