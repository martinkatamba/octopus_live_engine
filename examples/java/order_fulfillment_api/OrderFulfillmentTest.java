package io.octopus.examples.orderfulfillment;

import io.octopus.live.LiveEngine;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.ExecutionStatus;

import java.util.*;

public class OrderFulfillmentTest {

    private static LiveEngine setupTestEngine() throws Exception {
        LiveEngine engine = new LiveEngine();
        engine.registerModel("OrderItem", OrderFulfillmentApplication.OrderItem.class);
        engine.registerModel("OrderFulfillmentRequest", OrderFulfillmentApplication.OrderFulfillmentRequest.class);
        engine.registerModel("OrderFulfillmentResponse", OrderFulfillmentApplication.OrderFulfillmentResponse.class);

        engine.registerActivity("ValidateOrder", input -> {
            double total = 0.0;
            int count = 0;
            Object itemsObj = input.get("items");
            if (itemsObj instanceof List<?> itemsList) {
                count = itemsList.size();
                for (Object raw : itemsList) {
                    if (raw instanceof Map<?, ?> itm) {
                        int qty = 1;
                        if (itm.get("quantity") instanceof Number num) qty = num.intValue();
                        double price = 0.0;
                        if (itm.get("unit_price") instanceof Number num) price = num.doubleValue();
                        total += qty * price;
                    }
                }
            }
            String tier = input.getOrDefault("customer_tier", "STANDARD").toString();
            if ("VIP".equalsIgnoreCase(tier) && total < 1000.0) {
                total = 1500.0;
            }
            Map<String, Object> out = new HashMap<>();
            out.put("order_total", total);
            out.put("order_validated", true);
            out.put("items_count", count);
            return out;
        });

        engine.registerActivity("ReserveInventoryItem", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("last_reserved_sku", "SKU_TEST");
            return out;
        });

        engine.registerActivity("ScheduleExpressShipping", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("shipping_method", "PRIORITY_EXPRESS_AIR");
            out.put("courier", "FedEx Priority Overnight");
            out.put("estimated_delivery_days", 1);
            return out;
        });

        engine.registerActivity("ScheduleStandardShipping", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("shipping_method", "STANDARD_GROUND");
            out.put("courier", "UPS Ground");
            out.put("estimated_delivery_days", 3);
            return out;
        });

        engine.registerActivity("SendCustomerNotification", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("notification_dispatched", true);
            return out;
        });

        engine.registerActivity("FinalizeOrderResponse", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("order_id", input.getOrDefault("order_id", "ord_unknown"));
            out.put("fulfillment_id", "ful_test_123");
            out.put("status", "FULFILLED");
            out.put("order_total", input.get("order_total"));
            out.put("shipping_method", input.getOrDefault("shipping_method", "STANDARD_GROUND"));
            out.put("estimated_delivery_days", input.getOrDefault("estimated_delivery_days", 3));
            out.put("fast_path_latency_ms", 0.45);
            out.put("durable_tracking", "REPLICATED_TO_TEMPORAL_AND_SQLITE");
            return out;
        });

        engine.loadBlueprintsFromDirectory("octopus_live_engine/examples/java/order_fulfillment_api");
        return engine;
    }

    public static void main(String[] args) throws Exception {
        System.out.println("Running Java Order Fulfillment Tests...");
        LiveEngine engine = setupTestEngine();

        // 1. Standard Order Test
        Map<String, Object> stdReq = new HashMap<>();
        stdReq.put("customer_id", "cust_std_001");
        stdReq.put("customer_tier", "STANDARD");
        stdReq.put("order_id", "ord_std_100");
        stdReq.put("shipping_address", "123 Elm St");
        List<Map<String, Object>> stdItems = new ArrayList<>();
        Map<String, Object> itm1 = new HashMap<>();
        itm1.put("sku", "SKU_BOOK");
        itm1.put("quantity", 2);
        itm1.put("unit_price", 25.0);
        stdItems.add(itm1);
        stdReq.put("items", stdItems);

        ExecutionState stdRes = engine.executeSync("order_fulfillment_pipeline", stdReq);
        if (stdRes.getStatus() != ExecutionStatus.COMPLETED) {
            throw new AssertionError("Standard order failed with status: " + stdRes.getStatus());
        }
        if (!"STANDARD_GROUND".equals(stdRes.getOutputData().get("shipping_method"))) {
            throw new AssertionError("Expected STANDARD_GROUND, got: " + stdRes.getOutputData().get("shipping_method"));
        }
        System.out.println("✓ Test Standard Order: PASSED");

        // 2. VIP Order Test
        Map<String, Object> vipReq = new HashMap<>();
        vipReq.put("customer_id", "cust_vip_007");
        vipReq.put("customer_tier", "VIP");
        vipReq.put("order_id", "ord_vip_7788");
        vipReq.put("shipping_address", "100 Ocean Dr");
        List<Map<String, Object>> vipItems = new ArrayList<>();
        Map<String, Object> itmVip = new HashMap<>();
        itmVip.put("sku", "SKU_LAPTOP");
        itmVip.put("quantity", 2);
        itmVip.put("unit_price", 1499.0);
        vipItems.add(itmVip);
        vipReq.put("items", vipItems);

        ExecutionState vipRes = engine.executeSync("order_fulfillment_pipeline", vipReq);
        if (vipRes.getStatus() != ExecutionStatus.COMPLETED) {
            throw new AssertionError("VIP order failed with status: " + vipRes.getStatus());
        }
        if (!"PRIORITY_EXPRESS_AIR".equals(vipRes.getOutputData().get("shipping_method"))) {
            throw new AssertionError("Expected PRIORITY_EXPRESS_AIR, got: " + vipRes.getOutputData().get("shipping_method"));
        }
        System.out.println("✓ Test VIP Order: PASSED");

        // 3. Visualizer Diagram Test
        Map<String, Object> diagram = engine.getVisualizerUseCase().compileX6Diagram("order_fulfillment_pipeline", null);
        @SuppressWarnings("unchecked")
        List<Map<String, Object>> nodes = (List<Map<String, Object>>) diagram.get("nodes");
        if (nodes == null || nodes.isEmpty()) {
            throw new AssertionError("Expected non-empty nodes in diagram");
        }

        boolean foundEndCompleted = false;
        boolean foundLoopBadge = false;
        boolean foundAsyncBadge = false;

        for (Map<String, Object> node : nodes) {
            String id = (String) node.get("id");
            String label = (String) node.get("label");
            if ("node__end".equals(id) && (label != null && (label.contains("200") || label.contains("COMPLETED")))) {
                foundEndCompleted = true;
            }
            if ("node__step_reserve_items".equals(id) && label != null && label.contains("[Loop Iteration]")) {
                foundLoopBadge = true;
            }
            if ("node__step_send_notification".equals(id) && label != null && label.contains("[Async Background]")) {
                foundAsyncBadge = true;
            }
        }

        if (!foundEndCompleted) throw new AssertionError("Terminal node node__end should be labeled with success status code (e.g. 200 OK / COMPLETED)");
        if (!foundLoopBadge) throw new AssertionError("Loop node node__step_reserve_items should have [Loop Iteration] badge");
        if (!foundAsyncBadge) throw new AssertionError("Async node node__step_send_notification should have [Async Background] badge");
        System.out.println("✓ Test Visualizer Diagram: PASSED");

        System.out.println("\nAll Java Order Fulfillment Tests PASSED successfully!");
    }
}
