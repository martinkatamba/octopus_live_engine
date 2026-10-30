use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::adapters::outbound::in_memory_store::InMemoryStateStore;
use crate::adapters::outbound::sql_backend::{SqlBackend, SqlRow, SqlStatement, SqlValue};
use crate::domain::models::{ExecutionState, ExecutionStatus, LiveEvent, StepExecutionTrace};
use crate::domain::ports::IStateStore;
use crate::json::{self, Value, ValueMap};

/// Columns of `execution_states`, in the order every dialect selects them.
const STATE_COLUMNS: &str = "job_id, workflow_name, status, route, method, \
     current_step_id, current_index, total_steps, \
     data, output_data, error, started_at, finished_at, \
     client_host, duration_ms, error_code, error_type";

/// Columns of `step_traces` read back for a job (prefixed by job_id in bulk reads).
const TRACE_COLUMNS: &str = "step_id, activity_name, status, input_payload, \
     output_payload, error_message, started_at, finished_at, duration_ms";

/// The table layout matches the Go engine, so both can share one database.
pub trait SqlDialect: Send + Sync {
    fn name(&self) -> &str;
    fn placeholder(&self, index: usize) -> String;
    fn ddl_execution_states(&self) -> &str;
    fn ddl_step_traces(&self) -> &str;
    fn upsert_execution_state_sql(&self) -> &str;
    fn insert_step_trace_sql(&self) -> &str;
    fn select_state_sql(&self) -> &str;
    fn select_traces_sql(&self) -> &str;
    fn list_states_sql(&self, has_workflow: bool) -> &str;
    fn delete_traces_sql(&self) -> &str;
    /// Row-limit clause appended after ORDER BY, with the limit bound at `index`.
    fn limit_clause(&self, index: usize) -> String {
        format!("LIMIT {}", self.placeholder(index))
    }
}

pub struct PostgresDialect;
impl SqlDialect for PostgresDialect {
    fn name(&self) -> &str {
        "postgres"
    }
    fn placeholder(&self, index: usize) -> String {
        format!("${}", index)
    }
    fn ddl_execution_states(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS execution_states (
            job_id VARCHAR(255) PRIMARY KEY,
            workflow_name VARCHAR(255) NOT NULL,
            status VARCHAR(64) NOT NULL,
            route VARCHAR(512),
            method VARCHAR(32),
            current_step_id VARCHAR(255),
            current_index INT,
            total_steps INT,
            data TEXT,
            output_data TEXT,
            error TEXT,
            started_at VARCHAR(64),
            finished_at VARCHAR(64),
            client_host VARCHAR(255),
            duration_ms DOUBLE PRECISION,
            error_code INT,
            error_type VARCHAR(255)
        );
        CREATE INDEX IF NOT EXISTS idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);"
    }
    fn ddl_step_traces(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS step_traces (
            id BIGSERIAL PRIMARY KEY,
            job_id VARCHAR(255) NOT NULL REFERENCES execution_states(job_id) ON DELETE CASCADE,
            step_id VARCHAR(255) NOT NULL,
            activity_name VARCHAR(255),
            status VARCHAR(64) NOT NULL,
            input_payload TEXT,
            output_payload TEXT,
            error_message TEXT,
            started_at VARCHAR(64),
            finished_at VARCHAR(64),
            duration_ms DOUBLE PRECISION
        );
        CREATE INDEX IF NOT EXISTS idx_step_traces_job_id ON step_traces(job_id);"
    }
    fn upsert_execution_state_sql(&self) -> &str {
        "INSERT INTO execution_states (
            job_id, workflow_name, status, route, method,
            current_step_id, current_index, total_steps,
            data, output_data, error, started_at, finished_at,
            client_host, duration_ms, error_code, error_type
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        ON CONFLICT (job_id) DO UPDATE SET
            status = EXCLUDED.status,
            route = EXCLUDED.route,
            method = EXCLUDED.method,
            current_step_id = EXCLUDED.current_step_id,
            current_index = EXCLUDED.current_index,
            total_steps = EXCLUDED.total_steps,
            data = EXCLUDED.data,
            output_data = EXCLUDED.output_data,
            error = EXCLUDED.error,
            finished_at = EXCLUDED.finished_at,
            client_host = EXCLUDED.client_host,
            duration_ms = EXCLUDED.duration_ms,
            error_code = EXCLUDED.error_code,
            error_type = EXCLUDED.error_type"
    }
    fn insert_step_trace_sql(&self) -> &str {
        "INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
    }
    fn select_state_sql(&self) -> &str {
        "SELECT job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
         FROM execution_states WHERE job_id = $1"
    }
    fn select_traces_sql(&self) -> &str {
        "SELECT step_id, activity_name, status, input_payload,
                output_payload, error_message, started_at, finished_at, duration_ms
         FROM step_traces WHERE job_id = $1 ORDER BY id ASC"
    }
    fn list_states_sql(&self, has_workflow: bool) -> &str {
        if has_workflow {
            "SELECT job_id FROM execution_states WHERE workflow_name = $1 ORDER BY started_at DESC LIMIT $2"
        } else {
            "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT $1"
        }
    }
    fn delete_traces_sql(&self) -> &str {
        "DELETE FROM step_traces WHERE job_id = $1"
    }
}

pub struct MySqlDialect;
impl SqlDialect for MySqlDialect {
    fn name(&self) -> &str {
        "mysql"
    }
    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }
    fn ddl_execution_states(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS execution_states (
            job_id VARCHAR(255) PRIMARY KEY,
            workflow_name VARCHAR(255) NOT NULL,
            status VARCHAR(64) NOT NULL,
            route VARCHAR(512),
            method VARCHAR(32),
            current_step_id VARCHAR(255),
            current_index INT,
            total_steps INT,
            data LONGTEXT,
            output_data LONGTEXT,
            error TEXT,
            started_at VARCHAR(64),
            finished_at VARCHAR(64),
            client_host VARCHAR(255),
            duration_ms DOUBLE,
            error_code INT,
            error_type VARCHAR(255),
            INDEX idx_exec_states_wf_started (workflow_name, started_at DESC)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
    }
    fn ddl_step_traces(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS step_traces (
            id BIGINT AUTO_INCREMENT PRIMARY KEY,
            job_id VARCHAR(255) NOT NULL,
            step_id VARCHAR(255) NOT NULL,
            activity_name VARCHAR(255),
            status VARCHAR(64) NOT NULL,
            input_payload LONGTEXT,
            output_payload LONGTEXT,
            error_message TEXT,
            started_at VARCHAR(64),
            finished_at VARCHAR(64),
            duration_ms DOUBLE,
            INDEX idx_step_traces_job_id (job_id),
            FOREIGN KEY (job_id) REFERENCES execution_states(job_id) ON DELETE CASCADE
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"
    }
    fn upsert_execution_state_sql(&self) -> &str {
        "INSERT INTO execution_states (
            job_id, workflow_name, status, route, method,
            current_step_id, current_index, total_steps,
            data, output_data, error, started_at, finished_at,
            client_host, duration_ms, error_code, error_type
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON DUPLICATE KEY UPDATE
            status = VALUES(status),
            route = VALUES(route),
            method = VALUES(method),
            current_step_id = VALUES(current_step_id),
            current_index = VALUES(current_index),
            total_steps = VALUES(total_steps),
            data = VALUES(data),
            output_data = VALUES(output_data),
            error = VALUES(error),
            finished_at = VALUES(finished_at),
            client_host = VALUES(client_host),
            duration_ms = VALUES(duration_ms),
            error_code = VALUES(error_code),
            error_type = VALUES(error_type)"
    }
    fn insert_step_trace_sql(&self) -> &str {
        "INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    }
    fn select_state_sql(&self) -> &str {
        "SELECT job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
         FROM execution_states WHERE job_id = ?"
    }
    fn select_traces_sql(&self) -> &str {
        "SELECT step_id, activity_name, status, input_payload,
                output_payload, error_message, started_at, finished_at, duration_ms
         FROM step_traces WHERE job_id = ? ORDER BY id ASC"
    }
    fn list_states_sql(&self, has_workflow: bool) -> &str {
        if has_workflow {
            "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
        } else {
            "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?"
        }
    }
    fn delete_traces_sql(&self) -> &str {
        "DELETE FROM step_traces WHERE job_id = ?"
    }
}

pub struct SqlServerDialect;
impl SqlDialect for SqlServerDialect {
    fn name(&self) -> &str {
        "sqlserver"
    }
    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }
    fn ddl_execution_states(&self) -> &str {
        "IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='execution_states' and xtype='U')
        BEGIN
            CREATE TABLE execution_states (
                job_id NVARCHAR(255) PRIMARY KEY,
                workflow_name NVARCHAR(255) NOT NULL,
                status NVARCHAR(64) NOT NULL,
                route NVARCHAR(512),
                method NVARCHAR(32),
                current_step_id NVARCHAR(255),
                current_index INT,
                total_steps INT,
                data NVARCHAR(MAX),
                output_data NVARCHAR(MAX),
                error NVARCHAR(MAX),
                started_at NVARCHAR(64),
                finished_at NVARCHAR(64),
                client_host NVARCHAR(255),
                duration_ms FLOAT,
                error_code INT,
                error_type NVARCHAR(255)
            );
            CREATE INDEX idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);
        END"
    }
    fn ddl_step_traces(&self) -> &str {
        "IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='step_traces' and xtype='U')
        BEGIN
            CREATE TABLE step_traces (
                id BIGINT IDENTITY(1,1) PRIMARY KEY,
                job_id NVARCHAR(255) NOT NULL FOREIGN KEY REFERENCES execution_states(job_id) ON DELETE CASCADE,
                step_id NVARCHAR(255) NOT NULL,
                activity_name NVARCHAR(255),
                status NVARCHAR(64) NOT NULL,
                input_payload NVARCHAR(MAX),
                output_payload NVARCHAR(MAX),
                error_message NVARCHAR(MAX),
                started_at NVARCHAR(64),
                finished_at NVARCHAR(64),
                duration_ms FLOAT
            );
            CREATE INDEX idx_step_traces_job_id ON step_traces(job_id);
        END"
    }
    fn upsert_execution_state_sql(&self) -> &str {
        "MERGE INTO execution_states AS target
        USING (
            SELECT ? AS job_id, ? AS workflow_name, ? AS status, ? AS route, ? AS method,
                   ? AS current_step_id, ? AS current_index, ? AS total_steps,
                   ? AS data, ? AS output_data, ? AS error, ? AS started_at, ? AS finished_at,
                   ? AS client_host, ? AS duration_ms, ? AS error_code, ? AS error_type
        ) AS source
        ON target.job_id = source.job_id
        WHEN MATCHED THEN
            UPDATE SET
                status = source.status,
                route = source.route,
                method = source.method,
                current_step_id = source.current_step_id,
                current_index = source.current_index,
                total_steps = source.total_steps,
                data = source.data,
                output_data = source.output_data,
                error = source.error,
                finished_at = source.finished_at,
                client_host = source.client_host,
                duration_ms = source.duration_ms,
                error_code = source.error_code,
                error_type = source.error_type
        WHEN NOT MATCHED THEN
            INSERT (
                job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
            ) VALUES (
                source.job_id, source.workflow_name, source.status, source.route, source.method,
                source.current_step_id, source.current_index, source.total_steps,
                source.data, source.output_data, source.error, source.started_at, source.finished_at,
                source.client_host, source.duration_ms, source.error_code, source.error_type
            );"
    }
    fn insert_step_trace_sql(&self) -> &str {
        "INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    }
    fn select_state_sql(&self) -> &str {
        "SELECT job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
         FROM execution_states WHERE job_id = ?"
    }
    fn select_traces_sql(&self) -> &str {
        "SELECT step_id, activity_name, status, input_payload,
                output_payload, error_message, started_at, finished_at, duration_ms
         FROM step_traces WHERE job_id = ? ORDER BY id ASC"
    }
    fn list_states_sql(&self, has_workflow: bool) -> &str {
        // OFFSET/FETCH keeps the parameter order (workflow, limit) the same as the other dialects.
        if has_workflow {
            "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC OFFSET 0 ROWS FETCH NEXT ? ROWS ONLY"
        } else {
            "SELECT job_id FROM execution_states ORDER BY started_at DESC OFFSET 0 ROWS FETCH NEXT ? ROWS ONLY"
        }
    }
    fn delete_traces_sql(&self) -> &str {
        "DELETE FROM step_traces WHERE job_id = ?"
    }
    fn limit_clause(&self, _index: usize) -> String {
        "OFFSET 0 ROWS FETCH NEXT ? ROWS ONLY".to_string()
    }
}

pub struct SqliteDialect;
impl SqlDialect for SqliteDialect {
    fn name(&self) -> &str {
        "sqlite"
    }
    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }
    fn ddl_execution_states(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS execution_states (
            job_id TEXT PRIMARY KEY,
            workflow_name TEXT NOT NULL,
            status TEXT NOT NULL,
            route TEXT,
            method TEXT,
            current_step_id TEXT,
            current_index INTEGER,
            total_steps INTEGER,
            data TEXT,
            output_data TEXT,
            error TEXT,
            started_at TEXT,
            finished_at TEXT,
            client_host TEXT,
            duration_ms REAL,
            error_code INTEGER,
            error_type TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);"
    }
    fn ddl_step_traces(&self) -> &str {
        "CREATE TABLE IF NOT EXISTS step_traces (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL,
            step_id TEXT NOT NULL,
            activity_name TEXT,
            status TEXT NOT NULL,
            input_payload TEXT,
            output_payload TEXT,
            error_message TEXT,
            started_at TEXT,
            finished_at TEXT,
            duration_ms REAL,
            FOREIGN KEY (job_id) REFERENCES execution_states(job_id)
        );
        CREATE INDEX IF NOT EXISTS idx_step_traces_job_id ON step_traces(job_id);"
    }
    fn upsert_execution_state_sql(&self) -> &str {
        "INSERT INTO execution_states (
            job_id, workflow_name, status, route, method,
            current_step_id, current_index, total_steps,
            data, output_data, error, started_at, finished_at,
            client_host, duration_ms, error_code, error_type
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(job_id) DO UPDATE SET
            status=excluded.status,
            route=excluded.route,
            method=excluded.method,
            current_step_id=excluded.current_step_id,
            current_index=excluded.current_index,
            total_steps=excluded.total_steps,
            data=excluded.data,
            output_data=excluded.output_data,
            error=excluded.error,
            finished_at=excluded.finished_at,
            client_host=excluded.client_host,
            duration_ms=excluded.duration_ms,
            error_code=excluded.error_code,
            error_type=excluded.error_type"
    }
    fn insert_step_trace_sql(&self) -> &str {
        "INSERT INTO step_traces (
            job_id, step_id, activity_name, status,
            input_payload, output_payload, error_message,
            started_at, finished_at, duration_ms
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    }
    fn select_state_sql(&self) -> &str {
        "SELECT job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
         FROM execution_states WHERE job_id = ?"
    }
    fn select_traces_sql(&self) -> &str {
        "SELECT step_id, activity_name, status, input_payload,
                output_payload, error_message, started_at, finished_at, duration_ms
         FROM step_traces WHERE job_id = ? ORDER BY id ASC"
    }
    fn list_states_sql(&self, has_workflow: bool) -> &str {
        if has_workflow {
            "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
        } else {
            "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?"
        }
    }
    fn delete_traces_sql(&self) -> &str {
        "DELETE FROM step_traces WHERE job_id = ?"
    }
}

/// Persists executions to a SQL database, with an in-process write-through
/// copy that serves live events and keeps the engine running when the
/// database is unreachable.
pub struct RelationalStateStore {
    dialect: Box<dyn SqlDialect>,
    backend: Option<Arc<dyn SqlBackend>>,
    cache: Arc<InMemoryStateStore>,
    degraded: AtomicBool,
}

impl RelationalStateStore {
    /// A store without a database connection: it keeps jobs in memory only.
    pub fn new(dialect: Option<Box<dyn SqlDialect>>) -> Arc<Self> {
        Arc::new(Self::build(dialect, None))
    }

    /// A store backed by `backend`; creates the tables if they don't exist.
    pub fn with_backend(
        dialect: Box<dyn SqlDialect>,
        backend: Arc<dyn SqlBackend>,
    ) -> Result<Arc<Self>, String> {
        backend
            .exec_batch(dialect.ddl_execution_states())
            .map_err(|e| format!("creating execution_states: {}", e))?;
        backend
            .exec_batch(dialect.ddl_step_traces())
            .map_err(|e| format!("creating step_traces: {}", e))?;
        Ok(Arc::new(Self::build(Some(dialect), Some(backend))))
    }

    fn build(dialect: Option<Box<dyn SqlDialect>>, backend: Option<Arc<dyn SqlBackend>>) -> Self {
        Self {
            dialect: dialect.unwrap_or_else(|| Box::new(SqliteDialect)),
            backend,
            cache: InMemoryStateStore::new(),
            degraded: AtomicBool::new(false),
        }
    }

    pub fn dialect(&self) -> &dyn SqlDialect {
        self.dialect.as_ref()
    }

    pub fn is_persistent(&self) -> bool {
        self.backend.is_some()
    }

    fn report<T>(&self, op: &str, res: Result<T, String>) -> Result<T, String> {
        match &res {
            Ok(_) => {
                if self.degraded.swap(false, Ordering::Relaxed) {
                    eprintln!(
                        "✅ [RELATIONAL STORE] {} connection recovered",
                        self.dialect.name()
                    );
                }
            }
            Err(e) => {
                if !self.degraded.swap(true, Ordering::Relaxed) {
                    eprintln!(
                        "⚠️ [RELATIONAL STORE] {} {} failed, serving from memory until it recovers: {}",
                        self.dialect.name(),
                        op,
                        e
                    );
                }
            }
        }
        res
    }

    fn save_statements(&self, state: &ExecutionState) -> Vec<SqlStatement> {
        let d = &self.dialect;
        let output = state
            .output_data
            .as_ref()
            .map(|o| SqlValue::Text(json::map_to_json(o)))
            .unwrap_or(SqlValue::Null);
        let mut stmts = vec![
            (
                d.upsert_execution_state_sql().to_string(),
                vec![
                    SqlValue::text(&state.job_id),
                    SqlValue::text(&state.workflow_name),
                    SqlValue::text(state.status.as_str()),
                    SqlValue::text(&state.route),
                    SqlValue::text(&state.method),
                    SqlValue::text(&state.current_step_id),
                    SqlValue::Int(state.current_index as i64),
                    SqlValue::Int(state.total_steps as i64),
                    SqlValue::Text(json::map_to_json(&state.data)),
                    output,
                    SqlValue::opt_text(state.error.as_deref()),
                    SqlValue::text(&state.started_at),
                    SqlValue::opt_text(state.finished_at.as_deref()),
                    SqlValue::text(&state.client_host),
                    SqlValue::Float(state.duration_ms),
                    state
                        .error_code
                        .map(|c| SqlValue::Int(c as i64))
                        .unwrap_or(SqlValue::Null),
                    SqlValue::opt_text(state.error_type.as_deref()),
                ],
            ),
            (
                d.delete_traces_sql().to_string(),
                vec![SqlValue::text(&state.job_id)],
            ),
        ];
        for tr in &state.traces {
            stmts.push((
                d.insert_step_trace_sql().to_string(),
                vec![
                    SqlValue::text(&state.job_id),
                    SqlValue::text(&tr.step_id),
                    SqlValue::text(&tr.activity_name),
                    SqlValue::text(&tr.status),
                    SqlValue::Text(json::map_to_json(&tr.input_payload)),
                    SqlValue::Text(json::map_to_json(&tr.output_payload)),
                    SqlValue::text(&tr.error_message),
                    SqlValue::text(&tr.started_at),
                    SqlValue::text(&tr.finished_at),
                    SqlValue::Float(tr.duration_ms),
                ],
            ));
        }
        stmts
    }

    fn state_from_row(row: &SqlRow) -> ExecutionState {
        let col = |i: usize| row.get(i).cloned().unwrap_or(SqlValue::Null);
        let non_empty = |v: SqlValue| v.as_opt_string().filter(|s| !s.is_empty());
        ExecutionState {
            job_id: col(0).as_string(),
            workflow_name: col(1).as_string(),
            status: ExecutionStatus::from_str(&col(2).as_string()),
            route: col(3).as_string(),
            method: col(4).as_string(),
            current_step_id: col(5).as_string(),
            current_index: col(6).as_i64().unwrap_or(0).max(0) as usize,
            total_steps: col(7).as_i64().unwrap_or(0).max(0) as usize,
            data: parse_map(&col(8)),
            output_data: non_empty(col(9)).map(|s| parse_map(&SqlValue::Text(s))),
            traces: Vec::new(),
            error: non_empty(col(10)),
            started_at: col(11).as_string(),
            finished_at: non_empty(col(12)),
            client_host: col(13).as_string(),
            duration_ms: col(14).as_f64().unwrap_or(0.0),
            error_code: col(15).as_i64().map(|c| c as i32),
            error_type: non_empty(col(16)),
            route_output: None,
        }
    }

    fn trace_from_row(row: &[SqlValue]) -> StepExecutionTrace {
        let col = |i: usize| row.get(i).cloned().unwrap_or(SqlValue::Null);
        StepExecutionTrace {
            step_id: col(0).as_string(),
            activity_name: col(1).as_string(),
            status: col(2).as_string(),
            is_async: false,
            input_payload: parse_map(&col(3)),
            output_payload: parse_map(&col(4)),
            error_message: col(5).as_string(),
            attempts: 0,
            started_at: col(6).as_string(),
            finished_at: col(7).as_string(),
            duration_ms: col(8).as_f64().unwrap_or(0.0),
        }
    }

    /// The SQL schema has no columns for `is_async` / `attempts`; restore them
    /// from this process's copy when it holds the same steps.
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

    fn load_states(
        &self,
        backend: &Arc<dyn SqlBackend>,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        let d = &self.dialect;
        let filter = !workflow_name.is_empty() && !workflow_name.eq_ignore_ascii_case("ALL");
        let mut params = Vec::new();
        let mut sql = format!("SELECT {} FROM execution_states", STATE_COLUMNS);
        if filter {
            params.push(SqlValue::text(workflow_name));
            sql.push_str(&format!(" WHERE workflow_name = {}", d.placeholder(1)));
        }
        params.push(SqlValue::Int(limit as i64));
        sql.push_str(&format!(
            " ORDER BY started_at DESC {}",
            d.limit_clause(params.len())
        ));
        let rows = backend.query(&sql, &params)?;
        let mut states: Vec<ExecutionState> = rows.iter().map(Self::state_from_row).collect();
        if states.is_empty() {
            return Ok(states);
        }

        // One query for every listed job's traces instead of one per job.
        let ids: Vec<SqlValue> = states.iter().map(|s| SqlValue::text(&s.job_id)).collect();
        let marks: Vec<String> = (1..=ids.len()).map(|i| d.placeholder(i)).collect();
        let trace_sql = format!(
            "SELECT job_id, {} FROM step_traces WHERE job_id IN ({}) ORDER BY id ASC",
            TRACE_COLUMNS,
            marks.join(", ")
        );
        let mut by_job: HashMap<String, Vec<StepExecutionTrace>> = HashMap::new();
        for row in backend.query(&trace_sql, &ids)? {
            let job = row.first().map(|v| v.as_string()).unwrap_or_default();
            by_job
                .entry(job)
                .or_default()
                .push(Self::trace_from_row(&row[1..]));
        }
        for st in &mut states {
            st.traces = by_job.remove(&st.job_id).unwrap_or_default();
            self.enrich(st);
        }
        Ok(states)
    }

    fn load_state(
        &self,
        backend: &Arc<dyn SqlBackend>,
        job_id: &str,
    ) -> Result<Option<ExecutionState>, String> {
        let params = [SqlValue::text(job_id)];
        let rows = backend.query(self.dialect.select_state_sql(), &params)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let mut st = Self::state_from_row(row);
        st.traces = backend
            .query(self.dialect.select_traces_sql(), &params)?
            .iter()
            .map(|r| Self::trace_from_row(r))
            .collect();
        self.enrich(&mut st);
        Ok(Some(st))
    }
}

fn parse_map(v: &SqlValue) -> ValueMap {
    match v {
        SqlValue::Text(s) if !s.is_empty() => match json::parse(s) {
            Ok(Value::Object(m)) => m,
            _ => ValueMap::new(),
        },
        _ => ValueMap::new(),
    }
}

impl IStateStore for RelationalStateStore {
    fn save_state(&self, state: ExecutionState) -> Result<(), String> {
        let stmts = self.backend.as_ref().map(|_| self.save_statements(&state));
        self.cache.save_state(state)?;
        match (&self.backend, stmts) {
            (Some(backend), Some(stmts)) => self.report("save", backend.transaction(&stmts)),
            _ => Ok(()),
        }
    }

    fn get_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        if let Some(backend) = &self.backend {
            if let Ok(Some(st)) = self.report("read", self.load_state(backend, job_id)) {
                return Ok(st);
            }
        }
        self.cache.get_state(job_id)
    }

    fn list_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        let limit = if limit == 0 { 100 } else { limit };
        if let Some(backend) = &self.backend {
            if let Ok(states) = self.report("list", self.load_states(backend, workflow_name, limit))
            {
                return Ok(states);
            }
        }
        self.cache.list_states(workflow_name, limit)
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
