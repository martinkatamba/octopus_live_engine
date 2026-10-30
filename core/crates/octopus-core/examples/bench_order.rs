//! The order fulfillment workflow with Rust activities: what the core costs
//! on its own, with no host language. `cargo run --release --example
//! bench_order [items...]` prints the median run time per item count.

use std::sync::Arc;
use std::time::Instant;

use octopus_core::json::Value;
use octopus_core::{vmap, Engine, ValueMap};

const BLUEPRINT: &str = r#"
workflow_name: order_fulfillment_pipeline
version: "1.0.0"
api:
  - route: /api/v1/orders/fulfill
    method: POST
    steps:
      - id: step_validate_order
        activity: ValidateOrder
      - id: step_reserve_items
        activity: ReserveInventoryItem
        loop:
          items_field: "items"
          item_variable: "item"
          max_iterations: 1000
      - id: step_route_order
        branching_step: true
        condition:
          field: "order_total"
          operator: "gt"
          value: 1000
          then_steps:
            - id: step_express_shipping
              activity: ScheduleShipping
          else_steps:
            - id: step_standard_shipping
              activity: ScheduleShipping
      - id: step_send_notification
        activity: SendCustomerNotification
        async: true
      - id: step_finalize_order
        activity: FinalizeOrderResponse
        is_output: true
"#;

fn activity(f: fn(&ValueMap) -> ValueMap) -> octopus_core::domain::ports::outbound::ActivityFunc {
    Arc::new(move |p: &ValueMap| Ok(f(p)))
}

fn engine() -> Engine {
    let mut e = Engine::new("");
    e.register_blueprint_yaml(BLUEPRINT).unwrap();
    e.register_activity_func(
        "ValidateOrder",
        activity(|p| {
            let n = p
                .get("items")
                .and_then(|v| v.as_array())
                .map_or(0, |a| a.len());
            vmap! { "order_total" => 1500.0, "order_validated" => true, "items_count" => n as i64 }
        }),
    );
    e.register_activity_func(
        "ReserveInventoryItem",
        activity(|p| {
            let mut reserved = p
                .get("reserved_items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let sku = p
                .get("item")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("sku"))
                .cloned()
                .unwrap_or(Value::Null);
            reserved.push(Value::Object(
                vmap! { "sku" => sku.clone(), "status" => "ALLOCATED" },
            ));
            vmap! { "reserved_items" => Value::Array(reserved), "last_reserved_sku" => sku }
        }),
    );
    e.register_activity_func(
        "ScheduleShipping",
        activity(|_| vmap! { "shipping_method" => "PRIORITY_EXPRESS_AIR", "estimated_delivery_days" => 1i64 }),
    );
    e.register_activity_func(
        "SendCustomerNotification",
        activity(|_| vmap! { "notification_dispatched" => true }),
    );
    e.register_activity_func(
        "FinalizeOrderResponse",
        activity(|p| vmap! { "status" => "FULFILLED", "order_id" => p.get("order_id").cloned().unwrap_or(Value::Null) }),
    );
    e
}

fn order(items: usize) -> ValueMap {
    let items: Vec<Value> = (0..items)
        .map(|i| {
            Value::Object(
                vmap! { "sku" => format!("SKU_{i}"), "quantity" => 1i64, "unit_price" => 99.0 },
            )
        })
        .collect();
    vmap! { "customer_id" => "cust_vip_440", "customer_tier" => "VIP", "order_id" => "ord_1001", "items" => Value::Array(items) }
}

fn main() {
    let counts: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let counts = if counts.is_empty() {
        vec![1, 10, 50, 100]
    } else {
        counts
    };
    let e = engine();
    println!("| items | median µs | µs per item |");
    println!("|---:|---:|---:|");
    for n in counts {
        let input = order(n);
        let runs = (2000 / (n + 5)).max(20);
        for _ in 0..runs / 4 {
            e.execute_sync("order_fulfillment_pipeline", input.clone())
                .unwrap();
        }
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                e.execute_sync("order_fulfillment_pipeline", input.clone())
                    .unwrap();
                t.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = times[times.len() / 2];
        println!("| {n} | {median:.0} | {:.1} |", median / n as f64);
    }
}
