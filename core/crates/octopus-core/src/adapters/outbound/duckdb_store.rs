//! DuckDB state store, on the tables the Python SDK's `DuckDbStateStore`
//! writes, so either engine reads the other's file.
//!
//! Python's `execution_states` has no column for `client_host`,
//! `duration_ms`, `error_code` or `error_type`, and Python inserts its rows
//! positionally, so those go in a table of their own,
//! `execution_state_details`, which Python leaves alone. Live events stay in
//! this process, as with Python's store.

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

use duckdb::types::Value as DbValue;
use duckdb::{params, Connection};

use crate::adapters::outbound::in_memory_store::InMemoryStateStore;
use crate::domain::models::{ExecutionState, ExecutionStatus, LiveEvent, StepExecutionTrace};
use crate::domain::ports::IStateStore;
use crate::json::{self, Value, ValueMap};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS execution_states (
    job_id VARCHAR PRIMARY KEY,
    workflow_name VARCHAR,
    status VARCHAR,
    route VARCHAR,
    method VARCHAR,
    current_step_id VARCHAR,
    current_index INTEGER,
    total_steps INTEGER,
    data VARCHAR,
    output_data VARCHAR,
    error VARCHAR,
    started_at VARCHAR,
    finished_at VARCHAR
);
CREATE TABLE IF NOT EXISTS step_traces (
    job_id VARCHAR,
    step_id VARCHAR,
    activity_name VARCHAR,
    status VARCHAR,
    input_payload VARCHAR,
    output_payload VARCHAR,
    error_message VARCHAR,
    started_at VARCHAR,
    finished_at VARCHAR,
    duration_ms DOUBLE
);
CREATE TABLE IF NOT EXISTS execution_state_details (
    job_id VARCHAR PRIMARY KEY,
    client_host VARCHAR,
    duration_ms DOUBLE,
    error_code INTEGER,
    error_type VARCHAR
);";

const SELECT_STATE: &str = "
SELECT s.job_id, s.workflow_name, s.status, s.route, s.method, s.current_step_id,
       s.current_index, s.total_steps, s.data, s.output_data, s.error,
       s.started_at, s.finished_at,
       d.client_host, d.duration_ms, d.error_code, d.error_type
FROM execution_states s LEFT JOIN execution_state_details d ON d.job_id = s.job_id";

const SELECT_TRACES: &str = "
SELECT step_id, activity_name, status, input_payload, output_payload,
       error_message, started_at, finished_at, duration_ms
FROM step_traces WHERE job_id = ? ORDER BY rowid";

pub struct DuckDbStateStore {
    conn: Mutex<Connection>,
    /// Live events, and `is_async` / `attempts`, which no table holds.
    cache: Arc<InMemoryStateStore>,
}

impl DuckDbStateStore {
    /// Opens (or creates) the file at `path`; ":memory:" for none.
    pub fn open(path: &str) -> Result<Arc<Self>, String> {
        let conn = if path.is_empty() || path == ":memory:" {
            Connection::open_in_memory()
        } else {
            if let Some(dir) = std::path::Path::new(path).parent() {
                if !dir.as_os_str().is_empty() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("{}: {}", dir.display(), e))?;
                }
            }
            Connection::open(path)
        }
        .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            cache: InMemoryStateStore::new(),
        }))
    }

    fn write(&self, state: &ExecutionState) -> Result<(), duckdb::Error> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM execution_states WHERE job_id = ?",
            params![state.job_id],
        )?;
        tx.execute(
            "INSERT INTO execution_states VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                state.job_id,
                state.workflow_name,
                state.status.as_str(),
                state.route,
                state.method,
                state.current_step_id,
                state.current_index as i64,
                state.total_steps as i64,
                json::map_to_json(&state.data),
                state.output_data.as_ref().map(json::map_to_json),
                state.error,
                state.started_at,
                state.finished_at,
            ],
        )?;
        tx.execute(
            "DELETE FROM execution_state_details WHERE job_id = ?",
            params![state.job_id],
        )?;
        tx.execute(
            "INSERT INTO execution_state_details VALUES (?, ?, ?, ?, ?)",
            params![
                state.job_id,
                state.client_host,
                state.duration_ms,
                state.error_code,
                state.error_type,
            ],
        )?;
        tx.execute(
            "DELETE FROM step_traces WHERE job_id = ?",
            params![state.job_id],
        )?;
        {
            let mut insert =
                tx.prepare("INSERT INTO step_traces VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")?;
            for t in &state.traces {
                insert.execute(params![
                    state.job_id,
                    t.step_id,
                    t.activity_name,
                    t.status,
                    json::map_to_json(&t.input_payload),
                    json::map_to_json(&t.output_payload),
                    t.error_message,
                    t.started_at,
                    t.finished_at,
                    t.duration_ms,
                ])?;
            }
        }
        tx.commit()
    }

    fn read(&self, job_id: &str) -> Result<Option<ExecutionState>, duckdb::Error> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("{} WHERE s.job_id = ?", SELECT_STATE);
        let mut rows = conn
            .prepare(&sql)?
            .query_map(params![job_id], row_values)?
            .collect::<Result<Vec<_>, _>>()?;
        let Some(row) = rows.pop() else {
            return Ok(None);
        };
        let mut state = state_from_row(&row);
        state.traces = traces(&conn, job_id)?;
        drop(conn);
        self.enrich(&mut state);
        Ok(Some(state))
    }

    fn list(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, duckdb::Error> {
        let conn = self.conn.lock().unwrap();
        let all = workflow_name.is_empty() || workflow_name.eq_ignore_ascii_case("ALL");
        let rows = if all {
            let sql = format!("{} ORDER BY s.started_at DESC LIMIT ?", SELECT_STATE);
            conn.prepare(&sql)?
                .query_map(params![limit as i64], row_values)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let sql = format!(
                "{} WHERE s.workflow_name = ? ORDER BY s.started_at DESC LIMIT ?",
                SELECT_STATE
            );
            conn.prepare(&sql)?
                .query_map(params![workflow_name, limit as i64], row_values)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut states = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut state = state_from_row(row);
            state.traces = traces(&conn, &state.job_id)?;
            states.push(state);
        }
        drop(conn);
        for state in &mut states {
            self.enrich(state);
        }
        Ok(states)
    }

    /// No table has `is_async` / `attempts`: restore them from this
    /// process's copy when it holds the same steps.
    fn enrich(&self, state: &mut ExecutionState) {
        let Ok(local) = self.cache.get_state(&state.job_id) else {
            return;
        };
        for (tr, loc) in state.traces.iter_mut().zip(local.traces.iter()) {
            if tr.step_id == loc.step_id {
                tr.is_async = loc.is_async;
                tr.attempts = loc.attempts;
            }
        }
    }
}

fn row_values(row: &duckdb::Row<'_>) -> Result<Vec<DbValue>, duckdb::Error> {
    (0..row.as_ref().column_count())
        .map(|i| row.get::<_, DbValue>(i))
        .collect()
}

fn traces(conn: &Connection, job_id: &str) -> Result<Vec<StepExecutionTrace>, duckdb::Error> {
    let rows = conn
        .prepare(SELECT_TRACES)?
        .query_map(params![job_id], row_values)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .iter()
        .map(|r| StepExecutionTrace {
            step_id: text(r, 0),
            activity_name: text(r, 1),
            status: text(r, 2),
            is_async: false,
            input_payload: map(r, 3),
            output_payload: map(r, 4),
            error_message: text(r, 5),
            attempts: 0,
            started_at: text(r, 6),
            finished_at: text(r, 7),
            duration_ms: float(r, 8).unwrap_or(0.0),
        })
        .collect())
}

fn state_from_row(r: &[DbValue]) -> ExecutionState {
    ExecutionState {
        job_id: text(r, 0),
        workflow_name: text(r, 1),
        status: ExecutionStatus::from_str(&text(r, 2)),
        route: text(r, 3),
        method: text(r, 4),
        current_step_id: text(r, 5),
        current_index: int(r, 6).unwrap_or(0).max(0) as usize,
        total_steps: int(r, 7).unwrap_or(0).max(0) as usize,
        data: map(r, 8),
        output_data: opt_text(r, 9).map(|s| parse_map(&s)),
        traces: Vec::new(),
        error: opt_text(r, 10),
        started_at: text(r, 11),
        finished_at: opt_text(r, 12),
        client_host: text(r, 13),
        duration_ms: float(r, 14).unwrap_or(0.0),
        error_code: int(r, 15).map(|c| c as i32),
        error_type: opt_text(r, 16),
        route_output: None,
    }
}

fn opt_text(r: &[DbValue], i: usize) -> Option<String> {
    match r.get(i)? {
        DbValue::Text(s) if !s.is_empty() => Some(s.clone()),
        DbValue::Null | DbValue::Text(_) => None,
        DbValue::Int(n) => Some(n.to_string()),
        DbValue::BigInt(n) => Some(n.to_string()),
        DbValue::Double(f) => Some(f.to_string()),
        _ => None,
    }
}

fn text(r: &[DbValue], i: usize) -> String {
    opt_text(r, i).unwrap_or_default()
}

fn int(r: &[DbValue], i: usize) -> Option<i64> {
    match r.get(i)? {
        DbValue::TinyInt(n) => Some(*n as i64),
        DbValue::SmallInt(n) => Some(*n as i64),
        DbValue::Int(n) => Some(*n as i64),
        DbValue::BigInt(n) => Some(*n),
        DbValue::Double(f) => Some(*f as i64),
        DbValue::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn float(r: &[DbValue], i: usize) -> Option<f64> {
    match r.get(i)? {
        DbValue::Double(f) => Some(*f),
        DbValue::Float(f) => Some(*f as f64),
        DbValue::Int(n) => Some(*n as f64),
        DbValue::BigInt(n) => Some(*n as f64),
        DbValue::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn map(r: &[DbValue], i: usize) -> ValueMap {
    opt_text(r, i).map(|s| parse_map(&s)).unwrap_or_default()
}

fn parse_map(s: &str) -> ValueMap {
    match json::parse(s) {
        Ok(Value::Object(m)) => m,
        _ => ValueMap::new(),
    }
}

impl IStateStore for DuckDbStateStore {
    fn save_state(&self, state: ExecutionState) -> Result<(), String> {
        let written = self
            .write(&state)
            .map_err(|e| format!("duckdb save failed: {}", e));
        self.cache.save_state(state)?;
        written
    }

    fn get_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        match self.read(job_id) {
            Ok(Some(state)) => Ok(state),
            Ok(None) => Err(format!("state for job '{}' not found", job_id)),
            Err(e) => Err(format!("duckdb read failed: {}", e)),
        }
    }

    fn list_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        let limit = if limit == 0 { 100 } else { limit };
        self.list(workflow_name, limit)
            .map_err(|e| format!("duckdb list failed: {}", e))
    }

    fn publish_event(&self, job_id: &str, event: LiveEvent) -> Result<(), String> {
        self.cache.publish_event(job_id, event)
    }

    fn has_listeners(&self, job_id: &str) -> bool {
        self.cache.has_listeners(job_id)
    }

    fn subscribe_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String> {
        self.cache.subscribe_events(job_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, wf: &str, started: &str) -> ExecutionState {
        let mut data = ValueMap::new();
        data.insert("amount".to_string(), Value::Float(12.5));
        let mut out = ValueMap::new();
        out.insert("ok".to_string(), Value::Bool(true));
        ExecutionState {
            job_id: id.to_string(),
            workflow_name: wf.to_string(),
            status: ExecutionStatus::from_str("FAILED"),
            route: "/pay".to_string(),
            method: "POST".to_string(),
            current_step_id: "charge".to_string(),
            current_index: 1,
            total_steps: 2,
            data,
            output_data: Some(out.clone()),
            traces: vec![
                StepExecutionTrace {
                    step_id: "validate".to_string(),
                    activity_name: "Validate".to_string(),
                    status: "COMPLETED".to_string(),
                    is_async: true,
                    input_payload: ValueMap::new(),
                    output_payload: out,
                    error_message: String::new(),
                    attempts: 2,
                    started_at: started.to_string(),
                    finished_at: started.to_string(),
                    duration_ms: 1.5,
                },
                StepExecutionTrace {
                    step_id: "charge".to_string(),
                    activity_name: "Charge".to_string(),
                    status: "FAILED".to_string(),
                    error_message: "card declined".to_string(),
                    ..Default::default()
                },
            ],
            started_at: started.to_string(),
            finished_at: Some(started.to_string()),
            client_host: "10.0.0.7".to_string(),
            duration_ms: 4.25,
            error: Some("card declined".to_string()),
            error_code: Some(402),
            error_type: Some("CardDeclined".to_string()),
            route_output: None,
        }
    }

    #[test]
    fn a_job_reads_back_as_it_was_saved() {
        let store = DuckDbStateStore::open(":memory:").unwrap();
        let saved = job("job_1", "payments", "2026-10-30T10:00:00Z");
        store.save_state(saved.clone()).unwrap();
        let back = store.get_state("job_1").unwrap();
        assert_eq!(back.to_value(), saved.to_value());
        assert!(store.get_state("nope").is_err());
    }

    #[test]
    fn saving_again_replaces_the_job_and_its_traces() {
        let store = DuckDbStateStore::open(":memory:").unwrap();
        let mut st = job("job_1", "payments", "2026-10-30T10:00:00Z");
        store.save_state(st.clone()).unwrap();
        st.traces.truncate(1);
        st.status = ExecutionStatus::from_str("COMPLETED");
        store.save_state(st).unwrap();
        let back = store.get_state("job_1").unwrap();
        assert_eq!(back.traces.len(), 1);
        assert_eq!(back.status.as_str(), "COMPLETED");
    }

    #[test]
    fn jobs_list_newest_first_by_workflow() {
        let store = DuckDbStateStore::open(":memory:").unwrap();
        store
            .save_state(job("a", "payments", "2026-10-30T10:00:00Z"))
            .unwrap();
        store
            .save_state(job("b", "payments", "2026-10-30T11:00:00Z"))
            .unwrap();
        store
            .save_state(job("c", "refunds", "2026-10-30T12:00:00Z"))
            .unwrap();
        let ids = |v: Vec<ExecutionState>| v.into_iter().map(|s| s.job_id).collect::<Vec<_>>();
        assert_eq!(ids(store.list_states("payments", 10).unwrap()), ["b", "a"]);
        assert_eq!(ids(store.list_states("", 2).unwrap()), ["c", "b"]);
    }

    #[test]
    fn a_file_keeps_the_jobs_for_the_next_process() {
        let dir = std::env::temp_dir().join(format!("octopus-duckdb-{}", std::process::id()));
        let path = dir.join("sub").join("jobs.duckdb");
        let path = path.to_str().unwrap();
        {
            let store = DuckDbStateStore::open(path).unwrap();
            store
                .save_state(job("job_1", "payments", "2026-10-30T10:00:00Z"))
                .unwrap();
        }
        let store = DuckDbStateStore::open(path).unwrap();
        let back = store.get_state("job_1").unwrap();
        assert_eq!(back.error_code, Some(402));
        assert_eq!(back.traces.len(), 2);
        // Not this process's copy: the schema has no is_async / attempts.
        assert!(!back.traces[0].is_async);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
