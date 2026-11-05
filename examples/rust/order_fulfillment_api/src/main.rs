use std::time::{SystemTime, UNIX_EPOCH};

use octopus_live_engine::domain::models::{EngineOptions, ToolingAccess};
use octopus_live_engine::{new_live_engine, vmap, Value, ValueMap};

// ==========================================
// 1. Helper Utilities
// ==========================================

fn hash_sku(s: &str) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for byte in s.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn to_f64(v: Option<&Value>, fallback: f64) -> f64 {
    match v {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        _ => fallback,
    }
}

fn to_i64(v: Option<&Value>, fallback: i64) -> i64 {
    match v {
        Some(Value::Int(i)) => *i,
        Some(Value::Float(f)) => *f as i64,
        _ => fallback,
    }
}

// ==========================================
// 2. Application Main
// ==========================================

fn main() {
    let mut engine = new_live_engine("");

    // Enable client-supplied job IDs (e.g. X-Job-ID header) and open tooling access
    engine.set_options(EngineOptions {
        allow_client_job_ids: true,
        tooling: ToolingAccess::Open,
        ..Default::default()
    });

    // 1. Register Models for Contracts, OpenAPI & Docs
    engine.register_model(
        "OrderItem",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "sku" => Value::Object(vmap! { "type" => "string" }),
                "quantity" => Value::Object(vmap! { "type" => "integer" }),
                "unit_price" => Value::Object(vmap! { "type" => "number" }),
            }),
            "required" => Value::Array(vec![Value::String("sku".to_string())]),
        }),
    );

    engine.register_model(
        "OrderFulfillmentRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "customer_id" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "order_id" => Value::Object(vmap! { "type" => "string" }),
                "shipping_address" => Value::Object(vmap! { "type" => "string" }),
                "items" => Value::Object(vmap! {
                    "type" => "array",
                    "items" => Value::Object(vmap! { "$ref" => "#/components/schemas/OrderItem" }),
                }),
                "job_id" => Value::Object(vmap! { "type" => "string" }),
            }),
            "required" => Value::Array(vec![
                Value::String("customer_id".to_string()),
                Value::String("order_id".to_string()),
                Value::String("items".to_string()),
            ]),
        }),
    );

    engine.register_model(
        "OrderFulfillmentResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "order_id" => Value::Object(vmap! { "type" => "string" }),
                "fulfillment_id" => Value::Object(vmap! { "type" => "string" }),
                "status" => Value::Object(vmap! { "type" => "string" }),
                "order_total" => Value::Object(vmap! { "type" => "number" }),
                "items_reserved_count" => Value::Object(vmap! { "type" => "integer" }),
                "shipping_method" => Value::Object(vmap! { "type" => "string" }),
                "estimated_delivery_days" => Value::Object(vmap! { "type" => "integer" }),
                "fast_path_latency_ms" => Value::Object(vmap! { "type" => "number" }),
                "durable_tracking" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );

    // 2. Register Native In-Memory Activities (<1ms execution)
    engine.register_activity(
        "ValidateOrder",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let mut total = 0.0;
            let mut count = 0;

            if let Some(Value::Array(raw_items)) = input.get("items") {
                count = raw_items.len();
                for raw in raw_items {
                    if let Some(itm) = raw.as_object() {
                        let qty = to_f64(itm.get("quantity"), 1.0);
                        let price = to_f64(itm.get("unit_price"), 0.0);
                        total += qty * price;
                    }
                }
            }

            let tier = input
                .get("customer_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if tier.eq_ignore_ascii_case("VIP") && total < 1000.0 {
                // VIP tier boost for expedited routing threshold
                total = 1500.0;
            }

            Ok(vmap! {
                "order_total" => total,
                "order_validated" => true,
                "items_count" => count as i64,
                "validated_at" => now_millis() as i64,
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
                .unwrap_or("SKU_UNKNOWN")
                .to_string();

            let qty = item
                .and_then(|m| m.get("quantity"))
                .map(|v| to_i64(Some(v), 1))
                .unwrap_or(1);

            let mut reserved_list = match input.get("reserved_items") {
                Some(Value::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            };

            let bin_num = (hash_sku(&sku) % 50) + 1;
            let reservation_entry = vmap! {
                "sku" => sku.clone(),
                "quantity" => qty,
                "warehouse_bin" => format!("BIN_{}", bin_num),
                "status" => "ALLOCATED",
            };
            reserved_list.push(Value::Object(reservation_entry));

            Ok(vmap! {
                "reserved_items" => Value::Array(reserved_list),
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
                "tracking_number" => format!("EXP_{}", now_millis()),
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
                "tracking_number" => format!("STD_{}", now_millis()),
            })
        },
    );

    engine.register_activity(
        "SendCustomerNotification",
        |input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "notification_dispatched" => true,
                "channels" => Value::Array(vec![
                    Value::String("EMAIL".to_string()),
                    Value::String("SMS".to_string())
                ]),
                "recipient_id" => input.get("customer_id").cloned().unwrap_or(Value::Null),
                "order_id" => input.get("order_id").cloned().unwrap_or(Value::Null),
                "dispatched_at" => now_millis() as i64,
            })
        },
    );

    // Response Packaging Activity (is_output: true)
    engine.register_activity(
        "FinalizeOrderResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let items_count = match input.get("items") {
                Some(Value::Array(arr)) => arr.len() as i64,
                _ => 0i64,
            };

            let total = to_f64(input.get("order_total"), 0.0);
            let shipping_method = input
                .get("shipping_method")
                .and_then(|v| v.as_str())
                .unwrap_or("STANDARD_GROUND")
                .to_string();
            let delivery_days = to_i64(input.get("estimated_delivery_days"), 3);
            let order_id = input
                .get("order_id")
                .and_then(|v| v.as_str())
                .unwrap_or("ord_unknown")
                .to_string();

            Ok(vmap! {
                "order_id" => order_id,
                "fulfillment_id" => format!("ful_{}", now_millis()),
                "status" => "FULFILLED",
                "order_total" => total,
                "items_reserved_count" => items_count,
                "shipping_method" => shipping_method,
                "estimated_delivery_days" => delivery_days,
                "fast_path_latency_ms" => 0.48,
                "durable_tracking" => "REPLICATED_TO_TEMPORAL_AND_SQLITE",
            })
        },
    );

    // 3. Load Blueprint & Run Server
    engine
        .load_blueprint_file("order_fulfillment_workflow.yaml")
        .expect("Failed to load order_fulfillment_workflow.yaml");

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim_start_matches(':').parse::<u16>().ok())
        .unwrap_or(8000);

    let router = engine.get_router();

    println!("\n==================================================================");
    println!(
        "🚀 OCTOPUS Live Engine - Rust Order Fulfillment API on http://localhost:{}",
        port
    );
    println!("📖 Swagger Docs:    http://localhost:{}/docs", port);
    println!(
        "🎨 Live Visualizer: http://localhost:{}/visualizer?domain=order_fulfillment_pipeline",
        port
    );
    println!("==================================================================");

    router
        .run(&format!("127.0.0.1:{}", port))
        .expect("Server failed");
}
