//! Round-trips jobs through real databases. Each test runs only when its
//! connection variable is set, e.g.
//!   OCTOPUS_TEST_REDIS_URL=redis://localhost:6379
//!   OCTOPUS_TEST_POSTGRES_DSN=postgresql://postgres:postgrespassword@localhost:5432/octopus_db?sslmode=disable
//!   OCTOPUS_TEST_MYSQL_DSN=mysql://octopus_user:octopus_password@localhost:3306/octopus_db
//!   OCTOPUS_TEST_SQLSERVER_DSN="Server=localhost,1433;Database=master;User Id=sa;Password=StrongPassword123!;TrustServerCertificate=True"
//! SQLite always runs, against a temporary file.

use std::sync::Arc;
use std::time::Duration;

use octopus_live_engine::adapters::outbound::StoreFactory;
use octopus_live_engine::domain::models::{
    ExecutionState, ExecutionStatus, LiveEvent, StepExecutionTrace,
};
use octopus_live_engine::domain::ports::IStateStore;
use octopus_live_engine::{vmap, Value, ValueMap};

fn sample(job_id: &str, workflow: &str, started_at: &str) -> ExecutionState {
    ExecutionState {
        job_id: job_id.to_string(),
        workflow_name: workflow.to_string(),
        status: ExecutionStatus::Completed,
        route: "/api/v1/tax".to_string(),
        method: "POST".to_string(),
        current_step_id: "step_tax".to_string(),
        current_index: 1,
        total_steps: 2,
        data: vmap! { "subtotal" => 100.0, "customer" => "Zoë \"quoted\"" },
        output_data: Some(vmap! { "total" => 108.25 }),
        route_output: None,
        traces: vec![
            StepExecutionTrace {
                step_id: "step_tax".to_string(),
                activity_name: "CalculateSalesTax".to_string(),
                status: "COMPLETED".to_string(),
                is_async: true,
                input_payload: vmap! { "subtotal" => 100.0 },
                output_payload: vmap! { "tax" => 8.25 },
                attempts: 2,
                started_at: started_at.to_string(),
                finished_at: started_at.to_string(),
                duration_ms: 1.5,
                ..Default::default()
            },
            StepExecutionTrace {
                step_id: "step_notify".to_string(),
                activity_name: "Notify".to_string(),
                status: "FAILED".to_string(),
                error_message: "smtp down".to_string(),
                started_at: started_at.to_string(),
                finished_at: started_at.to_string(),
                ..Default::default()
            },
        ],
        started_at: started_at.to_string(),
        finished_at: Some(started_at.to_string()),
        client_host: "127.0.0.1".to_string(),
        duration_ms: 12.5,
        error: Some("partial".to_string()),
        error_code: Some(500),
        error_type: Some("ActivityError".to_string()),
    }
}

fn unique(prefix: &str) -> String {
    format!("{}-{}", prefix, octopus_live_engine::crypto::new_job_id())
}

/// Saves, updates, reads and lists jobs, and checks every field survives.
fn exercise(store: Arc<dyn IStateStore>) {
    let workflow = unique("wf");
    let a = sample(
        &unique("job-a"),
        &workflow,
        "2026-10-25T10:00:00.000000001Z",
    );
    let mut b = sample(
        &unique("job-b"),
        &workflow,
        "2026-10-25T11:00:00.000000001Z",
    );

    store.save_state(a.clone()).unwrap();
    store.save_state(b.clone()).unwrap();

    // Upsert: the second save of a job replaces it, traces included.
    b.status = ExecutionStatus::Failed;
    b.traces.truncate(1);
    store.save_state(b.clone()).unwrap();

    let got = store.get_state(&a.job_id).unwrap();
    assert_eq!(got.workflow_name, workflow);
    assert_eq!(got.status, ExecutionStatus::Completed);
    assert_eq!(got.current_index, 1);
    assert_eq!(got.total_steps, 2);
    assert_eq!(
        got.data.get("customer").and_then(|v| v.as_str()),
        Some("Zoë \"quoted\"")
    );
    assert_eq!(
        got.output_data
            .as_ref()
            .and_then(|o| o.get("total"))
            .and_then(|v| v.as_f64()),
        Some(108.25)
    );
    assert_eq!(got.error.as_deref(), Some("partial"));
    assert_eq!(got.error_code, Some(500));
    assert_eq!(got.error_type.as_deref(), Some("ActivityError"));
    assert_eq!(
        got.finished_at.as_deref(),
        Some("2026-10-25T10:00:00.000000001Z")
    );
    assert_eq!(got.duration_ms, 12.5);
    assert_eq!(got.traces.len(), 2);
    assert_eq!(got.traces[0].step_id, "step_tax");
    assert_eq!(
        got.traces[0]
            .output_payload
            .get("tax")
            .and_then(|v| v.as_f64()),
        Some(8.25)
    );
    assert!(got.traces[0].is_async);
    assert_eq!(got.traces[0].attempts, 2);
    assert_eq!(got.traces[1].error_message, "smtp down");

    let got_b = store.get_state(&b.job_id).unwrap();
    assert_eq!(got_b.status, ExecutionStatus::Failed);
    assert_eq!(got_b.traces.len(), 1);

    let listed = store.list_states(&workflow, 10).unwrap();
    let ids: Vec<&str> = listed.iter().map(|s| s.job_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![b.job_id.as_str(), a.job_id.as_str()],
        "newest first"
    );
    assert_eq!(listed[1].traces.len(), 2, "list returns traces too");
    assert_eq!(store.list_states(&workflow, 1).unwrap().len(), 1);

    assert!(store.get_state(&unique("missing")).is_err());
}

/// A second store on the same backend (another instance) sees the same jobs.
fn exercise_shared(make: impl Fn() -> Arc<dyn IStateStore>) {
    let (one, two) = (make(), make());
    let job = sample(&unique("shared"), &unique("wf"), "2026-10-25T12:00:00Z");
    one.save_state(job.clone()).unwrap();
    let seen = two
        .get_state(&job.job_id)
        .expect("second instance reads the job");
    assert_eq!(seen.traces.len(), 2);
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn relational(kind: &str, dsn: &str) -> Arc<dyn IStateStore> {
    StoreFactory::new().create(kind, Some(&vmap! { "dsn" => dsn }))
}

#[cfg(feature = "sqlite")]
#[test]
fn sqlite_store_persists_to_disk() {
    let dir = std::env::temp_dir().join(unique("octopus-sqlite"));
    let path = dir.join("state.sqlite").to_string_lossy().into_owned();
    let make = || StoreFactory::new().create("sqlite", Some(&vmap! { "path" => path.clone() }));
    exercise(make());
    exercise_shared(make);
    assert!(std::path::Path::new(&path).exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn postgres_store_round_trip() {
    let Some(dsn) = env("OCTOPUS_TEST_POSTGRES_DSN") else {
        return;
    };
    exercise(relational("postgres", &dsn));
    exercise_shared(|| relational("postgres", &dsn));
}

#[test]
fn mysql_store_round_trip() {
    let Some(dsn) = env("OCTOPUS_TEST_MYSQL_DSN") else {
        return;
    };
    exercise(relational("mysql", &dsn));
    exercise_shared(|| relational("mysql", &dsn));
}

#[test]
fn sqlserver_store_round_trip() {
    let Some(dsn) = env("OCTOPUS_TEST_SQLSERVER_DSN") else {
        return;
    };
    exercise(relational("sqlserver", &dsn));
    exercise_shared(|| relational("sqlserver", &dsn));
}

#[test]
fn redis_store_round_trip_and_cross_instance_events() {
    let Some(url) = env("OCTOPUS_TEST_REDIS_URL") else {
        return;
    };
    let prefix = unique("octopus-test");
    let make = || -> Arc<dyn IStateStore> {
        StoreFactory::new().create(
            "redis",
            Some(&vmap! { "url" => url.clone(), "prefix" => prefix.clone() }),
        )
    };
    exercise(make());
    exercise_shared(make);

    // An event published on one instance reaches a subscriber on another.
    let (publisher, listener) = (make(), make());
    std::thread::sleep(Duration::from_millis(300)); // let the pub/sub listeners subscribe
    let job_id = unique("evt");
    let (rx, unsubscribe) = listener.subscribe_events(&job_id).unwrap();
    publisher
        .publish_event(
            &job_id,
            LiveEvent {
                event: "step".to_string(),
                data: Value::String("done".to_string()),
            },
        )
        .unwrap();
    let ev = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("event relayed through Redis");
    assert_eq!(ev.event, "step");
    assert_eq!(ev.data.as_str(), Some("done"));
    unsubscribe();

    let _ = ValueMap::new();
}

#[test]
fn unreachable_database_falls_back_to_memory() {
    // Nothing listens on port 1; the store must still work, in memory.
    let store = relational(
        "postgres",
        "postgresql://u:p@127.0.0.1:1/db?connect_timeout=1",
    );
    exercise(store);
}
