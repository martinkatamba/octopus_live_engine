use std::fs;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use octopus_live_engine::domain::models::{
    EngineOptions, ExecutionStatus, MiddlewareConfig, ToolingAccess, WorkflowBlueprint,
    WorkflowStep,
};
use octopus_live_engine::domain::services::new_validation_exception;
use octopus_live_engine::{
    vmap, HttpContext, HttpRequest, OctopusLiveEngine, Router, Value, ValueMap,
};

fn wait_until<F>(what: &str, timeout: Duration, cond: F)
where
    F: Fn() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {}", what);
}

fn trace_status(engine: &OctopusLiveEngine, job_id: &str, step_id: &str) -> String {
    let Ok(st) = engine.get_execution_state(job_id) else {
        return String::new();
    };
    for tr in st.traces {
        if tr.step_id == step_id {
            return tr.status;
        }
    }
    String::new()
}

#[test]
fn test_step_timeout_enforced() {
    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Hang", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        std::thread::sleep(Duration::from_secs(5));
        Ok(ValueMap::new())
    });
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "slow".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![WorkflowStep {
                id: "hang".to_string(),
                activity: "Hang".to_string(),
                timeout_seconds: 1,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();

    let start = Instant::now();
    let res = engine.execute_sync("slow", ValueMap::new());
    assert!(res.is_err());
    assert!(start.elapsed() <= Duration::from_secs(3));
    let Err((Some(st), _)) = res else {
        panic!("expected failed ExecutionState");
    };
    assert_eq!(st.error_code, Some(504));
}

#[test]
fn test_step_retry_policy() {
    let mut engine = OctopusLiveEngine::new("");
    let flaky = Arc::new(AtomicI32::new(0));
    let rejected = Arc::new(AtomicI32::new(0));

    let flaky_clone = Arc::clone(&flaky);
    engine.register_activity(
        "Flaky",
        move |_in_map: &ValueMap| -> Result<ValueMap, String> {
            if flaky_clone.fetch_add(1, Ordering::SeqCst) + 1 < 3 {
                return Err("temporary outage".to_string());
            }
            Ok(vmap! { "ok" => true })
        },
    );

    let rejected_clone = Arc::clone(&rejected);
    engine.register_activity(
        "Reject",
        move |_in_map: &ValueMap| -> Result<ValueMap, String> {
            rejected_clone.fetch_add(1, Ordering::SeqCst);
            Err(new_validation_exception("bad input").to_string())
        },
    );

    let retry = vmap! {
        "max_attempts" => 3i64,
        "initial_interval_ms" => 5i64,
    };
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "retry".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![WorkflowStep {
                id: "flaky".to_string(),
                activity: "Flaky".to_string(),
                retry_policy: Some(retry.clone()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "reject".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![WorkflowStep {
                id: "reject".to_string(),
                activity: "Reject".to_string(),
                retry_policy: Some(retry),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();

    let st = engine.execute_sync("retry", ValueMap::new()).unwrap();
    assert_eq!(st.traces[0].attempts, 3);

    let rej_res = engine.execute_sync("reject", ValueMap::new());
    assert!(rej_res.is_err());
    assert_eq!(rejected.load(Ordering::SeqCst), 1);

    let once = Arc::new(AtomicI32::new(0));
    let once_clone = Arc::clone(&once);
    engine.register_activity(
        "Once",
        move |_in_map: &ValueMap| -> Result<ValueMap, String> {
            once_clone.fetch_add(1, Ordering::SeqCst);
            Err("down".to_string())
        },
    );
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "once".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![WorkflowStep {
                id: "o".to_string(),
                activity: "Once".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let _ = engine.execute_sync("once", ValueMap::new());
    assert_eq!(once.load(Ordering::SeqCst), 1);
}

#[test]
fn test_async_outcome_recorded() {
    let mut engine = OctopusLiveEngine::new("");
    let calls = Arc::new(AtomicI32::new(0));
    engine.register_activity(
        "Background",
        |_in_map: &ValueMap| -> Result<ValueMap, String> { Ok(vmap! { "sent" => true }) },
    );

    let calls_clone = Arc::clone(&calls);
    engine.register_activity(
        "BackgroundFail",
        move |_in_map: &ValueMap| -> Result<ValueMap, String> {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Err("provider unreachable".to_string())
        },
    );

    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "bg".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![
                WorkflowStep {
                    id: "ok".to_string(),
                    activity: "Background".to_string(),
                    r#async: true,
                    ..Default::default()
                },
                WorkflowStep {
                    id: "fail".to_string(),
                    activity: "BackgroundFail".to_string(),
                    r#async: true,
                    retry_policy: Some(vmap! {
                        "max_attempts" => 2i64,
                        "initial_interval_ms" => 5i64,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    let st = engine.execute_sync("bg", ValueMap::new()).unwrap();
    wait_until("async outcomes", Duration::from_secs(3), || {
        trace_status(&engine, &st.job_id, "ok") == "ASYNC_COMPLETED"
            && trace_status(&engine, &st.job_id, "fail") == "ASYNC_FAILED"
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn test_version_prefix_consistent() {
    for v in ["2.1.0", "v2.1.0"] {
        let mut engine = OctopusLiveEngine::new("");
        engine.set_options(EngineOptions {
            tooling: ToolingAccess::Open,
            ..Default::default()
        });
        engine.register_activity("Noop", |_in_map: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! { "ok" => true })
        });
        engine
            .register_blueprint(WorkflowBlueprint {
                workflow_name: "ver".to_string(),
                version: v.to_string(),
                steps: vec![WorkflowStep {
                    id: "n".to_string(),
                    activity: "Noop".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .unwrap();
        let r = engine.get_router();
        let w = r.serve_http(HttpRequest::new("POST", "/api/v2.1.0/ver/execute").with_body("{}"));
        assert_eq!(w.code, 200);
        let job_id = w.header("X-Job-ID");
        let w_res = r.serve_http(HttpRequest::new(
            "GET",
            &format!("/api/visualizer/resolve-job?job_id={}", job_id),
        ));
        assert!(w_res
            .body_string()
            .contains(r#""prefix":"/api/v2.1.0/ver""#));
    }
}

#[test]
fn test_middleware_order_deterministic() {
    let mut first = String::new();
    for run in 0..15 {
        let order = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut engine = OctopusLiveEngine::new("");
        for name in ["A", "B", "C", "D"] {
            let n = name.to_string();
            let order_clone = Arc::clone(&order);
            engine.register_custom_middleware(&format!("MW{}", n), move |_cfg, _ex, _disp| {
                let n2 = n.clone();
                let ord = Arc::clone(&order_clone);
                Arc::new(move |c: &mut HttpContext| {
                    ord.lock().unwrap().push(n2.clone());
                    c.next();
                })
            });
        }
        for (i, pair) in [vec!["MWA", "MWB"], vec!["MWC"], vec!["MWD"]]
            .iter()
            .enumerate()
        {
            let mut bp = WorkflowBlueprint {
                workflow_name: format!("wf{}", (b'a' + i as u8) as char),
                version: "1.0.0".to_string(),
                ..Default::default()
            };
            for m in pair {
                bp.middlewares.push(MiddlewareConfig {
                    name: m.to_string(),
                    ..Default::default()
                });
            }
            engine.register_blueprint(bp).unwrap();
        }
        let mut r = Router::new();
        engine.mount_routes(&mut r);
        let _ = r.serve_http(HttpRequest::new("GET", "/health"));
        let got = order.lock().unwrap().join("");
        if run == 0 {
            first = got.clone();
        }
        assert_eq!(got, first);
        assert_eq!(got, "ABCD");
    }
}

#[test]
fn test_loop_errors_surface() {
    let mut engine = OctopusLiveEngine::new("");
    let undone = Arc::new(AtomicBool::new(false));
    engine.register_activity(
        "Reserve",
        |_in_map: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! {
                "items" => Value::Array(vec![
                    Value::String("a".to_string()),
                    Value::String("bad".to_string()),
                    Value::String("c".to_string()),
                ]),
            })
        },
    );
    let undone_clone = Arc::clone(&undone);
    engine.register_activity(
        "Release",
        move |_in_map: &ValueMap| -> Result<ValueMap, String> {
            undone_clone.store(true, Ordering::SeqCst);
            Ok(ValueMap::new())
        },
    );
    engine.register_activity("Ship", |in_map: &ValueMap| -> Result<ValueMap, String> {
        if in_map.get("item").and_then(|v| v.as_str()) == Some("bad") {
            return Err("cannot ship bad".to_string());
        }
        Ok(vmap! {
            "shipped" => in_map.get("item").cloned().unwrap_or(Value::Null),
        })
    });

    let loop_cfg = vmap! {
        "items_field" => "items",
        "item_variable" => "item",
    };
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "loop".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![
                WorkflowStep {
                    id: "reserve".to_string(),
                    activity: "Reserve".to_string(),
                    compensation: "Release".to_string(),
                    ..Default::default()
                },
                WorkflowStep {
                    id: "ship".to_string(),
                    activity: "Ship".to_string(),
                    r#loop: Some(loop_cfg),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    let res = engine.execute_sync("loop", ValueMap::new());
    let Err((Some(st), _)) = res else {
        panic!("expected loop failure");
    };
    assert_eq!(st.status, ExecutionStatus::RolledBack);
    assert!(undone.load(Ordering::SeqCst));

    let tolerant = vmap! {
        "items_field" => "items",
        "item_variable" => "item",
        "continue_on_error" => true,
        "max_iterations" => 10.0,
    };
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "loop2".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![
                WorkflowStep {
                    id: "reserve".to_string(),
                    activity: "Reserve".to_string(),
                    ..Default::default()
                },
                WorkflowStep {
                    id: "ship".to_string(),
                    activity: "Ship".to_string(),
                    r#loop: Some(tolerant),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    let st2 = engine.execute_sync("loop2", ValueMap::new()).unwrap();
    assert_eq!(st2.status, ExecutionStatus::Completed);
}

#[test]
fn test_invalid_blueprint_files_fail() {
    let dir = std::env::temp_dir().join(format!("octopus_test_invalid_bp_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("good.yaml"),
        "workflow_name: good\nversion: \"1.0.0\"\nsteps: []\n",
    )
    .unwrap();
    fs::write(
        dir.join("typo.yaml"),
        "workflow_name: typo\nsteps:\n  - id: a\n   activity: X\n",
    )
    .unwrap();
    let mut engine = OctopusLiveEngine::new("");
    let res = engine.load_blueprints_from_directory(dir.to_str().unwrap());
    let _ = fs::remove_dir_all(&dir);
    assert!(
        res.as_ref()
            .err()
            .map(|e| e.contains("typo.yaml"))
            .unwrap_or(false),
        "invalid YAML not reported: {:?}",
        res
    );
}
