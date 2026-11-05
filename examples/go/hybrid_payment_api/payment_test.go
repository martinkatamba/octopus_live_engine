package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/martinkatamba/octopus_live_engine/go/pkg/octopus_live"
)

func setupPaymentEngine() *octopus_live.OctopusLiveEngine {
	gin.SetMode(gin.TestMode)
	engine := octopus_live.NewLiveEngine()
	engine.RegisterModel("PaymentRequest", PaymentRequest{})
	engine.RegisterModel("PaymentResponse", PaymentResponse{})

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
		return map[string]interface{}{
			"transaction_id": "txn_test_9921",
			"auth_code":      "AUTH_9921",
			"settled":        true,
		}, nil
	})

	engine.RegisterActivity("FormatPaymentResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		amount, _ := input["amount"].(float64)
		currency, _ := input["currency"].(string)
		if currency == "" {
			currency = "USD"
		}
		fraudScore, _ := input["fraud_score"].(float64)
		txnID, _ := input["transaction_id"].(string)

		return map[string]interface{}{
			"transaction_id":       txnID,
			"status":               "AUTHORIZED",
			"authorized_amount":    amount,
			"currency":             currency,
			"fraud_score":          fraudScore,
			"fast_path_latency_ms": 0.35,
			"durable_tracking":     "REPLICATED_TO_TEMPORAL_AND_SQLITE",
		}, nil
	})

	engine.LoadBlueprintFile("payment_workflow.yaml")
	return engine
}

func TestGoHybridPaymentFastPath(t *testing.T) {
	engine := setupPaymentEngine()
	router := engine.GetGinEngine()

	payload := map[string]interface{}{
		"account_id": "acc_go_8812",
		"amount":     499.00,
		"currency":   "USD",
		"card_token": "tok_visa_fast",
	}
	body, _ := json.Marshal(payload)
	req, _ := http.NewRequest("POST", "/api/v1/payments/authorize", bytes.NewBuffer(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Job-ID", "tx-go-hybrid-001")

	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != 200 {
		t.Fatalf("Expected 200 OK, got %d: %s", w.Code, w.Body.String())
	}

	var resp PaymentResponse
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("Failed to parse response: %v", err)
	}

	if resp.Status != "AUTHORIZED" {
		t.Errorf("Expected status AUTHORIZED, got %s", resp.Status)
	}
	if resp.AuthorizedAmount != 499.00 {
		t.Errorf("Expected amount 499.00, got %f", resp.AuthorizedAmount)
	}
	if resp.DurableTracking != "REPLICATED_TO_TEMPORAL_AND_SQLITE" {
		t.Errorf("Expected durable tracking string, got %s", resp.DurableTracking)
	}
}
