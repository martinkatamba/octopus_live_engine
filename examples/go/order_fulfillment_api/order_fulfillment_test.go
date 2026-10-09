package main

import (
	"context"
	"strings"
	"testing"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

func setupTestEngine(t *testing.T) *octopus_live.OctopusLiveEngine {
	engine := octopus_live.NewLiveEngine()
	engine.RegisterModel("OrderItem", OrderItem{})
	engine.RegisterModel("OrderFulfillmentRequest", OrderFulfillmentRequest{})
	engine.RegisterModel("OrderFulfillmentResponse", OrderFulfillmentResponse{})

	engine.RegisterActivity("ValidateOrder", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		var total float64
		count := 0
		if rawItems, ok := input["items"].([]interface{}); ok {
			count = len(rawItems)
			for _, raw := range rawItems {
				if itm, ok := raw.(map[string]interface{}); ok {
					qty := toFloat64(itm["quantity"], 1.0)
					price := toFloat64(itm["unit_price"], 0.0)
					total += qty * price
				}
			}
		}
		tier, _ := input["customer_tier"].(string)
		if tier == "VIP" && total < 1000 {
			total = 1500.0
		}
		return map[string]interface{}{
			"order_total":     total,
			"order_validated": true,
			"items_count":     count,
		}, nil
	})

	engine.RegisterActivity("ReserveInventoryItem", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		item, _ := input["item"].(map[string]interface{})
		if item == nil {
			item, _ = input["_item"].(map[string]interface{})
		}
		sku := "SKU_DEFAULT"
		if item != nil {
			if s, ok := item["sku"].(string); ok {
				sku = s
			}
		}
		return map[string]interface{}{
			"last_reserved_sku": sku,
		}, nil
	})

	engine.RegisterActivity("ScheduleExpressShipping", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"shipping_method":         "PRIORITY_EXPRESS_AIR",
			"courier":                 "FedEx Priority Overnight",
			"estimated_delivery_days": 1,
		}, nil
	})

	engine.RegisterActivity("ScheduleStandardShipping", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"shipping_method":         "STANDARD_GROUND",
			"courier":                 "UPS Ground",
			"estimated_delivery_days": 3,
		}, nil
	})

	engine.RegisterActivity("SendCustomerNotification", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		return map[string]interface{}{
			"notification_dispatched": true,
		}, nil
	})

	engine.RegisterActivity("FinalizeOrderResponse", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		itemsCount := 0
		if rawItems, ok := input["items"].([]interface{}); ok {
			itemsCount = len(rawItems)
		}
		total := toFloat64(input["order_total"], 0.0)
		shippingMethod, _ := input["shipping_method"].(string)
		deliveryDays := toInt(input["estimated_delivery_days"], 3)
		orderID, _ := input["order_id"].(string)

		return map[string]interface{}{
			"order_id":                orderID,
			"fulfillment_id":         "ful_test_123",
			"status":                  "FULFILLED",
			"order_total":             total,
			"items_reserved_count":    itemsCount,
			"shipping_method":         shippingMethod,
			"estimated_delivery_days": deliveryDays,
			"fast_path_latency_ms":    0.5,
			"durable_tracking":        "REPLICATED_TO_TEMPORAL_AND_SQLITE",
		}, nil
	})

	if err := engine.LoadBlueprintFile("order_fulfillment_workflow.yaml"); err != nil {
		t.Fatalf("Failed to load blueprint: %v", err)
	}

	return engine
}

func TestOrderFulfillment_StandardOrder(t *testing.T) {
	engine := setupTestEngine(t)

	reqBody := map[string]interface{}{
		"customer_id":      "cust_standard_001",
		"customer_tier":    "STANDARD",
		"order_id":         "ord_std_9921",
		"shipping_address": "742 Evergreen Terrace, Springfield, OR",
		"items": []interface{}{
			map[string]interface{}{
				"sku":        "SKU_BOOK_01",
				"quantity":   2,
				"unit_price": 25.0,
			},
			map[string]interface{}{
				"sku":        "SKU_PEN_02",
				"quantity":   5,
				"unit_price": 4.5,
			},
		},
	}

	res, err := engine.ExecuteSync(context.Background(), "order_fulfillment_pipeline", reqBody)
	if err != nil {
		t.Fatalf("Failed to execute workflow: %v", err)
	}

	if res.Status != "COMPLETED" {
		t.Fatalf("Expected status COMPLETED, got: %s", res.Status)
	}

	shippingMethod, ok := res.OutputData["shipping_method"].(string)
	if !ok || shippingMethod != "STANDARD_GROUND" {
		t.Fatalf("Expected shipping_method STANDARD_GROUND, got %v", res.OutputData["shipping_method"])
	}
}

func TestOrderFulfillment_VIPOrder(t *testing.T) {
	engine := setupTestEngine(t)

	reqBody := map[string]interface{}{
		"customer_id":      "cust_vip_007",
		"customer_tier":    "VIP",
		"order_id":         "ord_vip_7788",
		"shipping_address": "100 Ocean Drive, Miami, FL",
		"items": []interface{}{
			map[string]interface{}{
				"sku":        "SKU_LAPTOP_PRO",
				"quantity":   2,
				"unit_price": 1499.0,
			},
		},
	}

	res, err := engine.ExecuteSync(context.Background(), "order_fulfillment_pipeline", reqBody)
	if err != nil {
		t.Fatalf("Failed to execute workflow: %v", err)
	}

	if res.Status != "COMPLETED" {
		t.Fatalf("Expected status COMPLETED, got: %s", res.Status)
	}

	shippingMethod, ok := res.OutputData["shipping_method"].(string)
	if !ok || shippingMethod != "PRIORITY_EXPRESS_AIR" {
		t.Fatalf("Expected shipping_method PRIORITY_EXPRESS_AIR, got %v", res.OutputData["shipping_method"])
	}
}

func TestOrderFulfillment_VisualizerDiagram(t *testing.T) {
	engine := setupTestEngine(t)

	diagram, err := engine.VisualizerService.CompileX6Diagram(context.Background(), "order_fulfillment_pipeline", nil, nil, nil)
	if err != nil {
		t.Fatalf("Failed to compile diagram: %v", err)
	}

	nodes, ok := diagram["nodes"].([]map[string]interface{})
	if !ok {
		t.Fatalf("Expected nodes array in diagram, got %T", diagram["nodes"])
	}

	foundEndCompleted := false
	foundLoopBadge := false
	foundAsyncBadge := false

	for _, node := range nodes {
		id, _ := node["id"].(string)
		label, _ := node["label"].(string)
		if id == "node__end" {
			if label == "COMPLETED" || strings.Contains(label, "200") || strings.Contains(label, "OK") {
				foundEndCompleted = true
			}
		}
		if id == "node__step_reserve_items" {
			if len(label) > 0 {
				foundLoopBadge = true
			}
		}
		if id == "node__step_send_notification" {
			if len(label) > 0 {
				foundAsyncBadge = true
			}
		}
	}

	if !foundEndCompleted {
		t.Errorf("Terminal node node__end should be labeled with success status code (e.g. 200 OK / COMPLETED)")
	}
	if !foundLoopBadge {
		t.Errorf("Loop node step_reserve_items was not found or formatted properly")
	}
	if !foundAsyncBadge {
		t.Errorf("Async node step_send_notification was not found or formatted properly")
	}
}
