package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

func TestDurableTemporalSaga_SuccessPath(t *testing.T) {
	engine := octopus_live.NewLiveEngine()
	engine.RegisterModel("SettlementRequest", SettlementRequest{})
	engine.RegisterModel("SettlementResponse", SettlementResponse{})

	engine.RegisterActivity("ValidateSettlementRequest", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"validated": true}, nil
	})
	engine.RegisterActivity("ReserveEscrowFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"escrow_hold_id": "ESC-1", "escrow_status": "HELD"}, nil
	})
	engine.RegisterActivity("ReleaseEscrowHold", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"escrow_status": "RELEASED"}, nil
	})
	engine.RegisterActivity("AllocateInventoryStock", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"inventory_alloc_id": "ALLOC-1", "inventory_status": "RESERVED"}, nil
	})
	engine.RegisterActivity("RestockInventory", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"inventory_status": "RESTOCKED"}, nil
	})
	engine.RegisterActivity("ExecuteBankClearing", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"clearing_ref": "WIRE-1", "clearing_status": "SETTLED"}, nil
	})
	engine.RegisterActivity("FinalizeSettlementResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"status": "SETTLED", "settlement_id": "SETTLE-1"}, nil
	})

	if err := engine.LoadBlueprintFile("durable_saga_workflow.yaml"); err != nil {
		t.Fatalf("failed to load blueprint: %v", err)
	}

	st, err := engine.ExecuteSync(context.Background(), "durable_temporal_saga", map[string]interface{}{
		"account_id":       "ACC-CLIENT-001",
		"amount":           1500.0,
		"currency":         "USD",
		"sku":              "SKU-TITAN-X",
		"quantity":         1,
		"simulate_failure": false,
	})
	if err != nil {
		t.Fatalf("expected success, got err: %v", err)
	}
	if st.Status != "COMPLETED" {
		t.Fatalf("expected status COMPLETED, got %s", st.Status)
	}
}

func TestDurableTemporalSaga_RollbackAndTemporalCompensation(t *testing.T) {
	engine := octopus_live.NewLiveEngine()
	engine.RegisterModel("SettlementRequest", SettlementRequest{})
	engine.RegisterModel("SettlementResponse", SettlementResponse{})

	compensatedEscrow := false
	compensatedInventory := false

	engine.RegisterActivity("ValidateSettlementRequest", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"validated": true}, nil
	})
	engine.RegisterActivity("ReserveEscrowFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"escrow_hold_id": "ESC-999", "escrow_status": "HELD"}, nil
	})
	engine.RegisterActivity("ReleaseEscrowHold", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		compensatedEscrow = true
		return map[string]interface{}{"escrow_status": "RELEASED_TO_CLIENT", "compensated": true}, nil
	})
	engine.RegisterActivity("AllocateInventoryStock", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"inventory_alloc_id": "ALLOC-999", "inventory_status": "RESERVED"}, nil
	})
	engine.RegisterActivity("RestockInventory", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		compensatedInventory = true
		return map[string]interface{}{"inventory_status": "RESTOCKED", "compensated": true}, nil
	})
	engine.RegisterActivity("ExecuteBankClearing", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return nil, context.DeadlineExceeded
	})
	engine.RegisterActivity("FinalizeSettlementResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"status": "SETTLED"}, nil
	})

	if err := engine.LoadBlueprintFile("durable_saga_workflow.yaml"); err != nil {
		t.Fatalf("failed to load blueprint: %v", err)
	}

	_ = engine.StartEmbeddedWorker()
	defer engine.StopEmbeddedWorker()

	st, err := engine.ExecuteSync(context.Background(), "durable_temporal_saga", map[string]interface{}{
		"account_id":       "ACC-CLIENT-002",
		"amount":           2500.0,
		"currency":         "USD",
		"sku":              "SKU-TITAN-X",
		"quantity":         2,
		"simulate_failure": true,
	})

	if err == nil {
		t.Fatalf("expected error from ExecuteBankClearing, got nil")
	}
	if st.Status != "ROLLED_BACK" {
		t.Fatalf("expected status ROLLED_BACK, got %s", st.Status)
	}

	// Give Temporal worker up to 5 seconds to process dispatched compensation workflows
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if compensatedEscrow && compensatedInventory {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}

	if !compensatedEscrow {
		t.Errorf("expected ReleaseEscrowHold compensation to be executed by Temporal worker")
	}
	if !compensatedInventory {
		t.Errorf("expected RestockInventory compensation to be executed by Temporal worker")
	}
}

func TestDurableTemporalSaga_HTTPRoute(t *testing.T) {
	engine := octopus_live.NewLiveEngine()
	engine.RegisterModel("SettlementRequest", SettlementRequest{})
	engine.RegisterModel("SettlementResponse", SettlementResponse{})

	engine.RegisterActivity("ValidateSettlementRequest", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"validated": true}, nil
	})
	engine.RegisterActivity("ReserveEscrowFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"escrow_hold_id": "ESC-HTTP-1"}, nil
	})
	engine.RegisterActivity("ReleaseEscrowHold", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"status": "RELEASED"}, nil
	})
	engine.RegisterActivity("AllocateInventoryStock", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"inventory_alloc_id": "ALLOC-HTTP-1"}, nil
	})
	engine.RegisterActivity("RestockInventory", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"status": "RESTOCKED"}, nil
	})
	engine.RegisterActivity("ExecuteBankClearing", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"clearing_ref": "CLR-HTTP-1"}, nil
	})
	engine.RegisterActivity("FinalizeSettlementResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{"settlement_id": "SETTLE-HTTP-1", "status": "SETTLED"}, nil
	})

	if err := engine.LoadBlueprintFile("durable_saga_workflow.yaml"); err != nil {
		t.Fatalf("failed to load blueprint: %v", err)
	}

	r := engine.GetGinEngine()
	reqBody := `{"account_id":"ACC-001","amount":1500,"currency":"USD","sku":"SKU-TITAN","quantity":1,"simulate_failure":false}`
	req, _ := http.NewRequest("POST", "/api/v1/settlement/process", strings.NewReader(reqBody))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()

	r.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("expected HTTP 200, got %d (body: %s)", w.Code, w.Body.String())
	}
}
