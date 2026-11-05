use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use octopus_live_engine::domain::models::{MaskingConfig, WorkflowBlueprint, WorkflowStep};
use octopus_live_engine::domain::services::mask_payload;
use octopus_live_engine::{vmap, HttpRequest, OctopusLiveEngine, Value, ValueMap};

#[test]
fn test_stored_state_is_isolated_from_running_execution() {
    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Slow", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        let mut out = ValueMap::new();
        for i in 0..50 {
            out.insert(
                format!("k{}", (b'a' + (i % 26) as u8) as char),
                Value::Int(i),
            );
            std::thread::sleep(Duration::from_micros(200));
        }
        Ok(out)
    });
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "race".to_string(),
            version: "1.0.0".to_string(),
            steps: vec![
                WorkflowStep {
                    id: "a".to_string(),
                    activity: "Slow".to_string(),
                    ..Default::default()
                },
                WorkflowStep {
                    id: "b".to_string(),
                    activity: "Slow".to_string(),
                    ..Default::default()
                },
                WorkflowStep {
                    id: "c".to_string(),
                    activity: "Slow".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    let engine = Arc::new(engine);
    let stop = Arc::new(AtomicBool::new(false));

    let mut reader_handles = Vec::new();
    for _ in 0..4 {
        let eng = Arc::clone(&engine);
        let stp = Arc::clone(&stop);
        reader_handles.push(std::thread::spawn(move || {
            while !stp.load(Ordering::Relaxed) {
                if let Ok(states) = eng.list_execution_states("race", 10) {
                    for st in states {
                        let _ = st.to_value().to_json_string();
                    }
                }
            }
        }));
    }

    let mut run_handles = Vec::new();
    for _ in 0..4 {
        let eng = Arc::clone(&engine);
        run_handles.push(std::thread::spawn(move || {
            eng.execute_sync("race", vmap! { "n" => 1i64 }).unwrap();
        }));
    }

    for h in run_handles {
        h.join().unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for h in reader_handles {
        h.join().unwrap();
    }
}

#[test]
fn test_stored_state_masks_nested_fields() {
    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Load", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! {
            "account" => Value::Object(vmap! {
                "Owner" => "grace",
                "PIN" => "2468",
            }),
        })
    });
    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "masking".to_string(),
            version: "1.0.0".to_string(),
            masking: Some(MaskingConfig {
                enabled: true,
                fields: vec!["pin".to_string(), "phone".to_string()],
                mask_value: String::new(),
            }),
            steps: vec![WorkflowStep {
                id: "load".to_string(),
                activity: "Load".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();

    let st = engine
        .execute_sync("masking", vmap! { "phone" => "0772123456" })
        .unwrap();
    let stored = engine.get_execution_state(&st.job_id).unwrap();
    let json_str = stored.to_value().to_json_string();
    for secret in ["2468", "0772123456"] {
        assert!(
            !json_str.contains(secret),
            "stored state leaks {}: {}",
            secret,
            json_str
        );
    }
    assert!(json_str.contains("grace"));
}

#[test]
fn test_masking_covers_injected_copies() {
    let out = mask_payload(
        &vmap! {
            "pin" => "1234",
            "_injected_pin" => "1234",
            "_INJECTED_Api_Key" => "k",
            "nested" => Value::Object(vmap! {
                "_injected_pin" => "1234",
            }),
            "name" => "ok",
        },
        &["pin".to_string(), "api_key".to_string()],
        "",
    );
    for k in ["pin", "_injected_pin", "_INJECTED_Api_Key"] {
        assert_eq!(out.get(k).and_then(|v| v.as_str()), Some("********"));
    }
    let nested = out.get("nested").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        nested.get("_injected_pin").and_then(|v| v.as_str()),
        Some("********")
    );
    assert_eq!(out.get("name").and_then(|v| v.as_str()), Some("ok"));
}

// D5: a declared route that issues a masked field (a login returning
// access_token) responds with the real value; the job itself stays masked.
#[test]
fn test_declared_route_returns_masked_fields_to_the_caller() {
    let dir = std::env::temp_dir().join(format!("octopus-d5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tokens.yaml");
    std::fs::write(
        &path,
        "workflow_name: tokens\nversion: \"1.0.0\"\nmasking:\n  fields: [access_token]\n\
         api:\n  - route: /api/v1.0.0/tokens/login\n    method: POST\n    steps:\n      \
         - id: issue\n        activity: Issue\n        is_output: true\n",
    )
    .unwrap();
    let mut engine = OctopusLiveEngine::new("");
    engine.register_activity("Issue", |_in: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! { "access_token" => "tok-123" })
    });
    engine.load_blueprint_file(path.to_str().unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let w = engine.get_router().serve_http(
        HttpRequest::new("POST", "/api/v1.0.0/tokens/login")
            .with_header("Content-Type", "application/json")
            .with_body("{}"),
    );
    assert_eq!(w.code, 200, "body: {}", w.body_string());
    let body = w.json().unwrap();
    assert_eq!(
        body.get("access_token").and_then(|v| v.as_str()),
        Some("tok-123")
    );
    let stored = engine.get_execution_state(&w.header("X-Job-ID")).unwrap();
    let raw = stored.to_value().to_json_string();
    assert!(
        !raw.contains("tok-123"),
        "stored job contains the token: {}",
        raw
    );
}
