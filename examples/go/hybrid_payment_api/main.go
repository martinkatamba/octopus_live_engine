package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"
	"time"

	"github.com/martinkatamba/octopus_live_engine/go/pkg/octopus_live"
)

// ==========================================================================
// ZERO-IMPORT DEVELOPER EXPERIENCE (Go):
// Notice: ZERO imports of temporalio, duckdb, redis, or database drivers!
// All connection management, drivers, and background durability replication
// are handled automatically by the OCTOPUS Live Engine.
// ==========================================================================

// 1. Domain Models
type PaymentRequest struct {
	AccountID string  `json:"account_id"`
	Amount    float64 `json:"amount"`
	Currency  string  `json:"currency"`
	CardToken string  `json:"card_token"`
	JobID     string  `json:"job_id,omitempty"`
}

type PaymentResponse struct {
	TransactionID     string  `json:"transaction_id"`
	Status            string  `json:"status"`
	AuthorizedAmount  float64 `json:"authorized_amount"`
	Currency          string  `json:"currency"`
	FraudScore        float64 `json:"fraud_score"`
	FastPathLatencyMs float64 `json:"fast_path_latency_ms"`
	DurableTracking   string  `json:"durable_tracking"`
}

func main() {
	engine := octopus_live.NewLiveEngine()

	// 1. Register Models for OpenAPI Schema & Validation
	engine.RegisterModel("PaymentRequest", PaymentRequest{})
	engine.RegisterModel("PaymentResponse", PaymentResponse{})

	// 2. Register Native In-Memory Activities (<1ms execution)
	engine.RegisterActivity("ScreenFraud", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		amount, _ := input["amount"].(float64)
		score := 0.02
		decision := "APPROVE"
		if amount >= 1000 {
			score = 0.85
			decision = "REVIEW"
		}
		return map[string]interface{}{
			"fraud_score":    score,
			"fraud_decision": decision,
		}, nil
	})

	engine.RegisterActivity("AuthorizeCard", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		txnID := fmt.Sprintf("txn_%d", time.Now().UnixNano()/1000000)
		return map[string]interface{}{
			"transaction_id": txnID,
			"auth_code":      "AUTH_9921",
			"settled":        true,
		}, nil
	})

	// Response Packaging Activity (is_output: true)
	engine.RegisterActivity("FormatPaymentResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		amount, _ := input["amount"].(float64)
		currency, _ := input["currency"].(string)
		if currency == "" {
			currency = "USD"
		}
		fraudScore, _ := input["fraud_score"].(float64)
		txnID, _ := input["transaction_id"].(string)
		if txnID == "" {
			txnID = "txn_unknown"
		}

		return map[string]interface{}{
			"transaction_id":       txnID,
			"status":               "AUTHORIZED",
			"authorized_amount":    amount,
			"currency":             currency,
			"fraud_score":          fraudScore,
			"fast_path_latency_ms": 0.42,
			"durable_tracking":     "REPLICATED_TO_TEMPORAL_AND_SQLITE",
		}, nil
	})

	// 3. Load Blueprint from YAML (Zero-Import persistence & Temporal config)
	engine.LoadBlueprintFile("payment_workflow.yaml")

	port := os.Getenv("PORT")
	if port == "" {
		port = "8000"
	}
	if !strings.HasPrefix(port, ":") {
		port = ":" + port
	}

	r := engine.GetGinEngine()
	fmt.Println("\n==================================================================")
	fmt.Printf("🚀 OCTOPUS Live Engine - Go Hybrid Payment API on port %s\n", port)
	fmt.Printf("📖 Swagger Docs:    http://localhost%s/docs\n", port)
	fmt.Printf("🎨 Live Visualizer: http://localhost%s/visualizer?domain=hybrid_payment_gateway\n", port)
	fmt.Println("==================================================================")

	if err := r.Run(port); err != nil {
		log.Fatalf("Server failed to start: %v", err)
	}
}
