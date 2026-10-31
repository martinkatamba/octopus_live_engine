use std::time::{SystemTime, UNIX_EPOCH};

use octopus_live_engine::crypto::{base64url_encode, hmac_sha256};
use octopus_live_engine::domain::models::{EngineOptions, ToolingAccess};
use octopus_live_engine::json;
use octopus_live_engine::{new_live_engine, vmap, Value, ValueMap};

const JWT_SECRET: &str = "enterprise-super-secret-key-2026";

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

fn create_hs256_jwt(claims: &ValueMap, secret: &str) -> String {
    let header = vmap! {
        "alg" => "HS256",
        "typ" => "JWT",
    };
    let h_json = json::map_to_json(&header);
    let c_json = json::map_to_json(claims);
    let h_b64 = base64url_encode(h_json.as_bytes());
    let c_b64 = base64url_encode(c_json.as_bytes());
    let signing_input = format!("{}.{}", h_b64, c_b64);
    let sig = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    let s_b64 = base64url_encode(&sig);
    format!("{}.{}", signing_input, s_b64)
}

// ==========================================
// 2. Application Entrypoint
// ==========================================

fn main() {
    let mut engine = new_live_engine("");

    // Enable client-supplied job IDs (X-Job-ID) and open tooling access for visualizer
    engine.set_options(EngineOptions {
        allow_client_job_ids: true,
        tooling: ToolingAccess::Open,
        ..Default::default()
    });

    // -------------------------------------------------------------
    // Register Schemas for OpenAPI 3.0 & Swagger UI
    // -------------------------------------------------------------
    engine.register_model(
        "LoginRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "username" => Value::Object(vmap! { "type" => "string" }),
                "password" => Value::Object(vmap! { "type" => "string" }),
            }),
            "required" => Value::Array(vec![
                Value::String("username".to_string()),
                Value::String("password".to_string()),
            ]),
        }),
    );

    engine.register_model(
        "LoginResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "access_token" => Value::Object(vmap! { "type" => "string" }),
                "token_type" => Value::Object(vmap! { "type" => "string" }),
                "user_id" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "email" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );

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
        "SettlementRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "order_id" => Value::Object(vmap! { "type" => "string" }),
                "currency" => Value::Object(vmap! { "type" => "string" }),
                "card_number" => Value::Object(vmap! { "type" => "string" }),
                "cvv" => Value::Object(vmap! { "type" => "string" }),
                "items" => Value::Object(vmap! {
                    "type" => "array",
                    "items" => Value::Object(vmap! { "$ref" => "#/components/schemas/OrderItem" }),
                }),
                "simulate_decline" => Value::Object(vmap! { "type" => "boolean" }),
            }),
            "required" => Value::Array(vec![
                Value::String("order_id".to_string()),
                Value::String("items".to_string()),
            ]),
        }),
    );

    engine.register_model(
        "SettlementResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "order_id" => Value::Object(vmap! { "type" => "string" }),
                "settlement_id" => Value::Object(vmap! { "type" => "string" }),
                "status" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "subtotal" => Value::Object(vmap! { "type" => "number" }),
                "discount_applied" => Value::Object(vmap! { "type" => "number" }),
                "final_amount" => Value::Object(vmap! { "type" => "number" }),
                "shipping_method" => Value::Object(vmap! { "type" => "string" }),
                "items_count" => Value::Object(vmap! { "type" => "integer" }),
            }),
        }),
    );

    engine.register_model(
        "HoldRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "hold_id" => Value::Object(vmap! { "type" => "string" }),
                "amount" => Value::Object(vmap! { "type" => "number" }),
                "currency" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );

    engine.register_model(
        "HoldResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "hold_id" => Value::Object(vmap! { "type" => "string" }),
                "status" => Value::Object(vmap! { "type" => "string" }),
                "message" => Value::Object(vmap! { "type" => "string" }),
            }),
        }),
    );

    // -------------------------------------------------------------
    // Register Business Activities & Sagas
    // -------------------------------------------------------------

    // Activity: VerifyUserCredentials
    engine.register_activity(
        "VerifyUserCredentials",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let username = input.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let password = input.get("password").and_then(|v| v.as_str()).unwrap_or("");

            if username == "sarah_vip" && password == "secret123" {
                Ok(vmap! {
                    "authenticated" => true,
                    "user_id" => "usr_sarah_vip",
                    "customer_tier" => "VIP",
                    "roles" => Value::Array(vec![Value::String("ROLE_CUSTOMER".to_string()), Value::String("ROLE_VIP".to_string())]),
                    "email" => "sarah.vip@enterprise.com",
                })
            } else if username == "john_standard" && password == "secret123" {
                Ok(vmap! {
                    "authenticated" => true,
                    "user_id" => "usr_john_std",
                    "customer_tier" => "STANDARD",
                    "roles" => Value::Array(vec![Value::String("ROLE_CUSTOMER".to_string())]),
                    "email" => "john.std@enterprise.com",
                })
            } else {
                Err("InvalidCredentials: Invalid username or password. Please verify credentials.".to_string())
            }
        },
    );

    // Activity: IssueJwtToken
    engine.register_activity(
        "IssueJwtToken",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let user_id = input.get("user_id").and_then(|v| v.as_str()).unwrap_or("usr_anon");
            let customer_tier = input.get("customer_tier").and_then(|v| v.as_str()).unwrap_or("STANDARD");
            let email = input.get("email").and_then(|v| v.as_str()).unwrap_or("customer@enterprise.com");
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();

            let claims = vmap! {
                "sub" => user_id,
                "user_id" => user_id,
                "customer_tier" => customer_tier,
                "email" => email,
                "iat" => now,
                "exp" => now + 86400.0,
            };

            let token = create_hs256_jwt(&claims, JWT_SECRET);

            Ok(vmap! {
                "access_token" => token,
                "token_type" => "Bearer",
                "user_id" => user_id,
                "customer_tier" => customer_tier,
                "email" => email,
                "expires_in" => 86400i64,
            })
        },
    );

    // Activity: ValidateOrderPayload
    engine.register_activity(
        "ValidateOrderPayload",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let items = match input.get("items") {
                Some(Value::Array(arr)) if !arr.is_empty() => arr,
                _ => return Err("Order must contain at least one item".to_string()),
            };

            let mut subtotal = 0.0;
            for it in items {
                if let Value::Object(obj) = it {
                    let qty = to_f64(obj.get("quantity"), 1.0);
                    let price = to_f64(obj.get("unit_price"), 0.0);
                    subtotal += qty * price;
                }
            }

            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");

            Ok(vmap! {
                "order_validated" => true,
                "order_id" => order_id,
                "subtotal" => subtotal,
                "items_count" => items.len() as i64,
            })
        },
    );

    // Activity: ReserveInventoryItem (Loop step)
    engine.register_activity(
        "ReserveInventoryItem",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let item_obj = match input.get("current_item") {
                Some(Value::Object(m)) => m,
                _ => return Err("Missing current_item in loop iteration".to_string()),
            };

            let sku = item_obj.get("sku").and_then(|v| v.as_str()).unwrap_or("SKU-GENERIC");
            let qty = to_i64(item_obj.get("quantity"), 1);
            let bin = format!("BIN-WH-{}", hash_sku(sku) % 100);

            Ok(vmap! {
                "item_reserved" => true,
                "sku" => sku,
                "quantity_reserved" => qty,
                "warehouse_bin" => bin,
                "reservation_ts" => now_millis() as i64,
            })
        },
    );

    // Activity: ReleaseInventoryBatch (Saga Compensation for ReserveInventoryItem)
    engine.register_activity(
        "ReleaseInventoryBatch",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            println!("↩️ [SAGA ROLLBACK] Executing compensation: ReleaseInventoryBatch for order {}", order_id);
            Ok(vmap! {
                "inventory_released" => true,
                "rollback_order_id" => order_id,
                "status" => "RELEASED",
            })
        },
    );

    // Activity: ApplyVipPreferentialPricing (VIP Branch)
    engine.register_activity(
        "ApplyVipPreferentialPricing",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let subtotal = to_f64(input.get("subtotal"), 100.0);
            let discount = subtotal * 0.20;
            let final_amt = subtotal - discount;

            Ok(vmap! {
                "pricing_tier" => "VIP_LOYALTY",
                "discount_applied" => discount,
                "shipping_fee" => 0.0,
                "shipping_method" => "FREE_PRIORITY_EXPRESS_AIR",
                "final_amount" => final_amt,
            })
        },
    );

    // Activity: ApplyStandardPricing (Standard Branch)
    engine.register_activity(
        "ApplyStandardPricing",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let subtotal = to_f64(input.get("subtotal"), 100.0);
            let shipping = 15.0;
            let final_amt = subtotal + shipping;

            Ok(vmap! {
                "pricing_tier" => "STANDARD_RETAIL",
                "discount_applied" => 0.0,
                "shipping_fee" => shipping,
                "shipping_method" => "STANDARD_GROUND_FREIGHT",
                "final_amount" => final_amt,
            })
        },
    );

    // Activity: RecordPendingLedger (Saga Step)
    engine.register_activity(
        "RecordPendingLedger",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            Ok(vmap! {
                "ledger_recorded" => true,
                "ledger_id" => format!("LDG-{}", now_millis()),
                "order_id" => order_id,
            })
        },
    );

    // Activity: ReversePendingLedger (Saga Compensation for RecordPendingLedger)
    engine.register_activity(
        "ReversePendingLedger",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            println!("↩️ [SAGA ROLLBACK] Executing compensation: ReversePendingLedger for order {}", order_id);
            Ok(vmap! {
                "ledger_reversed" => true,
                "rollback_order_id" => order_id,
                "status" => "VOIDED",
            })
        },
    );

    // Activity: AuthorizePayment (Can fail to trigger Saga)
    engine.register_activity(
        "AuthorizePayment",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let card = input.get("card_number").and_then(|v| v.as_str()).unwrap_or("");
            let simulate_decline = input.get("simulate_decline").and_then(|v| v.as_bool()).unwrap_or(false);

            if simulate_decline || card.ends_with("0002") || card.contains("declined") {
                return Err("Card authorization declined by issuing bank (insufficient funds / stolen)".to_string());
            }

            let auth_code = format!("AUTH-BANK-{:06}", hash_sku(card) % 1000000);
            Ok(vmap! {
                "payment_authorized" => true,
                "authorization_code" => auth_code,
                "clearing_gateway" => "ENTERPRISE_DIRECT_PAY",
                "authorized_at" => now_millis() as i64,
            })
        },
    );

    // Activity: ReversePaymentAuthorization (Saga Compensation for AuthorizePayment)
    engine.register_activity(
        "ReversePaymentAuthorization",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            println!("↩️ [SAGA ROLLBACK] Executing compensation: ReversePaymentAuthorization for order {}", order_id);
            Ok(vmap! {
                "payment_reversed" => true,
                "refund_reference" => format!("REF-BANK-{}", now_millis()),
                "status" => "VOIDED",
            })
        },
    );

    // Activity: SendAsyncDispatchNotification (Async Background Step)
    engine.register_activity(
        "SendAsyncDispatchNotification",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            let email = input.get("email").and_then(|v| v.as_str()).unwrap_or("customer@enterprise.com");
            println!("⚡ [ASYNC DISPATCH] Sent dispatch notification email for {} to {}", order_id, email);
            Ok(vmap! {
                "notification_sent" => true,
                "recipient_email" => email,
                "dispatched_at" => now_millis() as i64,
            })
        },
    );

    // Activity: FinalizeSettlementResponse (Output Step)
    engine.register_activity(
        "FinalizeSettlementResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").and_then(|v| v.as_str()).unwrap_or("ord_unknown");
            let tier = input.get("customer_tier").and_then(|v| v.as_str()).unwrap_or("STANDARD");
            let subtotal = to_f64(input.get("subtotal"), 0.0);
            let discount = to_f64(input.get("discount_applied"), 0.0);
            let final_amt = to_f64(input.get("final_amount"), subtotal);
            let shipping = input.get("shipping_method").and_then(|v| v.as_str()).unwrap_or("STANDARD_GROUND_FREIGHT");
            let items_count = to_i64(input.get("items_count"), 1);

            Ok(vmap! {
                "order_id" => order_id,
                "settlement_id" => format!("SETTLE-{}", now_millis()),
                "status" => "COMPLETED",
                "customer_tier" => tier,
                "subtotal" => subtotal,
                "discount_applied" => discount,
                "final_amount" => final_amt,
                "shipping_method" => shipping,
                "items_count" => items_count,
            })
        },
    );

    // Activity: HoldInventory (Two-phase settlement)
    engine.register_activity(
        "HoldInventory",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let hold_id = input.get("hold_id").and_then(|v| v.as_str()).unwrap_or("hold_001");
            Ok(vmap! {
                "inventory_held" => true,
                "hold_id" => hold_id,
            })
        },
    );

    // Activity: ReleaseInventoryHold (Compensation for HoldInventory)
    engine.register_activity(
        "ReleaseInventoryHold",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! { "inventory_hold_released" => true })
        },
    );

    // Activity: PreliminaryAuthorization (Two-phase settlement)
    engine.register_activity(
        "PreliminaryAuthorization",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let amount = to_f64(input.get("amount"), 100.0);
            Ok(vmap! {
                "preliminary_auth" => true,
                "held_amount" => amount,
                "hold_token" => format!("HOLD-{}", now_millis()),
            })
        },
    );

    // Activity: ReversePreliminaryAuthorization (Compensation)
    engine.register_activity(
        "ReversePreliminaryAuthorization",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! { "preliminary_auth_reversed" => true })
        },
    );

    // Activity: ReturnHoldReceipt (Output formatter for hold endpoint)
    engine.register_activity(
        "ReturnHoldReceipt",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let hold_id = input.get("hold_id").and_then(|v| v.as_str()).unwrap_or("hold_001");
            Ok(vmap! {
                "hold_id" => hold_id,
                "status" => "FUNDS_HELD_AWAITING_CLEARING",
                "message" => "Preliminary funds held. Awaiting clearing signal to finalize settlement.",
            })
        },
    );

    // Activity: ConfirmClearedSettlement (Resumed after await_signal)
    engine.register_activity(
        "ConfirmClearedSettlement",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let hold_id = input.get("hold_id").and_then(|v| v.as_str()).unwrap_or("hold_001");
            Ok(vmap! {
                "settlement_confirmed" => true,
                "hold_id" => hold_id,
                "status" => "SETTLED_AND_CLEARED",
            })
        },
    );

    // -------------------------------------------------------------
    // Load Blueprint & Mount Routes
    // -------------------------------------------------------------
    engine
        .load_blueprint_file("enterprise_settlement_workflow.yaml")
        .expect("Failed to load enterprise_settlement_workflow.yaml");

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim_start_matches(':').parse::<u16>().ok())
        .unwrap_or(8095);

    let router = engine.get_router();

    println!("\n==================================================================");
    println!(
        "🚀 OCTOPUS Live Engine - Rust Enterprise Settlement API on http://localhost:{}",
        port
    );
    println!("📖 Swagger Docs:    http://localhost:{}/docs", port);
    println!(
        "🎨 Live Visualizer: http://localhost:{}/visualizer?domain=enterprise_settlement_gateway",
        port
    );
    println!("==================================================================");

    router
        .run(&format!("127.0.0.1:{}", port))
        .expect("Server failed");
}
