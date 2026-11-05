//! Runs async steps through a real Temporal server. Needs `--features temporal`
//! and OCTOPUS_TEST_TEMPORAL_HOST (e.g. localhost:7233, from `temporal server start-dev`).
#![cfg(feature = "temporal")]

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use octopus_live_engine::domain::models::{WorkflowBlueprint, WorkflowStep};
use octopus_live_engine::{vmap, OctopusLiveEngine, Value, ValueMap};

fn trace_status(engine: &OctopusLiveEngine, job_id: &str, step_id: &str) -> String {
    engine
        .get_execution_state(job_id)
        .ok()
        .and_then(|st| st.traces.into_iter().find(|t| t.step_id == step_id))
        .map(|t| t.status)
        .unwrap_or_default()
}

#[test]
fn async_steps_run_on_temporal() {
    let Some(host) = std::env::var("OCTOPUS_TEST_TEMPORAL_HOST")
        .ok()
        .filter(|h| !h.is_empty())
    else {
        return;
    };
    let queue = format!(
        "octopus-rust-test-{}",
        octopus_live_engine::crypto::new_job_id()
    );

    let mut engine = OctopusLiveEngine::new("");
    let fail_calls = Arc::new(AtomicI32::new(0));
    engine.register_activity(
        "Background",
        |input: &ValueMap| -> Result<ValueMap, String> {
            Ok(vmap! { "echo" => input.get("value").cloned().unwrap_or(Value::Null) })
        },
    );
    let counter = Arc::clone(&fail_calls);
    engine.register_activity(
        "BackgroundFail",
        move |_: &ValueMap| -> Result<ValueMap, String> {
            counter.fetch_add(1, Ordering::SeqCst);
            Err("provider unreachable".to_string())
        },
    );

    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "bg_temporal".to_string(),
            version: "1.0.0".to_string(),
            persistence: Some(vmap! {
                "temporal" => Value::Object(vmap! { "host" => host.clone(), "task_queue" => queue.clone() }),
            }),
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
                    retry_policy: Some(vmap! { "max_attempts" => 3i64, "initial_interval_ms" => 50i64 }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(engine.async_dispatcher.get_dispatcher_type(), "TEMPORAL");
    engine
        .start_embedded_worker()
        .expect("worker connects to Temporal");

    let st = engine
        .execute_sync("bg_temporal", vmap! { "value" => "hello" })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline
        && !(trace_status(&engine, &st.job_id, "ok") == "ASYNC_COMPLETED"
            && trace_status(&engine, &st.job_id, "fail") == "ASYNC_FAILED")
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(trace_status(&engine, &st.job_id, "ok"), "ASYNC_COMPLETED");
    assert_eq!(trace_status(&engine, &st.job_id, "fail"), "ASYNC_FAILED");
    // Temporal applied the step's retry policy: three attempts, then failure.
    assert_eq!(fail_calls.load(Ordering::SeqCst), 3);

    let history = engine.async_dispatcher.get_dispatched_tasks_history();
    assert!(
        history
            .iter()
            .filter(|r| r.get("job_id").and_then(|v| v.as_str()) == Some(st.job_id.as_str()))
            .all(|r| r.get("status").and_then(|v| v.as_str()) == Some("DISPATCHED_TO_TEMPORAL")),
        "both steps went through Temporal, not the in-process fallback: {:?}",
        history
    );

    engine.stop_embedded_worker();
}
