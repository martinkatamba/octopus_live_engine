use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octopus_live_engine::domain::models::{
    APIEndpointConfig, ExecutionStatus, StepCondition, WorkflowBlueprint, WorkflowStep,
};
use octopus_live_engine::domain::ports::IBlueprintRepository;
use octopus_live_engine::domain::services::{ERR_NO_SUSPENSION, ERR_SIGNAL_MISMATCH};
use octopus_live_engine::{vmap, HttpRequest, OctopusLiveEngine, Router, Value, ValueMap};

#[allow(dead_code)]
struct PayFlow {
    engine: OctopusLiveEngine,
    router: Router,
    confirmed: Arc<AtomicI32>,
    expired: Arc<AtomicI32>,
    released: Arc<AtomicI32>,
    delivered: Arc<AtomicI32>,
    seen_order: Arc<Mutex<String>>,
}

impl PayFlow {
    fn new(timeout: i32, fail_after_resume: bool) -> Self {
        let mut engine = OctopusLiveEngine::new("");
        let confirmed = Arc::new(AtomicI32::new(0));
        let expired = Arc::new(AtomicI32::new(0));
        let released = Arc::new(AtomicI32::new(0));
        let delivered = Arc::new(AtomicI32::new(0));
        let seen_order = Arc::new(Mutex::new(String::new()));

        engine.register_activity(
            "Initiate",
            |_in_map: &ValueMap| -> Result<ValueMap, String> {
                Ok(vmap! {
                    "payment_ref" => "ref-1",
                    "order_id" => "order-42",
                })
            },
        );

        let rel_clone = Arc::clone(&released);
        engine.register_activity(
            "Release",
            move |_in_map: &ValueMap| -> Result<ValueMap, String> {
                rel_clone.fetch_add(1, Ordering::SeqCst);
                Ok(ValueMap::new())
            },
        );

        engine.register_activity("Respond", |in_map: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "message" => "check your phone",
                "payment_ref" => in_map.get("payment_ref").cloned().unwrap_or(Value::Null),
            })
        });

        let conf_clone = Arc::clone(&confirmed);
        let seen_clone = Arc::clone(&seen_order);
        engine.register_activity(
            "Confirm",
            move |in_map: &ValueMap| -> Result<ValueMap, String> {
                conf_clone.fetch_add(1, Ordering::SeqCst);
                if let Some(ord) = in_map.get("order_id").and_then(|v| v.as_str()) {
                    *seen_clone.lock().unwrap() = ord.to_string();
                }
                Ok(vmap! { "paid" => true })
            },
        );

        let exp_clone = Arc::clone(&expired);
        engine.register_activity(
            "Expire",
            move |_in_map: &ValueMap| -> Result<ValueMap, String> {
                exp_clone.fetch_add(1, Ordering::SeqCst);
                Ok(vmap! { "paid" => false })
            },
        );

        let deliv_clone = Arc::clone(&delivered);
        engine.register_activity(
            "Deliver",
            move |_in_map: &ValueMap| -> Result<ValueMap, String> {
                deliv_clone.fetch_add(1, Ordering::SeqCst);
                if fail_after_resume {
                    return Err("courier unavailable".to_string());
                }
                Ok(ValueMap::new())
            },
        );

        engine
            .register_blueprint(WorkflowBlueprint {
                workflow_name: "pay".to_string(),
                version: "1.0.0".to_string(),
                api: vec![APIEndpointConfig {
                    route: "/api/v1.0.0/pay/start".to_string(),
                    method: "POST".to_string(),
                    steps: vec![
                        WorkflowStep {
                            id: "initiate".to_string(),
                            activity: "Initiate".to_string(),
                            compensation: "Release".to_string(),
                            ..Default::default()
                        },
                        WorkflowStep {
                            id: "respond".to_string(),
                            activity: "Respond".to_string(),
                            is_output: true,
                            ..Default::default()
                        },
                        WorkflowStep {
                            id: "wait_for_provider".to_string(),
                            await_signal: "payment_result".to_string(),
                            timeout_seconds: timeout,
                            ..Default::default()
                        },
                        WorkflowStep {
                            id: "settle".to_string(),
                            condition: Some(StepCondition {
                                variable: "signal.data.status".to_string(),
                                operator: "==".to_string(),
                                value: Value::String("successful".to_string()),
                                ..Default::default()
                            }),
                            then_steps: vec![
                                WorkflowStep {
                                    id: "confirm".to_string(),
                                    activity: "Confirm".to_string(),
                                    ..Default::default()
                                },
                                WorkflowStep {
                                    id: "deliver".to_string(),
                                    activity: "Deliver".to_string(),
                                    ..Default::default()
                                },
                            ],
                            else_steps: vec![WorkflowStep {
                                id: "expire".to_string(),
                                activity: "Expire".to_string(),
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();

        let mut router = Router::new();
        engine.mount_routes(&mut router);

        Self {
            engine,
            router,
            confirmed,
            expired,
            released,
            delivered,
            seen_order,
        }
    }

    fn start(&self) -> String {
        let w = self.router.serve_http(
            HttpRequest::new("POST", "/api/v1.0.0/pay/start")
                .with_header("Content-Type", "application/json")
                .with_body("{}"),
        );
        assert_eq!(w.code, 200);
        assert_eq!(w.header("X-Execution-Status"), "AWAITING_SIGNAL");
        let out = w.json().unwrap();
        assert_eq!(
            out.get("message").and_then(|v| v.as_str()),
            Some("check your phone")
        );
        w.header("X-Job-ID")
    }
}

#[test]
fn test_await_signal_resumes_once() {
    let f = PayFlow::new(0, false);
    let job_id = f.start();
    let st = f.engine.get_execution_state(&job_id).unwrap();
    assert_eq!(st.status, ExecutionStatus::AwaitingSignal);

    let wrong = f
        .engine
        .signal(&job_id, "something_else", ValueMap::new(), ValueMap::new());
    assert!(matches!(wrong, Err((_, ref msg)) if msg == ERR_SIGNAL_MISMATCH));

    let st = f
        .engine
        .signal(
            &job_id,
            "payment_result",
            vmap! { "status" => "successful", "order_id" => "evil" },
            ValueMap::new(),
        )
        .unwrap();
    assert_eq!(st.status, ExecutionStatus::Completed);
    assert_eq!(f.confirmed.load(Ordering::SeqCst), 1);
    assert_eq!(*f.seen_order.lock().unwrap(), "order-42");

    let second = f.engine.signal(
        &job_id,
        "payment_result",
        vmap! { "status" => "successful" },
        ValueMap::new(),
    );
    assert!(matches!(second, Err((_, ref msg)) if msg == ERR_NO_SUSPENSION));
    assert_eq!(f.confirmed.load(Ordering::SeqCst), 1);

    let stored = f.engine.get_execution_state(&job_id).unwrap();
    let saw_signal = stored
        .traces
        .iter()
        .any(|tr| tr.status == "SIGNAL_RECEIVED");
    assert_eq!(stored.status, ExecutionStatus::Completed);
    assert!(saw_signal);
}

#[test]
fn test_await_signal_timeout() {
    let f = PayFlow::new(1, false);
    let job_id = f.start();
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Ok(st) = f.engine.get_execution_state(&job_id) {
            if st.status == ExecutionStatus::Completed {
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for timeout branch"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.expired.load(Ordering::SeqCst), 1);
    assert_eq!(f.confirmed.load(Ordering::SeqCst), 0);

    let late = f
        .engine
        .signal(&job_id, "payment_result", ValueMap::new(), ValueMap::new());
    assert!(matches!(late, Err((_, ref msg)) if msg == ERR_NO_SUSPENSION));
}

#[test]
fn test_compensation_survives_suspension() {
    let f = PayFlow::new(0, true);
    let job_id = f.start();
    let res = f.engine.signal(
        &job_id,
        "payment_result",
        vmap! { "status" => "successful" },
        ValueMap::new(),
    );
    let Err((Some(st), _)) = res else {
        panic!("expected rollback after resume");
    };
    assert_eq!(st.status, ExecutionStatus::RolledBack);
    assert_eq!(f.released.load(Ordering::SeqCst), 1);
}

#[test]
fn test_signal_http_route() {
    let f = PayFlow::new(0, false);
    let job_id = f.start();
    let path = format!("/api/v1.0.0/pay/signal/{}", job_id);

    let w_remote = f.router.serve_http(
        HttpRequest::new("POST", &path)
            .with_remote_addr("203.0.113.9:1")
            .with_header("Content-Type", "application/json")
            .with_body(r#"{"signal":"payment_result","status":"successful"}"#),
    );
    assert_eq!(w_remote.code, 403);

    let w_local = f.router.serve_http(
        HttpRequest::new("POST", &path)
            .with_remote_addr("127.0.0.1:1")
            .with_header("Content-Type", "application/json")
            .with_body(r#"{"signal":"payment_result","status":"successful"}"#),
    );
    assert_eq!(w_local.code, 200);
    assert_eq!(f.confirmed.load(Ordering::SeqCst), 1);

    let w_after = f.router.serve_http(
        HttpRequest::new("POST", &path)
            .with_remote_addr("127.0.0.1:1")
            .with_header("Content-Type", "application/json")
            .with_body("{}"),
    );
    assert_eq!(w_after.code, 404);
}

#[test]
fn test_unsupported_blueprint_keys_rejected() {
    let mut engine = OctopusLiveEngine::new("");
    assert!(engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "dyn".to_string(),
            steps: vec![WorkflowStep {
                id: "x".to_string(),
                dynamic_branching: Some(vmap! { "field" => "kind" }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .is_err());

    assert!(engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "roles".to_string(),
            allowed_initiator_roles: vec!["admin".to_string()],
            ..Default::default()
        })
        .is_err());

    assert!(engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "asyncwait".to_string(),
            steps: vec![WorkflowStep {
                id: "w".to_string(),
                await_signal: "x".to_string(),
                r#async: true,
                ..Default::default()
            }],
            ..Default::default()
        })
        .is_err());
}

#[test]
fn test_pivot_stops_compensation() {
    for pivot in [false, true] {
        let mut f = PayFlow::new(0, true);
        let mut bp =
            WorkflowBlueprint::clone(&f.engine.blueprint_repo.get_blueprint("pay").unwrap());
        bp.api[0].steps[1].pivot = pivot;
        f.engine.register_blueprint(bp).unwrap();
        let mut router = Router::new();
        f.engine.mount_routes(&mut router);
        f.router = router;

        let job_id = f.start();
        let res = f.engine.signal(
            &job_id,
            "payment_result",
            vmap! { "status" => "successful" },
            ValueMap::new(),
        );
        let Err((Some(st), _)) = res else {
            panic!("expected error");
        };
        let (want, released) = if pivot {
            (ExecutionStatus::Failed, 0)
        } else {
            (ExecutionStatus::RolledBack, 1)
        };
        assert_eq!(st.status, want);
        assert_eq!(f.released.load(Ordering::SeqCst), released);
    }
}
