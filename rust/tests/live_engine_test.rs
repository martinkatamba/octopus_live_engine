use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use octopus_live_engine::adapters::outbound::{
    MySqlDialect, PostgresDialect, RelationalStateStore, SqlDialect, SqlServerDialect,
    SqliteDialect, StoreFactory,
};
use octopus_live_engine::domain::models::{
    APIEndpointConfig, EngineOptions, ExecutionState, ExecutionStatus, MaskingConfig,
    StepExecutionTrace, ToolingAccess, WorkflowBlueprint, WorkflowStep,
};
use octopus_live_engine::domain::ports::{IStateStore, IVisualizerCompiler};
use octopus_live_engine::domain::services::mask_payload;
use octopus_live_engine::{vmap, HttpRequest, OctopusLiveEngine, Value, ValueMap};

#[test]
fn test_rust_live_engine_sync_execution() {
    let mut engine = OctopusLiveEngine::new("");

    engine.register_activity(
        "ValidateOrder",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let order_id = input.get("order_id").cloned().unwrap_or(Value::Null);
            Ok(vmap! {
                "is_valid" => true,
                "order_id" => order_id,
            })
        },
    );

    engine.register_activity(
        "CalculateTax",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let amount = input
                .get("amount")
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0);
            let tax = amount * 0.15;
            Ok(vmap! {
                "tax_amount" => tax,
                "total" => amount + tax,
            })
        },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "order_tax_pipeline".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        steps: vec![
            WorkflowStep {
                id: "validate".to_string(),
                activity: "ValidateOrder".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "calc_tax".to_string(),
                activity: "CalculateTax".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let state = engine
        .execute_sync(
            "order_tax_pipeline",
            vmap! {
                "order_id" => "ORD-999",
                "amount" => 200.0,
            },
        )
        .expect("Execution failed");

    assert_eq!(state.status, ExecutionStatus::Completed);
    assert_eq!(state.traces.len(), 2);
    assert_eq!(
        state.data.get("total").and_then(|v| v.as_f64()),
        Some(230.0)
    );

    let diagram = engine
        .visualizer_service
        .compile_x6_diagram("order_tax_pipeline", Some(&state.job_id), None, None)
        .expect("Visualizer compilation failed");

    let nodes = diagram
        .get("nodes")
        .and_then(|v| v.as_array())
        .expect("Expected nodes array");
    assert_eq!(nodes.len(), 4);
}

#[test]
fn test_rust_live_engine_loop_and_async() {
    let mut engine = OctopusLiveEngine::new("");

    let async_called = Arc::new(AtomicBool::new(false));
    engine.register_activity(
        "ProcessItem",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let price = input
                .get("item")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("price"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            Ok(vmap! {
                "processed_price" => price * 1.1,
            })
        },
    );

    let async_called_clone = Arc::clone(&async_called);
    engine.register_activity(
        "AsyncAuditLog",
        move |_input: &ValueMap| -> Result<ValueMap, String> {
            async_called_clone.store(true, Ordering::SeqCst);
            Ok(vmap! { "audited" => true })
        },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "batch_loop_pipeline".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        steps: vec![
            WorkflowStep {
                id: "step_loop".to_string(),
                activity: "ProcessItem".to_string(),
                r#loop: Some(vmap! {
                    "items_field" => "items",
                    "item_variable" => "item",
                }),
                ..Default::default()
            },
            WorkflowStep {
                id: "step_async_log".to_string(),
                activity: "AsyncAuditLog".to_string(),
                is_async: true,
                ..Default::default()
            },
            WorkflowStep {
                id: "step_settle".to_string(),
                settle: true,
                reason: "Batch settled successfully".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "step_unreachable".to_string(),
                activity: "ProcessItem".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let state = engine
        .execute_sync(
            "batch_loop_pipeline",
            vmap! {
                "items" => Value::Array(vec![
                    Value::Object(vmap! { "name" => "Item A", "price" => 10.0 }),
                    Value::Object(vmap! { "name" => "Item B", "price" => 20.0 }),
                ]),
            },
        )
        .expect("Execution failed");

    assert_eq!(state.status, ExecutionStatus::Completed);
    let results = state
        .data
        .get("step_loop_results")
        .and_then(|v| v.as_array())
        .expect("Expected loop results");
    assert_eq!(results.len(), 2);

    std::thread::sleep(Duration::from_millis(50));
    assert!(async_called.load(Ordering::SeqCst));
}

#[test]
fn test_rust_live_engine_saga_compensation() {
    let mut engine = OctopusLiveEngine::new("");

    let compensated = Arc::new(AtomicBool::new(false));
    engine.register_activity(
        "ReserveFunds",
        |_input: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "hold_id" => "HOLD-999" }) },
    );

    let comp_clone = Arc::clone(&compensated);
    engine.register_activity(
        "ReleaseFunds",
        move |_input: &ValueMap| -> Result<ValueMap, String> {
            comp_clone.store(true, Ordering::SeqCst);
            Ok(vmap! { "released" => true })
        },
    );

    engine.register_activity(
        "FailingActivity",
        |_input: &ValueMap| -> Result<ValueMap, String> {
            Err("upstream service unavailable".to_string())
        },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "saga_flow".to_string(),
        compensation_mode: "sync".to_string(),
        steps: vec![
            WorkflowStep {
                id: "reserve".to_string(),
                activity: "ReserveFunds".to_string(),
                compensation: "ReleaseFunds".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "failing_step".to_string(),
                activity: "FailingActivity".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let res = engine.execute_sync("saga_flow", vmap! { "amount" => 500.0 });
    let Err((Some(state), _err)) = res else {
        panic!("Expected error with rolled back state");
    };
    assert_eq!(state.status, ExecutionStatus::RolledBack);
    assert!(compensated.load(Ordering::SeqCst));
}

#[test]
fn test_rust_declarative_multi_route_api() {
    let mut engine = OctopusLiveEngine::new("");
    engine.set_options(EngineOptions {
        allow_client_job_ids: true,
        tooling: ToolingAccess::Open,
        ..Default::default()
    });

    engine.register_activity(
        "CalculateSubtotal",
        |_input: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "subtotal" => 100.0 }) },
    );

    engine.register_activity(
        "CalculateTax",
        |input: &ValueMap| -> Result<ValueMap, String> {
            let amount = input
                .get("amount")
                .and_then(|v| v.as_f64())
                .or_else(|| input.get("subtotal").and_then(|v| v.as_f64()))
                .unwrap_or(100.0);
            let tax = amount * 0.15;
            Ok(vmap! {
                "tax_amount" => tax,
                "total" => amount + tax,
            })
        },
    );

    engine.register_activity(
        "PackageTaxResponse",
        |input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "tax_total" => input.get("tax_amount").cloned().unwrap_or(Value::Null),
                "total" => input.get("total").cloned().unwrap_or(Value::Null),
            })
        },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "ecommerce_tax_service".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        steps: vec![
            WorkflowStep {
                id: "def_subtotal".to_string(),
                activity: "CalculateSubtotal".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "def_tax".to_string(),
                activity: "CalculateTax".to_string(),
                ..Default::default()
            },
        ],
        api: vec![
            APIEndpointConfig {
                route: "/api/v1/checkout/tax".to_string(),
                method: "POST".to_string(),
                input_model: "CheckoutTaxRequest".to_string(),
                output_model: "CheckoutTaxResponse".to_string(),
                status_code: 200,
                summary: "Calculate tax for cart".to_string(),
                description: String::new(),
                tags: vec!["Checkout".to_string()],
                steps: vec![
                    WorkflowStep {
                        id: "ep_subtotal".to_string(),
                        activity: "CalculateSubtotal".to_string(),
                        ..Default::default()
                    },
                    WorkflowStep {
                        id: "ep_tax".to_string(),
                        activity: "CalculateTax".to_string(),
                        ..Default::default()
                    },
                    WorkflowStep {
                        id: "ep_package".to_string(),
                        activity: "PackageTaxResponse".to_string(),
                        is_output: true,
                        ..Default::default()
                    },
                ],
            },
            APIEndpointConfig {
                route: "/api/v1/checkout/quick-tax".to_string(),
                method: "GET".to_string(),
                input_model: "QuickTaxRequest".to_string(),
                output_model: "CheckoutTaxResponse".to_string(),
                status_code: 200,
                summary: "Quick tax quote".to_string(),
                description: String::new(),
                tags: vec!["Checkout".to_string()],
                steps: vec![WorkflowStep {
                    id: "quick_tax".to_string(),
                    activity: "CalculateTax".to_string(),
                    is_output: true,
                    ..Default::default()
                }],
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let router = engine.get_router();

    let w = router.serve_http(
        HttpRequest::new("POST", "/api/v1/checkout/tax")
            .with_header("Content-Type", "application/json")
            .with_header("X-Job-ID", "rust-custom-tx-999")
            .with_body(r#"{"cart_id":"CART-123"}"#),
    );
    assert_eq!(w.code, 200, "body: {}", w.body_string());
    assert_eq!(w.header("X-Job-ID"), "rust-custom-tx-999");
    assert_eq!(w.header("X-Execution-Status"), "COMPLETED");

    let resp_json = w.json().unwrap();
    assert_eq!(resp_json.get("total").and_then(|v| v.as_f64()), Some(115.0));
    assert_eq!(
        resp_json.get("tax_total").and_then(|v| v.as_f64()),
        Some(15.0)
    );

    let get_w = router.serve_http(HttpRequest::new(
        "GET",
        "/api/v1/checkout/quick-tax?amount=200.0",
    ));
    assert_eq!(get_w.code, 200);
    assert_eq!(get_w.header("X-Execution-Status"), "COMPLETED");
    let get_json = get_w.json().unwrap();
    assert_eq!(get_json.get("total").and_then(|v| v.as_f64()), Some(230.0));

    let openapi_w = router.serve_http(HttpRequest::new("GET", "/openapi.json"));
    assert_eq!(openapi_w.code, 200);
}

#[cfg(feature = "temporal")]
#[test]
fn test_rust_temporal_async_activity_dispatcher() {
    let mut engine = OctopusLiveEngine::new("");

    engine.register_activity(
        "AsyncAuditLog",
        |_input: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "logged" => true }) },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "rust_durable_async_wf".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        persistence: Some(vmap! {
            "temporal" => Value::Object(vmap! {
                "host" => "localhost:7233",
                "namespace" => "default",
                "task_queue" => "rust-test-queue",
            }),
        }),
        steps: vec![
            WorkflowStep {
                id: "step_sync".to_string(),
                activity: "AsyncAuditLog".to_string(),
                r#async: false,
                ..Default::default()
            },
            WorkflowStep {
                id: "step_async".to_string(),
                activity: "AsyncAuditLog".to_string(),
                r#async: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    assert_eq!(engine.async_dispatcher.get_dispatcher_type(), "TEMPORAL");

    let state = engine
        .execute_sync(
            "rust_durable_async_wf",
            vmap! { "job_id" => "rust-durable-job-001" },
        )
        .expect("Workflow failed");

    assert_eq!(state.status, ExecutionStatus::Completed);
    assert_eq!(state.traces.len(), 2);
    let async_trace = &state.traces[1];
    assert_eq!(async_trace.status, "ASYNC_DISPATCHED");
    assert!(async_trace.is_async);
    assert_eq!(
        async_trace
            .output_payload
            .get("dispatcher")
            .and_then(|v| v.as_str()),
        Some("TEMPORAL")
    );
}

#[test]
fn test_rust_jobs_history_endpoint() {
    let mut engine = OctopusLiveEngine::new("");
    engine.set_options(EngineOptions {
        allow_client_job_ids: true,
        tooling: ToolingAccess::Open,
        ..Default::default()
    });

    engine.register_activity(
        "ValidateOrder",
        |_input: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "status" => "valid" }) },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "order_tax_pipeline".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        steps: vec![WorkflowStep {
            id: "validate".to_string(),
            activity: "ValidateOrder".to_string(),
            ..Default::default()
        }],
        api: vec![APIEndpointConfig {
            route: "/orders/create".to_string(),
            method: "POST".to_string(),
            steps: vec![WorkflowStep {
                id: "validate".to_string(),
                activity: "ValidateOrder".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let router = engine.get_router();

    let w = router.serve_http(
        HttpRequest::new("POST", "/api/v1.0.0/order_tax_pipeline/orders/create")
            .with_header("Content-Type", "application/json")
            .with_header("X-Job-ID", "job-rust-test-888")
            .with_body(r#"{"item":"laptop"}"#),
    );
    assert_eq!(w.code, 200);

    let w_jobs = router.serve_http(HttpRequest::new("GET", "/api/visualizer/jobs"));
    assert_eq!(w_jobs.code, 200);
    let jobs_val = w_jobs.json().unwrap();
    let jobs_list = jobs_val.as_array().expect("Expected array of jobs");
    assert!(!jobs_list.is_empty());

    let found = jobs_list.iter().any(|j| {
        if j.get("job_id").and_then(|v| v.as_str()) == Some("job-rust-test-888") {
            assert_eq!(
                j.get("workflow_name").and_then(|v| v.as_str()),
                Some("order_tax_pipeline")
            );
            assert_eq!(j.get("status").and_then(|v| v.as_str()), Some("COMPLETED"));
            assert_eq!(j.get("method").and_then(|v| v.as_str()), Some("POST"));
            true
        } else {
            false
        }
    });
    assert!(found, "job-rust-test-888 not found in /api/visualizer/jobs");

    let w_api_jobs = router.serve_http(HttpRequest::new("GET", "/api/jobs"));
    assert_eq!(w_api_jobs.code, 200);

    let w_wf_jobs = router.serve_http(HttpRequest::new(
        "GET",
        "/api/v1.0.0/order_tax_pipeline/jobs",
    ));
    assert_eq!(w_wf_jobs.code, 200);

    let w_vis = router.serve_http(HttpRequest::new("GET", "/visualizer"));
    assert_eq!(w_vis.code, 200);
    let vis_html = w_vis.body_string();
    assert!(vis_html.contains("jobsModalBackdrop"));
    assert!(vis_html.contains("Execution Jobs History"));
    assert!(vis_html.contains("openJobsModal()"));
}

#[test]
fn test_sensitive_field_masking() {
    let raw_payload = vmap! {
        "username" => "sarah_vip",
        "password" => "supersecretpassword",
        "API_KEY" => "sk_live_99999",
        "meta" => Value::Object(vmap! {
            "client_ip" => "127.0.0.1",
            "Secret" => "nested_secret_val",
        }),
        "tokens" => Value::Array(vec![
            Value::Object(vmap! {
                "token_type" => "Bearer",
                "TOKEN" => "jwt.token.abc",
            }),
        ]),
    };

    let mask_fields = vec![
        "password".to_string(),
        "api_key".to_string(),
        "secret".to_string(),
        "token".to_string(),
    ];
    let masked = mask_payload(&raw_payload, &mask_fields, "********");

    assert_eq!(
        masked.get("username").and_then(|v| v.as_str()),
        Some("sarah_vip")
    );
    assert_eq!(
        masked.get("password").and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(
        masked.get("API_KEY").and_then(|v| v.as_str()),
        Some("********")
    );

    let nested_meta = masked.get("meta").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        nested_meta.get("Secret").and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(
        nested_meta.get("client_ip").and_then(|v| v.as_str()),
        Some("127.0.0.1")
    );

    let tokens_list = masked.get("tokens").and_then(|v| v.as_array()).unwrap();
    let token_item = tokens_list[0].as_object().unwrap();
    assert_eq!(
        token_item.get("TOKEN").and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(
        token_item.get("token_type").and_then(|v| v.as_str()),
        Some("Bearer")
    );

    let mut engine = OctopusLiveEngine::new("");
    let received_password = Arc::new(Mutex::new(String::new()));
    let recv_pw_clone = Arc::clone(&received_password);
    engine.register_activity(
        "LoginAuthActivity",
        move |input: &ValueMap| -> Result<ValueMap, String> {
            let pw = input
                .get("password")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            *recv_pw_clone.lock().unwrap() = pw.clone();
            if pw != "secret123" {
                return Err("invalid credentials".to_string());
            }
            Ok(vmap! {
                "access_token" => "token_abc123xyz",
                "customer_id" => "cust_001",
            })
        },
    );

    engine.register_activity(
        "StepWithCustomMask",
        |_input: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "processed" => true }) },
    );

    let bp = WorkflowBlueprint {
        workflow_name: "masked_auth_pipeline".to_string(),
        version: "1.0.0".to_string(),
        mode: "sync".to_string(),
        masking: Some(MaskingConfig {
            enabled: true,
            fields: vec!["password".to_string(), "access_token".to_string()],
            mask_value: "********".to_string(),
        }),
        steps: vec![
            WorkflowStep {
                id: "step_login".to_string(),
                activity: "LoginAuthActivity".to_string(),
                ..Default::default()
            },
            WorkflowStep {
                id: "step_custom_pin".to_string(),
                activity: "StepWithCustomMask".to_string(),
                mask_fields: vec!["pin_code".to_string()],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    engine.register_blueprint(bp).unwrap();

    let state = engine
        .execute_sync(
            "masked_auth_pipeline",
            vmap! {
                "username" => "sarah_vip",
                "password" => "secret123",
                "pin_code" => "4321",
            },
        )
        .expect("Workflow execution failed");

    assert_eq!(*received_password.lock().unwrap(), "secret123");
    assert_eq!(
        state.traces[0]
            .input_payload
            .get("password")
            .and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(
        state.traces[0]
            .output_payload
            .get("access_token")
            .and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(
        state.traces[1]
            .input_payload
            .get("pin_code")
            .and_then(|v| v.as_str()),
        Some("********")
    );

    let diagram = engine
        .visualizer_service
        .compile_x6_diagram("masked_auth_pipeline", Some(&state.job_id), None, None)
        .unwrap();
    let nodes = diagram.get("nodes").and_then(|v| v.as_array()).unwrap();
    let login_node = nodes
        .iter()
        .find_map(|n| {
            let data = n.get("data")?.as_object()?;
            if data.get("step_id")?.as_str() == Some("step_login") {
                Some(data)
            } else {
                None
            }
        })
        .expect("step_login node not found");
    let node_input = login_node
        .get("input_payload")
        .and_then(|v| v.as_object())
        .unwrap();
    assert_eq!(
        node_input.get("password").and_then(|v| v.as_str()),
        Some("********")
    );
}

#[test]
fn test_relational_state_store_and_dialects() {
    let pg = PostgresDialect;
    assert_eq!(pg.name(), "postgres");
    assert_eq!(pg.placeholder(1), "$1");
    assert!(pg
        .upsert_execution_state_sql()
        .contains("ON CONFLICT (job_id) DO UPDATE"));

    let my = MySqlDialect;
    assert_eq!(my.name(), "mysql");
    assert_eq!(my.placeholder(1), "?");
    assert!(my
        .upsert_execution_state_sql()
        .contains("ON DUPLICATE KEY UPDATE"));

    let ms = SqlServerDialect;
    assert_eq!(ms.name(), "sqlserver");
    assert!(ms
        .upsert_execution_state_sql()
        .contains("MERGE INTO execution_states"));

    let sq = SqliteDialect;
    assert_eq!(sq.name(), "sqlite");

    let factory = StoreFactory::new();
    let _s_mem = factory.create("inmemory", None);
    let _s_pg = factory.create(
        "postgres",
        Some(&vmap! { "dsn" => "postgres://localhost:5432/db" }),
    );
    let _s_my = factory.create(
        "mysql",
        Some(&vmap! { "dsn" => "mysql://localhost:3306/db" }),
    );
    let _s_ms = factory.create(
        "sqlserver",
        Some(&vmap! { "dsn" => "sqlserver://localhost:1433" }),
    );

    let rel_store = RelationalStateStore::new(Some(Box::new(PostgresDialect)));
    let state = ExecutionState {
        job_id: "rust-relational-job-101".to_string(),
        workflow_name: "order_tax_pipeline".to_string(),
        status: ExecutionStatus::Completed,
        route: "/api/v1/tax".to_string(),
        method: "POST".to_string(),
        started_at: "2026-09-23T18:00:00Z".to_string(),
        data: vmap! { "subtotal" => 100.0 },
        output_data: Some(vmap! { "tax" => 8.25, "total" => 108.25 }),
        traces: vec![StepExecutionTrace {
            step_id: "step_tax".to_string(),
            activity_name: "CalculateSalesTax".to_string(),
            status: "COMPLETED".to_string(),
            is_async: false,
            input_payload: vmap! { "subtotal" => 100.0 },
            output_payload: vmap! { "tax" => 8.25 },
            error_message: String::new(),
            attempts: 1,
            started_at: String::new(),
            finished_at: String::new(),
            duration_ms: 0.85,
        }],
        ..Default::default()
    };

    rel_store.save_state(state).unwrap();
    let fetched = rel_store.get_state("rust-relational-job-101").unwrap();
    assert_eq!(fetched.job_id, "rust-relational-job-101");
    assert_eq!(fetched.workflow_name, "order_tax_pipeline");
    assert_eq!(fetched.traces.len(), 1);
    assert_eq!(fetched.traces[0].activity_name, "CalculateSalesTax");

    let listed = rel_store.list_states("order_tax_pipeline", 10).unwrap();
    assert_eq!(listed.len(), 1);
}
