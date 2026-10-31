use std::time::{SystemTime, UNIX_EPOCH};

use octopus_live_engine::crypto::{base64url_encode, hmac_sha256};
use octopus_live_engine::domain::models::{EngineOptions, ToolingAccess};
use octopus_live_engine::json;
use octopus_live_engine::{new_live_engine, vmap, Value, ValueMap};

pub const JWT_SECRET: &str = "octopus-live-engine-secret-key-2026";

// ============================================================================
// 1. Helper Utilities & JWT Cryptographic Signer
// ============================================================================

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn to_f64(v: Option<&Value>, fallback: f64) -> f64 {
    match v {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        _ => fallback,
    }
}

pub fn round_two_decimals(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

pub fn create_jwt_token(claims: &ValueMap, secret: &str, expires_in_seconds: u64) -> String {
    let header = vmap! {
        "alg" => "HS256",
        "typ" => "JWT",
    };

    let mut claims_copy = claims.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    claims_copy.insert("iat".to_string(), Value::Float(now));
    claims_copy.insert(
        "exp".to_string(),
        Value::Float(now + (expires_in_seconds as f64)),
    );

    let h_json = json::map_to_json(&header);
    let c_json = json::map_to_json(&claims_copy);
    let h_b64 = base64url_encode(h_json.as_bytes());
    let c_b64 = base64url_encode(c_json.as_bytes());
    let signing_input = format!("{}.{}", h_b64, c_b64);
    let sig = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    let s_b64 = base64url_encode(&sig);

    format!("{}.{}", signing_input, s_b64)
}

// ============================================================================
// 2. In-Memory Mock User Database
// ============================================================================

pub struct UserRecord {
    pub user_id: &'static str,
    pub username: &'static str,
    pub email: &'static str,
    pub password: &'static str,
    pub customer_tier: &'static str,
    pub roles: &'static [&'static str],
    pub account_status: &'static str,
    pub member_since: &'static str,
}

pub static USER_DATABASE: &[UserRecord] = &[
    UserRecord {
        user_id: "usr_9988_vip",
        username: "sarah_vip",
        email: "sarah@acme-corp.com",
        password: "password123",
        customer_tier: "VIP",
        roles: &["customer", "vip_member"],
        account_status: "ACTIVE_VIP",
        member_since: "2024-01-15",
    },
    UserRecord {
        user_id: "usr_1024_std",
        username: "john_doe",
        email: "john.doe@example.com",
        password: "password123",
        customer_tier: "STANDARD",
        roles: &["customer"],
        account_status: "ACTIVE_STANDARD",
        member_since: "2025-06-20",
    },
];

// ============================================================================
// 3. Application Setup & Activities
// ============================================================================

pub fn build_engine() -> octopus_live_engine::OctopusLiveEngine {
    let mut engine = new_live_engine("");

    // Enable client-supplied job IDs (X-Job-ID) and open tooling access for visualizer
    engine.set_options(EngineOptions {
        allow_client_job_ids: true,
        tooling: ToolingAccess::Open,
        ..Default::default()
    });

    // -------------------------------------------------------------
    // Register Models for OpenAPI 3.0 & Swagger UI
    // -------------------------------------------------------------
    engine.register_model(
        "LoginRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "username" => Value::Object(vmap! { "type" => "string", "description" => "User login username (e.g. sarah_vip or john_doe)" }),
                "password" => Value::Object(vmap! { "type" => "string", "description" => "User login password" }),
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
                "token_type" => Value::Object(vmap! { "type" => "string", "default" => "Bearer" }),
                "user_id" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "expires_in_seconds" => Value::Object(vmap! { "type" => "integer", "default" => 3600 }),
            }),
        }),
    );

    engine.register_model(
        "UserProfileResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "user_id" => Value::Object(vmap! { "type" => "string" }),
                "username" => Value::Object(vmap! { "type" => "string" }),
                "email" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "account_status" => Value::Object(vmap! { "type" => "string" }),
                "member_since" => Value::Object(vmap! { "type" => "string" }),
                "roles" => Value::Object(vmap! {
                    "type" => "array",
                    "items" => Value::Object(vmap! { "type" => "string" }),
                }),
            }),
        }),
    );

    engine.register_model(
        "CartItem",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "sku" => Value::Object(vmap! { "type" => "string" }),
                "description" => Value::Object(vmap! { "type" => "string" }),
                "quantity" => Value::Object(vmap! { "type" => "integer", "default" => 1 }),
                "unit_price" => Value::Object(vmap! { "type" => "number" }),
            }),
            "required" => Value::Array(vec![
                Value::String("sku".to_string()),
                Value::String("unit_price".to_string()),
            ]),
        }),
    );

    engine.register_model(
        "CheckoutRequest",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "cart_id" => Value::Object(vmap! { "type" => "string" }),
                "items" => Value::Object(vmap! {
                    "type" => "array",
                    "items" => Value::Object(vmap! { "$ref" => "#/components/schemas/CartItem" }),
                }),
            }),
            "required" => Value::Array(vec![
                Value::String("cart_id".to_string()),
            ]),
        }),
    );

    engine.register_model(
        "CheckoutResponse",
        Value::Object(vmap! {
            "type" => "object",
            "properties" => Value::Object(vmap! {
                "order_id" => Value::Object(vmap! { "type" => "string" }),
                "customer_id" => Value::Object(vmap! { "type" => "string" }),
                "customer_tier" => Value::Object(vmap! { "type" => "string" }),
                "subtotal" => Value::Object(vmap! { "type" => "number" }),
                "discount_applied" => Value::Object(vmap! { "type" => "number" }),
                "final_total" => Value::Object(vmap! { "type" => "number" }),
                "pricing_tier" => Value::Object(vmap! { "type" => "string" }),
                "shipping_method" => Value::Object(vmap! { "type" => "string" }),
                "fast_path_latency_ms" => Value::Object(vmap! { "type" => "number", "default" => 0.35 }),
            }),
        }),
    );

    // -------------------------------------------------------------
    // Register Domain Activities
    // -------------------------------------------------------------

    // Activity: VerifyUserCredentials
    engine.register_activity(
        "VerifyUserCredentials",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let username = input.get("username").and_then(|v| v.as_str()).unwrap_or("");
            let password = input.get("password").and_then(|v| v.as_str()).unwrap_or("");

            // Simulate unexpected runtime crash for crash_user
            if username == "crash_user" {
                return Err("division by zero during unexpected calculation crash".to_string());
            }

            let user = USER_DATABASE.iter().find(|u| u.username == username);
            match user {
                Some(u) if u.password == password => {
                    let roles = u.roles.iter().map(|r| Value::String(r.to_string())).collect();
                    Ok(vmap! {
                        "authenticated" => true,
                        "user_id" => u.user_id,
                        "verified_username" => u.username,
                        "email" => u.email,
                        "customer_tier" => u.customer_tier,
                        "roles" => Value::Array(roles),
                    })
                }
                _ => Err("InvalidCredentials: Invalid username or password. Please try again.".to_string()),
            }
        },
    );

    // Activity: IssueBearerToken
    engine.register_activity(
        "IssueBearerToken",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let user_id = input.get("user_id").and_then(|v| v.as_str()).unwrap_or("usr_anon");
            let tier = input.get("customer_tier").and_then(|v| v.as_str()).unwrap_or("STANDARD");
            let username = input.get("verified_username").and_then(|v| v.as_str()).unwrap_or("guest");
            let email = input.get("email").and_then(|v| v.as_str()).unwrap_or("guest@example.com");

            let roles = match input.get("roles") {
                Some(Value::Array(r)) => Value::Array(r.clone()),
                _ => Value::Array(vec![Value::String("customer".to_string())]),
            };

            let claims = vmap! {
                "sub" => user_id,
                "user_id" => user_id,
                "username" => username,
                "email" => email,
                "customer_tier" => tier,
                "roles" => roles,
            };

            let token = create_jwt_token(&claims, JWT_SECRET, 3600);

            Ok(vmap! {
                "access_token" => token,
                "token_type" => "Bearer",
                "user_id" => user_id,
                "customer_tier" => tier,
                "expires_in_seconds" => 3600i64,
            })
        },
    );

    // Activity: FetchUserProfile
    engine.register_activity(
        "FetchUserProfile",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let user_id = input.get("user_id").and_then(|v| v.as_str()).unwrap_or("usr_unknown");
            let user = USER_DATABASE.iter().find(|u| u.user_id == user_id);

            match user {
                Some(u) => {
                    let roles = u.roles.iter().map(|r| Value::String(r.to_string())).collect();
                    Ok(vmap! {
                        "user_id" => u.user_id,
                        "username" => u.username,
                        "email" => u.email,
                        "customer_tier" => u.customer_tier,
                        "account_status" => u.account_status,
                        "member_since" => u.member_since,
                        "roles" => Value::Array(roles),
                    })
                }
                None => {
                    let username = input.get("username").and_then(|v| v.as_str()).unwrap_or("guest");
                    let email = input.get("email").and_then(|v| v.as_str()).unwrap_or("guest@example.com");
                    let tier = input.get("customer_tier").and_then(|v| v.as_str()).unwrap_or("STANDARD");
                    let roles = match input.get("roles") {
                        Some(Value::Array(r)) => Value::Array(r.clone()),
                        _ => Value::Array(vec![Value::String("customer".to_string())]),
                    };

                    Ok(vmap! {
                        "user_id" => user_id,
                        "username" => username,
                        "email" => email,
                        "customer_tier" => tier,
                        "account_status" => "ACTIVE",
                        "member_since" => "2026-01-01",
                        "roles" => roles,
                    })
                }
            }
        },
    );

    // Activity: ValidateCartItems
    engine.register_activity(
        "ValidateCartItems",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let mut subtotal = 0.0;
            let mut items: Vec<Value> = Vec::new();

            if let Some(Value::Array(arr)) = input.get("items") {
                items = arr.clone();
                for it in arr {
                    if let Value::Object(obj) = it {
                        let qty = to_f64(obj.get("quantity"), 1.0);
                        let price = to_f64(obj.get("unit_price"), 0.0);
                        subtotal += qty * price;
                    }
                }
            }

            if subtotal == 0.0 {
                subtotal = 300.00;
                items = vec![Value::Object(vmap! {
                    "sku" => "HEADPHONES-PRO",
                    "description" => "Noise Cancelling",
                    "quantity" => 1i64,
                    "unit_price" => 300.00,
                })];
            }

            let cart_id = input.get("cart_id").and_then(|v| v.as_str()).unwrap_or("cart_default");

            Ok(vmap! {
                "subtotal" => round_two_decimals(subtotal),
                "validated_items" => Value::Array(items),
                "cart_id" => cart_id,
            })
        },
    );

    // Activity: ApplyVIPDiscount
    engine.register_activity(
        "ApplyVIPDiscount",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let subtotal = to_f64(input.get("subtotal"), 0.0);
            let discount = round_two_decimals(subtotal * 0.20);
            let final_total = round_two_decimals(subtotal - discount);

            Ok(vmap! {
                "discount_applied" => discount,
                "final_total" => final_total,
                "pricing_tier" => "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
                "shipping_method" => "FREE_PRIORITY_EXPRESS_AIR",
            })
        },
    );

    // Activity: ApplyStandardPricing
    engine.register_activity(
        "ApplyStandardPricing",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let subtotal = to_f64(input.get("subtotal"), 0.0);

            Ok(vmap! {
                "discount_applied" => 0.0,
                "final_total" => subtotal,
                "pricing_tier" => "STANDARD_RETAIL_PRICING",
                "shipping_method" => "STANDARD_GROUND_SHIPPING",
            })
        },
    );

    // Activity: FinalizeCheckoutOrder
    engine.register_activity(
        "FinalizeCheckoutOrder",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let subtotal = to_f64(input.get("subtotal"), 0.0);
            let discount = to_f64(input.get("discount_applied"), 0.0);
            let final_total = to_f64(input.get("final_total"), subtotal);

            let customer_id = input
                .get("customer_id")
                .or_else(|| input.get("user_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("usr_anon");

            let tier = input
                .get("customer_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("STANDARD");

            let pricing_tier = input
                .get("pricing_tier")
                .and_then(|v| v.as_str())
                .unwrap_or("STANDARD_RETAIL_PRICING");

            let shipping_method = input
                .get("shipping_method")
                .and_then(|v| v.as_str())
                .unwrap_or("STANDARD_GROUND_SHIPPING");

            Ok(vmap! {
                "order_id" => format!("ord_{}", now_millis()),
                "customer_id" => customer_id,
                "customer_tier" => tier,
                "subtotal" => subtotal,
                "discount_applied" => discount,
                "final_total" => final_total,
                "pricing_tier" => pricing_tier,
                "shipping_method" => shipping_method,
                "fast_path_latency_ms" => 0.35,
            })
        },
    );

    // Activity: LogSecurityAuditViolation
    engine.register_activity(
        "LogSecurityAuditViolation",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let exception = input.get("exception").and_then(|v| v.as_str()).unwrap_or("SecurityViolation");
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let method = input.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let client_host = input.get("client_host").and_then(|v| v.as_str()).unwrap_or("unknown");

            println!(
                "🚨 [SECURITY AUDIT] Unauthorized access attempt blocked: {} {} from {} ({})",
                method, path, client_host, exception
            );

            Ok(vmap! {
                "audit_logged" => true,
                "security_incident_id" => format!("inc_{}", now_millis()),
                "timestamp" => now_millis() as i64,
            })
        },
    );

    // Activity: RecordResponseAuditLog
    engine.register_activity(
        "RecordResponseAuditLog",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let route = input.get("route").and_then(|v| v.as_str()).unwrap_or("");
            let method = input.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let status_code = to_f64(input.get("status_code"), 200.0) as i64;
            let duration_ms = to_f64(input.get("duration_ms"), 0.0);

            println!(
                "📊 [AUDIT LOG] Recorded: {} {} -> Status {} in {:.2}ms",
                method, route, status_code, duration_ms
            );

            Ok(vmap! {
                "logged" => true,
                "timestamp" => now_millis() as i64,
            })
        },
    );

    // Activity: HandleGlobalUnhandledException
    engine.register_activity(
        "HandleGlobalUnhandledException",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let job_id = input.get("job_id").and_then(|v| v.as_str()).unwrap_or("unknown");
            let failing_step = input
                .get("failing_step_id")
                .or_else(|| input.get("step_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let err_type = input.get("error_type").and_then(|v| v.as_str()).unwrap_or("UnknownError");
            let err_msg = input
                .get("error_message")
                .or_else(|| input.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("Internal Error");

            eprintln!(
                "🚨 [GLOBAL EXCEPTION HANDLER] Incident logged for Job={} Step={}: {} - {}",
                job_id, failing_step, err_type, err_msg
            );

            Ok(vmap! {
                "incident_id" => format!("inc_{}", now_millis()),
                "handled" => true,
                "status" => "INCIDENT_LOGGED",
                "error_type" => err_type,
                "message" => "An unexpected internal server error occurred. Incident reference logged.",
                "timestamp" => now_millis() as i64,
            })
        },
    );

    engine
}
