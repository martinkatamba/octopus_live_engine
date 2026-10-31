package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

// ============================================================================
// DURABLE TEMPORAL SAGA COMPENSATION SHOWCASE (Go):
// Demonstrates end-to-end Saga compensation with 100% durable worker guarantees.
// When an activity fails downstream, OCTOPUS Live Engine automatically delegates
// the reverse compensation steps to Temporal Server as durable workflows.
// If a process crashes, Temporal guarantees retries and durability until
// compensation completes.
// ============================================================================

// 1. Domain Models
type SettlementRequest struct {
	AccountID       string  `json:"account_id"`
	Amount          float64 `json:"amount"`
	Currency        string  `json:"currency"`
	SKU             string  `json:"sku"`
	Quantity        int     `json:"quantity"`
	SimulateFailure bool    `json:"simulate_failure"`
	JobID           string  `json:"job_id,omitempty"`
}

type SettlementResponse struct {
	SettlementID       string  `json:"settlement_id"`
	Status             string  `json:"status"`
	EscrowHoldID       string  `json:"escrow_hold_id"`
	InventoryAllocID   string  `json:"inventory_alloc_id"`
	ClearingRef        string  `json:"clearing_ref"`
	Amount             float64 `json:"amount"`
	Currency           string  `json:"currency"`
	TemporalGuaranteed string  `json:"temporal_guaranteed"`
}

func main() {
	engine := octopus_live.NewLiveEngine()
	engine.SetOptions(models.EngineOptions{
		AllowClientJobIDs: true,
	})

	// 1. Register Models for Schema & Validation
	engine.RegisterModel("SettlementRequest", SettlementRequest{})
	engine.RegisterModel("SettlementResponse", SettlementResponse{})

	// 2. Register Activities & Compensations
	// --------------------------------------------------------------------------
	// Activity 1: Validate Settlement Request
	// --------------------------------------------------------------------------
	engine.RegisterActivity("ValidateSettlementRequest", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		accountID, _ := input["account_id"].(string)
		amount, _ := input["amount"].(float64)
		if accountID == "" {
			return nil, fmt.Errorf("validation error: account_id is required")
		}
		if amount <= 0 {
			return nil, fmt.Errorf("validation error: amount must be greater than 0")
		}
		log.Printf("🔍 [COMPLIANCE] Account %s verified for settlement of $%.2f", accountID, amount)
		return map[string]interface{}{
			"validated":   true,
			"verified_at": time.Now().UTC().Format(time.RFC3339),
		}, nil
	})

	// --------------------------------------------------------------------------
	// Activity 2: Reserve Escrow Funds
	// --------------------------------------------------------------------------
	engine.RegisterActivity("ReserveEscrowFunds", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		accountID, _ := input["account_id"].(string)
		amount, _ := input["amount"].(float64)
		holdID := fmt.Sprintf("ESC-HOLD-%d", time.Now().UnixNano()%1000000)
		log.Printf("🔒 [ESCROW LEDGER] Funds reserved: $%.2f on hold %s for account %s", amount, holdID, accountID)
		return map[string]interface{}{
			"escrow_hold_id": holdID,
			"escrow_amount":  amount,
			"escrow_status":  "HELD",
		}, nil
	})

	// --------------------------------------------------------------------------
	// Compensation 2: Release Escrow Hold (DURABLE ON TEMPORAL)
	// --------------------------------------------------------------------------
	engine.RegisterActivity("ReleaseEscrowHold", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		holdID, _ := input["escrow_hold_id"].(string)
		accountID, _ := input["account_id"].(string)
		amount, _ := input["amount"].(float64)
		wfID, _ := input["temporal_workflow_id"].(string)

		log.Printf("🛡️ [TEMPORAL WORKER COMPENSATING] Escrow hold %s ($%.2f) RELEASED back to account %s! (Temporal Workflow: %s)", holdID, amount, accountID, wfID)
		return map[string]interface{}{
			"escrow_status":        "RELEASED_TO_CLIENT",
			"released_hold_id":     holdID,
			"compensated_by":       "TEMPORAL_DURABLE_WORKER",
			"temporal_workflow_id": wfID,
			"compensated_at":       time.Now().UTC().Format(time.RFC3339),
		}, nil
	})

	// --------------------------------------------------------------------------
	// Activity 3: Allocate Inventory Stock
	// --------------------------------------------------------------------------
	engine.RegisterActivity("AllocateInventoryStock", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		sku, _ := input["sku"].(string)
		if sku == "" {
			sku = "SKU-TITAN-X"
		}
		qty, _ := input["quantity"].(float64)
		if qty <= 0 {
			qty = 1
		}
		allocID := fmt.Sprintf("ALLOC-%d", time.Now().UnixNano()%1000000)
		log.Printf("📦 [INVENTORY LEDGER] Allocated SKU %s (Qty: %.0f) on alloc ID %s", sku, qty, allocID)
		return map[string]interface{}{
			"inventory_alloc_id": allocID,
			"allocated_sku":      sku,
			"allocated_qty":      qty,
			"inventory_status":   "RESERVED",
		}, nil
	})

	// --------------------------------------------------------------------------
	// Compensation 3: Restock Inventory (DURABLE ON TEMPORAL)
	// --------------------------------------------------------------------------
	engine.RegisterActivity("RestockInventory", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		allocID, _ := input["inventory_alloc_id"].(string)
		sku, _ := input["allocated_sku"].(string)
		if sku == "" {
			sku, _ = input["sku"].(string)
		}
		wfID, _ := input["temporal_workflow_id"].(string)

		log.Printf("🛡️ [TEMPORAL WORKER COMPENSATING] Allocation %s for SKU %s RESTOCKED back to warehouse! (Temporal Workflow: %s)", allocID, sku, wfID)
		return map[string]interface{}{
			"inventory_status":     "RESTOCKED",
			"restocked_alloc_id":   allocID,
			"compensated_by":       "TEMPORAL_DURABLE_WORKER",
			"temporal_workflow_id": wfID,
			"compensated_at":       time.Now().UTC().Format(time.RFC3339),
		}, nil
	})

	// --------------------------------------------------------------------------
	// Activity 4: Execute Bank Clearing (Simulates downstream rejection)
	// --------------------------------------------------------------------------
	engine.RegisterActivity("ExecuteBankClearing", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		simFail, _ := input["simulate_failure"].(bool)
		if simFail {
			log.Printf("❌ [CLEARING HOUSE] Wire clearing REJECTED: Counterparty bank liquidity sanctions stop")
			return nil, fmt.Errorf("BankClearingRejected: counterparty bank liquidity violation / sanctions stop")
		}

		clrRef := fmt.Sprintf("WIRE-CLR-%d", time.Now().UnixNano()%1000000)
		log.Printf("✅ [CLEARING HOUSE] Wire clearing SETTLED with reference %s", clrRef)
		return map[string]interface{}{
			"clearing_ref":    clrRef,
			"clearing_status": "SETTLED",
		}, nil
	})

	// --------------------------------------------------------------------------
	// Activity 5: Finalize Settlement Response
	// --------------------------------------------------------------------------
	engine.RegisterActivity("FinalizeSettlementResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		holdID, _ := input["escrow_hold_id"].(string)
		allocID, _ := input["inventory_alloc_id"].(string)
		clrRef, _ := input["clearing_ref"].(string)
		amount, _ := input["amount"].(float64)
		currency, _ := input["currency"].(string)
		if currency == "" {
			currency = "USD"
		}
		settleID := fmt.Sprintf("SETTLE-%d", time.Now().UnixNano()%1000000)

		return map[string]interface{}{
			"settlement_id":       settleID,
			"status":              "SETTLED",
			"escrow_hold_id":      holdID,
			"inventory_alloc_id":  allocID,
			"clearing_ref":        clrRef,
			"amount":              amount,
			"currency":            currency,
			"temporal_guaranteed": "ACTIVE",
		}, nil
	})

	// 3. Load Workflow Blueprint from YAML
	if err := engine.LoadBlueprintFile("durable_saga_workflow.yaml"); err != nil {
		log.Fatalf("Failed to load blueprint: %v", err)
	}

	// 4. Start Embedded Temporal Worker
	log.Printf("🚀 Connecting to Temporal Server at 127.0.0.1:7233 and starting embedded worker...")
	if err := engine.StartEmbeddedWorker(); err != nil {
		log.Printf("⚠️ Could not start Temporal embedded worker: %v", err)
	} else {
		log.Printf("👷 Temporal Embedded Worker successfully polling on 'durable_saga_queue'")
	}
	defer engine.StopEmbeddedWorker()

	// 5. Mount Routes & Start HTTP Server
	port := os.Getenv("PORT")
	if port == "" {
		port = "8095"
	}
	if !strings.HasPrefix(port, ":") {
		port = ":" + port
	}

	r := engine.GetGinEngine()
	// Allow CORS for visualizer
	r.Use(func(c *gin.Context) {
		c.Header("Access-Control-Allow-Origin", "*")
		c.Header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
		c.Header("Access-Control-Allow-Headers", "Content-Type, Authorization, X-Requested-With")
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(204)
			return
		}
		c.Next()
	})

	fmt.Println("\n==================================================================")
	fmt.Printf("🚀 OCTOPUS Live Engine - Durable Temporal Saga API on port %s\n", port)
	fmt.Printf("📖 Swagger Docs:    http://localhost%s/docs\n", port)
	fmt.Printf("🎨 Live Visualizer: http://localhost%s/visualizer?domain=durable_temporal_saga\n", port)
	fmt.Printf("⏱️ Temporal Web UI: http://127.0.0.1:8233/namespaces/default/workflows\n")
	fmt.Println("==================================================================")

	if err := r.Run(port); err != nil {
		log.Fatalf("Server failed to start: %v", err)
	}
}
