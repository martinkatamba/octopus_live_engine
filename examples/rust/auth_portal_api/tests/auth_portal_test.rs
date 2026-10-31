use auth_portal_api::{build_engine, to_f64};
use octopus_live_engine::domain::ports::{IActivityExecutor, IVisualizerCompiler};
use octopus_live_engine::{vmap, Value};

#[test]
fn test_visualizer_diagram_compilation_and_routes() {
    let mut engine = build_engine();
    engine
        .load_blueprint_file("auth_portal_workflow.yaml")
        .expect("Failed to load blueprint");

    let diag = engine
        .visualizer_service
        .compile_x6_diagram("customer_auth_portal", None, None, None)
        .expect("Failed to compile X6 diagram");

    let nodes = match diag.get("nodes") {
        Some(Value::Array(arr)) => arr,
        _ => panic!("Expected nodes array in diagram"),
    };

    assert!(!nodes.is_empty(), "Expected nodes in diagram");

    let routes = match diag.get("routes") {
        Some(Value::Array(arr)) => arr,
        _ => panic!("Expected routes array in diagram"),
    };

    assert_eq!(routes.len(), 3, "Expected 3 declared API routes");

    let route_strs: Vec<String> = routes
        .iter()
        .filter_map(|r| match r {
            Value::String(s) => Some(s.clone()),
            Value::Object(m) => m.get("route").and_then(|v: &Value| v.as_str()).map(|s| s.to_string()),
            _ => None,
        })
        .collect();

    assert!(
        route_strs.iter().any(|r| r.contains("auth/login")),
        "Expected login route in diagram routes"
    );
    assert!(
        route_strs.iter().any(|r| r.contains("profile")),
        "Expected profile route in diagram routes"
    );
    assert!(
        route_strs.iter().any(|r| r.contains("cart/checkout")),
        "Expected checkout route in diagram routes"
    );
}

#[test]
fn test_auth_and_discount_activities() {
    let engine = build_engine();

    // 1. Verify User Credentials (Success)
    let login_input = vmap! {
        "username" => "sarah_vip",
        "password" => "password123",
    };
    let login_res = engine
        .activity_executor
        .execute_activity("VerifyUserCredentials", &login_input)
        .expect("VerifyUserCredentials should succeed");

    assert_eq!(
        login_res.get("authenticated").and_then(|v: &Value| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        login_res.get("customer_tier").and_then(|v: &Value| v.as_str()),
        Some("VIP")
    );

    // 2. Verify User Credentials (Failure)
    let bad_input = vmap! {
        "username" => "sarah_vip",
        "password" => "wrong_password",
    };
    let bad_res = engine
        .activity_executor
        .execute_activity("VerifyUserCredentials", &bad_input);
    assert!(bad_res.is_err(), "Bad credentials must return error");
    assert!(
        bad_res.unwrap_err().contains("InvalidCredentials"),
        "Error message must specify InvalidCredentials"
    );

    // 3. Issue Token
    let token_res = engine
        .activity_executor
        .execute_activity("IssueBearerToken", &login_res)
        .expect("IssueBearerToken should succeed");
    let token = token_res.get("access_token").and_then(|v: &Value| v.as_str()).unwrap();
    assert!(!token.is_empty());
    assert_eq!(
        token_res.get("token_type").and_then(|v: &Value| v.as_str()),
        Some("Bearer")
    );

    // 4. Fetch User Profile
    let prof_input = vmap! {
        "user_id" => "usr_9988_vip",
    };
    let prof_res = engine
        .activity_executor
        .execute_activity("FetchUserProfile", &prof_input)
        .expect("FetchUserProfile should succeed");
    assert_eq!(
        prof_res.get("username").and_then(|v: &Value| v.as_str()),
        Some("sarah_vip")
    );
    assert_eq!(
        prof_res.get("customer_tier").and_then(|v: &Value| v.as_str()),
        Some("VIP")
    );

    // 5. VIP Pricing Calculation (20% discount)
    let vip_pricing_input = vmap! {
        "subtotal" => 1000.0,
    };
    let vip_res = engine
        .activity_executor
        .execute_activity("ApplyVIPDiscount", &vip_pricing_input)
        .expect("ApplyVIPDiscount should succeed");

    let discount = to_f64(vip_res.get("discount_applied"), 0.0);
    let final_tot = to_f64(vip_res.get("final_total"), 0.0);

    assert_eq!(discount, 200.0);
    assert_eq!(final_tot, 800.0);

    // 6. Standard Pricing Calculation (0% discount)
    let std_pricing_input = vmap! {
        "subtotal" => 100.0,
    };
    let std_res = engine
        .activity_executor
        .execute_activity("ApplyStandardPricing", &std_pricing_input)
        .expect("ApplyStandardPricing should succeed");

    let std_disc = to_f64(std_res.get("discount_applied"), -1.0);
    let std_tot = to_f64(std_res.get("final_total"), 0.0);

    assert_eq!(std_disc, 0.0);
    assert_eq!(std_tot, 100.0);
}
