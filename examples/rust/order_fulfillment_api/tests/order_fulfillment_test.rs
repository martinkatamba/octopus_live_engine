use octopus_live_engine::domain::models::ExecutionStatus;
use octopus_live_engine::domain::ports::IVisualizerCompiler;
use octopus_live_engine::{new_live_engine, vmap, Value, ValueMap};

fn setup_test_engine() -> octopus_live_engine::engine::OctopusLiveEngine {
    let mut engine = new_live_engine("");

    engine.register_activity(
        "ValidateOrder",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let mut total = 0.0;
            let mut count = 0;
            if let Some(Value::Array(items)) = input.get("items") {
                count = items.len();
                for item_val in items {
                    if let Some(item) = item_val.as_object() {
                        let qty = item.get("quantity").and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let price = item.get("unit_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        total += qty * price;
                    }
                }
            }

            let tier = input
                .get("customer_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if tier.eq_ignore_ascii_case("VIP") && total < 1000.0 {
                total = 1500.0;
            }

            Ok(vmap! {
                "order_total" => total,
                "order_validated" => true,
                "items_count" => count as i64,
            })
        },
    );

    engine.register_activity(
        "ReserveInventoryItem",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let item = input
                .get("item")
                .and_then(|v| v.as_object())
                .or_else(|| input.get("_item").and_then(|v| v.as_object()));

            let sku = item
                .and_then(|m| m.get("sku"))
                .and_then(|v| v.as_str())
                .unwrap_or("SKU_DEFAULT")
                .to_string();

            Ok(vmap! {
                "last_reserved_sku" => sku,
            })
        },
    );

    engine.register_activity(
        "ScheduleExpressShipping",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "shipping_method" => "PRIORITY_EXPRESS_AIR",
                "courier" => "FedEx Priority Overnight",
                "estimated_delivery_days" => 1i64,
            })
        },
    );

    engine.register_activity(
        "ScheduleStandardShipping",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "shipping_method" => "STANDARD_GROUND",
                "courier" => "UPS Ground",
                "estimated_delivery_days" => 3i64,
            })
        },
    );

    engine.register_activity(
        "SendCustomerNotification",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "notification_dispatched" => true,
            })
        },
    );

    engine.register_activity(
        "FinalizeOrderResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let items_count = match input.get("items") {
                Some(Value::Array(arr)) => arr.len() as i64,
                _ => 0i64,
            };
            let total = input.get("order_total").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let shipping_method = input
                .get("shipping_method")
                .and_then(|v| v.as_str())
                .unwrap_or("STANDARD_GROUND")
                .to_string();
            let delivery_days = input
                .get("estimated_delivery_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(3);
            let order_id = input
                .get("order_id")
                .and_then(|v| v.as_str())
                .unwrap_or("ord_unknown")
                .to_string();

            Ok(vmap! {
                "order_id" => order_id,
                "fulfillment_id" => "ful_test_123",
                "status" => "FULFILLED",
                "order_total" => total,
                "items_reserved_count" => items_count,
                "shipping_method" => shipping_method,
                "estimated_delivery_days" => delivery_days,
                "fast_path_latency_ms" => 0.5,
                "durable_tracking" => "REPLICATED_TO_TEMPORAL_AND_SQLITE",
            })
        },
    );

    engine
        .load_blueprint_file("order_fulfillment_workflow.yaml")
        .expect("Failed to load blueprint");

    engine
}

#[test]
fn test_order_fulfillment_standard_order() {
    let engine = setup_test_engine();

    let items = vec![
        Value::Object(vmap! {
            "sku" => "SKU_BOOK_01",
            "quantity" => 2i64,
            "unit_price" => 25.0,
        }),
        Value::Object(vmap! {
            "sku" => "SKU_PEN_02",
            "quantity" => 5i64,
            "unit_price" => 4.5,
        }),
    ];

    let payload = vmap! {
        "customer_id" => "cust_standard_001",
        "customer_tier" => "STANDARD",
        "order_id" => "ord_std_9921",
        "shipping_address" => "742 Evergreen Terrace, Springfield, OR",
        "items" => Value::Array(items),
    };

    let state = engine
        .execute_sync("order_fulfillment_pipeline", payload)
        .expect("Execution failed");

    assert_eq!(state.status, ExecutionStatus::Completed);
    assert_eq!(
        state
            .data
            .get("shipping_method")
            .and_then(|v: &Value| v.as_str()),
        Some("STANDARD_GROUND")
    );
}

#[test]
fn test_order_fulfillment_vip_order() {
    let engine = setup_test_engine();

    let items = vec![Value::Object(vmap! {
        "sku" => "SKU_LAPTOP_PRO",
        "quantity" => 2i64,
        "unit_price" => 1499.0,
    })];

    let payload = vmap! {
        "customer_id" => "cust_vip_007",
        "customer_tier" => "VIP",
        "order_id" => "ord_vip_7788",
        "shipping_address" => "100 Ocean Drive, Miami, FL",
        "items" => Value::Array(items),
    };

    let state = engine
        .execute_sync("order_fulfillment_pipeline", payload)
        .expect("Execution failed");

    assert_eq!(state.status, ExecutionStatus::Completed);
    assert_eq!(
        state
            .data
            .get("shipping_method")
            .and_then(|v: &Value| v.as_str()),
        Some("PRIORITY_EXPRESS_AIR")
    );
}

#[test]
fn test_order_fulfillment_visualizer_diagram() {
    let engine = setup_test_engine();

    let diagram = engine
        .visualizer_service
        .compile_x6_diagram("order_fulfillment_pipeline", None, None, None)
        .expect("Failed to compile diagram");

    let nodes = diagram
        .get("nodes")
        .and_then(|v: &Value| v.as_array())
        .expect("Expected nodes array in diagram");

    let mut found_end_completed = false;
    let mut found_loop_badge = false;
    let mut found_async_badge = false;

    for node in nodes {
        if let Some(m) = node.as_object() {
            let id = m.get("id").and_then(|v: &Value| v.as_str()).unwrap_or("");
            let label = m.get("label").and_then(|v: &Value| v.as_str()).unwrap_or("");
            if id == "node__end" {
                if label == "COMPLETED" || label.contains("200") || label.contains("OK") {
                    found_end_completed = true;
                }
            }
            if id.contains("step_reserve_items") && !label.is_empty() {
                found_loop_badge = true;
            }
            if id.contains("step_send_notification") && !label.is_empty() {
                found_async_badge = true;
            }
        }
    }

    assert!(
        found_end_completed,
        "Terminal node node__end should be labeled with success status"
    );
    assert!(
        found_loop_badge,
        "Loop node step_reserve_items was not found or formatted properly"
    );
    assert!(
        found_async_badge,
        "Async node step_send_notification was not found or formatted properly"
    );
}
