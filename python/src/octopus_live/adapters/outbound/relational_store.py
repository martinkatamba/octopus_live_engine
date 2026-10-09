import asyncio
import json
import os
import sqlite3
from typing import Any, AsyncIterator, Dict, List, Optional, Tuple, Union
from ...domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace
from ...domain.ports.outbound import IStateStore


class SqlDialect:
    """Base SQL Dialect interface for multi-database state storage."""

    @property
    def name(self) -> str:
        return "base"

    def placeholder(self, index: int) -> str:
        return "?"

    def get_ddl_execution_states(self) -> str:
        raise NotImplementedError

    def get_ddl_step_traces(self) -> str:
        raise NotImplementedError

    def get_upsert_execution_state_sql(self) -> str:
        raise NotImplementedError

    def get_insert_step_trace_sql(self) -> str:
        raise NotImplementedError

    def get_select_state_sql(self) -> str:
        raise NotImplementedError

    def get_select_traces_sql(self) -> str:
        raise NotImplementedError

    def get_list_states_sql(self, has_workflow: bool) -> str:
        raise NotImplementedError

    def get_delete_traces_sql(self) -> str:
        raise NotImplementedError


class SqliteDialect(SqlDialect):
    """SQLite dialect implementation."""

    @property
    def name(self) -> str:
        return "sqlite"

    def placeholder(self, index: int) -> str:
        return "?"

    def get_ddl_execution_states(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS execution_states (
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
            )
        """

    def get_ddl_step_traces(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS step_traces (
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
            )
        """

    def get_upsert_execution_state_sql(self) -> str:
        return """
            INSERT INTO execution_states (
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
                error_type=excluded.error_type
        """

    def get_insert_step_trace_sql(self) -> str:
        return """
            INSERT INTO step_traces (
                job_id, step_id, activity_name, status,
                input_payload, output_payload, error_message,
                started_at, finished_at, duration_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """

    def get_select_state_sql(self) -> str:
        return """
            SELECT job_id, workflow_name, status, route, method,
                   current_step_id, current_index, total_steps,
                   data, output_data, error, started_at, finished_at,
                   client_host, duration_ms, error_code, error_type
            FROM execution_states WHERE job_id = ?
        """

    def get_select_traces_sql(self) -> str:
        return """
            SELECT step_id, activity_name, status, input_payload,
                   output_payload, error_message, started_at, finished_at, duration_ms
            FROM step_traces WHERE job_id = ? ORDER BY id ASC
        """

    def get_list_states_sql(self, has_workflow: bool) -> str:
        if has_workflow:
            return "SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?"
        return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?"

    def get_delete_traces_sql(self) -> str:
        return "DELETE FROM step_traces WHERE job_id = ?"


class PostgresDialect(SqlDialect):
    """PostgreSQL dialect implementation with parameterized syntax and index definitions."""

    @property
    def name(self) -> str:
        return "postgres"

    def placeholder(self, index: int) -> str:
        return "%s"

    def get_ddl_execution_states(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS execution_states (
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
            CREATE INDEX IF NOT EXISTS idx_exec_states_wf_started ON execution_states(workflow_name, started_at DESC);
        """

    def get_ddl_step_traces(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS step_traces (
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
            CREATE INDEX IF NOT EXISTS idx_step_traces_job_id ON step_traces(job_id);
        """

    def get_upsert_execution_state_sql(self) -> str:
        return """
            INSERT INTO execution_states (
                job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
            ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
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
                error_type = EXCLUDED.error_type
        """

    def get_insert_step_trace_sql(self) -> str:
        return """
            INSERT INTO step_traces (
                job_id, step_id, activity_name, status,
                input_payload, output_payload, error_message,
                started_at, finished_at, duration_ms
            ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        """

    def get_select_state_sql(self) -> str:
        return """
            SELECT job_id, workflow_name, status, route, method,
                   current_step_id, current_index, total_steps,
                   data, output_data, error, started_at, finished_at,
                   client_host, duration_ms, error_code, error_type
            FROM execution_states WHERE job_id = %s
        """

    def get_select_traces_sql(self) -> str:
        return """
            SELECT step_id, activity_name, status, input_payload,
                   output_payload, error_message, started_at, finished_at, duration_ms
            FROM step_traces WHERE job_id = %s ORDER BY id ASC
        """

    def get_list_states_sql(self, has_workflow: bool) -> str:
        if has_workflow:
            return "SELECT job_id FROM execution_states WHERE workflow_name = %s ORDER BY started_at DESC LIMIT %s"
        return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT %s"

    def get_delete_traces_sql(self) -> str:
        return "DELETE FROM step_traces WHERE job_id = %s"


class MySqlDialect(SqlDialect):
    """MySQL / MariaDB dialect implementation."""

    @property
    def name(self) -> str:
        return "mysql"

    def placeholder(self, index: int) -> str:
        return "%s"

    def get_ddl_execution_states(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS execution_states (
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
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
        """

    def get_ddl_step_traces(self) -> str:
        return """
            CREATE TABLE IF NOT EXISTS step_traces (
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
            ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
        """

    def get_upsert_execution_state_sql(self) -> str:
        return """
            INSERT INTO execution_states (
                job_id, workflow_name, status, route, method,
                current_step_id, current_index, total_steps,
                data, output_data, error, started_at, finished_at,
                client_host, duration_ms, error_code, error_type
            ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
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
                error_type = VALUES(error_type)
        """

    def get_insert_step_trace_sql(self) -> str:
        return """
            INSERT INTO step_traces (
                job_id, step_id, activity_name, status,
                input_payload, output_payload, error_message,
                started_at, finished_at, duration_ms
            ) VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
        """

    def get_select_state_sql(self) -> str:
        return """
            SELECT job_id, workflow_name, status, route, method,
                   current_step_id, current_index, total_steps,
                   data, output_data, error, started_at, finished_at,
                   client_host, duration_ms, error_code, error_type
            FROM execution_states WHERE job_id = %s
        """

    def get_select_traces_sql(self) -> str:
        return """
            SELECT step_id, activity_name, status, input_payload,
                   output_payload, error_message, started_at, finished_at, duration_ms
            FROM step_traces WHERE job_id = %s ORDER BY id ASC
        """

    def get_list_states_sql(self, has_workflow: bool) -> str:
        if has_workflow:
            return "SELECT job_id FROM execution_states WHERE workflow_name = %s ORDER BY started_at DESC LIMIT %s"
        return "SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT %s"

    def get_delete_traces_sql(self) -> str:
        return "DELETE FROM step_traces WHERE job_id = %s"


class SqlServerDialect(SqlDialect):
    """Microsoft SQL Server (T-SQL) dialect implementation."""

    @property
    def name(self) -> str:
        return "sqlserver"

    def placeholder(self, index: int) -> str:
        return "%s"

    def get_ddl_execution_states(self) -> str:
        return """
            IF NOT EXISTS (SELECT * FROM sys.tables WHERE name='execution_states')
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
            END
        """

    def get_ddl_step_traces(self) -> str:
        return """
            IF NOT EXISTS (SELECT * FROM sys.tables WHERE name='step_traces')
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
            END
        """

    def get_upsert_execution_state_sql(self) -> str:
        return """
            MERGE INTO execution_states AS target
            USING (
                SELECT %s AS job_id, %s AS workflow_name, %s AS status, %s AS route, %s AS method,
                       %s AS current_step_id, %s AS current_index, %s AS total_steps,
                       %s AS data, %s AS output_data, %s AS error, %s AS started_at, %s AS finished_at,
                       %s AS client_host, %s AS duration_ms, %s AS error_code, %s AS error_type
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
                );
        """

    def get_insert_step_trace_sql(self) -> str:
        return """
            INSERT INTO step_traces (
                job_id, step_id, activity_name, status,
                input_payload, output_payload, error_message,
                started_at, finished_at, duration_ms
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """

    def get_select_state_sql(self) -> str:
        return """
            SELECT job_id, workflow_name, status, route, method,
                   current_step_id, current_index, total_steps,
                   data, output_data, error, started_at, finished_at,
                   client_host, duration_ms, error_code, error_type
            FROM execution_states WHERE job_id = ?
        """

    def get_select_traces_sql(self) -> str:
        return """
            SELECT step_id, activity_name, status, input_payload,
                   output_payload, error_message, started_at, finished_at, duration_ms
            FROM step_traces WHERE job_id = ? ORDER BY id ASC
        """

    def get_list_states_sql(self, has_workflow: bool) -> str:
        if has_workflow:
            return "SELECT TOP (?) job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC"
        return "SELECT TOP (?) job_id FROM execution_states ORDER BY started_at DESC"

    def get_delete_traces_sql(self) -> str:
        return "DELETE FROM step_traces WHERE job_id = ?"


class RelationalSqlStateStore(IStateStore):
    """Universal Relational Database State Store supporting PostgreSQL, MySQL, SQL Server, and SQLite."""

    def __init__(
        self,
        db_type: str = "sqlite",
        dsn: Optional[str] = None,
        db_path: Optional[str] = None,
        dialect: Optional[SqlDialect] = None,
        connection: Optional[Any] = None,
    ):
        self.db_type = (db_type or "sqlite").lower()
        self.dsn = dsn
        self.db_path = db_path or ":memory:"
        self._lock = asyncio.Lock()
        self._event_queues: Dict[str, List[asyncio.Queue[Dict[str, Any]]]] = {}

        # Resolve dialect
        if dialect:
            self.dialect = dialect
        elif self.db_type in ("postgres", "postgresql", "pgsql"):
            self.dialect = PostgresDialect()
        elif self.db_type in ("mysql", "mariadb"):
            self.dialect = MySqlDialect()
        elif self.db_type in ("sqlserver", "mssql", "tsql"):
            self.dialect = SqlServerDialect()
        else:
            self.dialect = SqliteDialect()

        # Embedded connection or external driver
        if connection:
            self._conn = connection
        elif self.dialect.name == "sqlite":
            if self.db_path != ":memory:":
                parent = os.path.dirname(self.db_path)
                if parent and not os.path.exists(parent):
                    os.makedirs(parent, exist_ok=True)
            self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        elif self.dialect.name == "postgres" and self.dsn:
            try:
                import psycopg2
                self._conn = psycopg2.connect(self.dsn)
                self._conn.autocommit = True
            except Exception:
                self._conn = None
        elif self.dialect.name == "mysql" and self.dsn:
            try:
                import pymysql
                from urllib.parse import urlparse, unquote
                dsn = self.dsn
                if dsn.startswith("mysql://") or dsn.startswith("mariadb://"):
                    u = urlparse(dsn)
                    self._conn = pymysql.connect(
                        host=u.hostname or "localhost",
                        port=u.port or 3306,
                        user=unquote(u.username or ""),
                        password=unquote(u.password or ""),
                        database=u.path.lstrip("/"),
                        autocommit=True,
                        cursorclass=pymysql.cursors.Cursor
                    )
                else:
                    self._conn = pymysql.connect(self.dsn, autocommit=True)
            except Exception:
                self._conn = None
        elif self.dialect.name == "sqlserver" and self.dsn:
            try:
                import pymssql
                from urllib.parse import urlparse, unquote
                dsn = self.dsn
                if dsn.startswith("sqlserver://") or dsn.startswith("mssql://") or dsn.startswith("tsql://"):
                    clean = dsn.replace("sqlserver://", "http://").replace("mssql://", "http://").replace("tsql://", "http://")
                    u = urlparse(clean)
                    self._conn = pymssql.connect(
                        server=u.hostname or "localhost",
                        port=u.port or 1433,
                        user=unquote(u.username or "sa"),
                        password=unquote(u.password or ""),
                        database=u.path.lstrip("/") or "octopus_db",
                        autocommit=True
                    )
                else:
                    kv = {}
                    for part in dsn.split(";"):
                        if "=" in part:
                            k, v = part.split("=", 1)
                            kv[k.strip().lower()] = v.strip()
                    server_val = kv.get("server", "localhost")
                    port = 1433
                    if "," in server_val:
                        srv, p = server_val.split(",", 1)
                        server_val = srv
                        port = int(p)
                    elif ":" in server_val:
                        srv, p = server_val.split(":", 1)
                        server_val = srv
                        port = int(p)
                    self._conn = pymssql.connect(
                        server=server_val,
                        port=port,
                        user=kv.get("user id", kv.get("user", kv.get("uid", "sa"))),
                        password=kv.get("password", kv.get("pwd", "")),
                        database=kv.get("database", kv.get("initial catalog", "octopus_db")),
                        autocommit=True
                    )
            except Exception as e:
                print(f"⚠️ [SQLSERVER STATE STORE] Failed to connect: {e}. Falling back to in-memory store.")
                self._conn = None
        else:
            self._conn = None

        self._init_db()

    def _init_db(self) -> None:
        if self._conn is not None:
            cur = self._conn.cursor()
            for ddl in (self.dialect.get_ddl_execution_states(), self.dialect.get_ddl_step_traces()):
                if self.dialect.name == "sqlserver":
                    try:
                        cur.execute(ddl.strip())
                    except Exception as e:
                        print(f"⚠️ [SQLSERVER DDL INIT WARNING]: {e}")
                else:
                    for stmt in ddl.strip().split(";"):
                        s = stmt.strip()
                        if s:
                            cur.execute(s)
            if hasattr(self._conn, "commit") and not getattr(self._conn, "autocommit", False):
                self._conn.commit()

    async def save_state(self, state: ExecutionState) -> None:
        async with self._lock:
            if self._conn is None:
                return

            cur = self._conn.cursor()
            upsert_sql = self.dialect.get_upsert_execution_state_sql()
            status_str = state.status.value if hasattr(state.status, "value") else str(state.status)

            cur.execute(upsert_sql, (
                state.job_id,
                state.workflow_name,
                status_str,
                state.route,
                state.method,
                state.current_step_id,
                state.current_index,
                state.total_steps,
                json.dumps(state.data or {}),
                json.dumps(state.output_data) if state.output_data is not None else None,
                state.error,
                state.started_at,
                state.finished_at,
                state.client_host,
                state.duration_ms,
                state.error_code,
                state.error_type
            ))

            # Delete old traces and insert updated traces
            cur.execute(self.dialect.get_delete_traces_sql(), (state.job_id,))
            insert_trace_sql = self.dialect.get_insert_step_trace_sql()
            for t in state.traces:
                cur.execute(insert_trace_sql, (
                    state.job_id,
                    t.step_id,
                    t.activity_name,
                    t.status,
                    json.dumps(t.input_payload or {}),
                    json.dumps(t.output_payload or {}),
                    t.error_message,
                    t.started_at,
                    t.finished_at,
                    t.duration_ms
                ))
            if hasattr(self._conn, "commit") and not getattr(self._conn, "autocommit", False):
                self._conn.commit()

    def _parse_row(self, row: Any, trace_rows: List[Any]) -> ExecutionState:
        traces = []
        for tr in trace_rows:
            traces.append(StepExecutionTrace(
                step_id=tr[0],
                activity_name=tr[1],
                status=tr[2],
                input_payload=json.loads(tr[3]) if tr[3] and isinstance(tr[3], str) else (tr[3] if isinstance(tr[3], dict) else {}),
                output_payload=json.loads(tr[4]) if tr[4] and isinstance(tr[4], str) else (tr[4] if isinstance(tr[4], dict) else {}),
                error_message=tr[5],
                started_at=tr[6],
                finished_at=tr[7],
                duration_ms=float(tr[8] or 0.0)
            ))

        status_val = row[2]
        try:
            status_enum = ExecutionStatus(status_val)
        except Exception:
            status_enum = ExecutionStatus.COMPLETED if status_val == "COMPLETED" else ExecutionStatus.FAILED

        data_val = row[8]
        if isinstance(data_val, str) and data_val:
            data_dict = json.loads(data_val)
        elif isinstance(data_val, dict):
            data_dict = data_val
        else:
            data_dict = {}

        output_val = row[9]
        if isinstance(output_val, str) and output_val:
            output_dict = json.loads(output_val)
        elif isinstance(output_val, dict):
            output_dict = output_val
        else:
            output_dict = None

        return ExecutionState(
            job_id=row[0],
            workflow_name=row[1],
            status=status_enum,
            route=row[3],
            method=row[4],
            current_step_id=row[5],
            current_index=int(row[6] or 0),
            total_steps=int(row[7] or 0),
            data=data_dict,
            output_data=output_dict,
            error=row[10],
            started_at=row[11],
            finished_at=row[12],
            client_host=row[13] if len(row) > 13 else None,
            duration_ms=float(row[14]) if len(row) > 14 and row[14] is not None else None,
            error_code=int(row[15]) if len(row) > 15 and row[15] is not None else None,
            error_type=row[16] if len(row) > 16 else None,
            traces=traces
        )

    def get_state_sync(self, job_id: str) -> Optional[ExecutionState]:
        if self._conn is None:
            return None

        cur = self._conn.cursor()
        cur.execute(self.dialect.get_select_state_sql(), (job_id,))
        row = cur.fetchone()

        if not row and job_id.startswith("wf-"):
            parts = job_id.split("-", 2)
            if len(parts) >= 3:
                cur.execute(self.dialect.get_select_state_sql(), (parts[2],))
                row = cur.fetchone()
            if not row:
                cur.execute(self.dialect.get_select_state_sql(), (job_id[3:],))
                row = cur.fetchone()

        if not row:
            return None

        actual_job_id = row[0]
        cur.execute(self.dialect.get_select_traces_sql(), (actual_job_id,))
        trace_rows = cur.fetchall()
        return self._parse_row(row, trace_rows)

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        async with self._lock:
            return self.get_state_sync(job_id)

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        async with self._lock:
            if self._conn is None:
                return []

            cur = self._conn.cursor()
            list_sql = self.dialect.get_list_states_sql(has_workflow=bool(workflow_name))
            if workflow_name:
                cur.execute(list_sql, (workflow_name, limit))
            else:
                cur.execute(list_sql, (limit,))
            rows = cur.fetchall()

        states = []
        for r in rows:
            st = await self.get_state(r[0])
            if st:
                states.append(st)
        return states

    async def publish_event(self, job_id: str, event_type: str, data: Dict[str, Any]) -> None:
        payload = {"event": event_type, "data": data}
        async with self._lock:
            queues = self._event_queues.get(job_id, [])
            for q in queues:
                await q.put(payload)

    async def subscribe(self, job_id: str) -> AsyncIterator[Dict[str, Any]]:
        q: asyncio.Queue[Dict[str, Any]] = asyncio.Queue()
        async with self._lock:
            if job_id not in self._event_queues:
                self._event_queues[job_id] = []
            self._event_queues[job_id].append(q)

        try:
            while True:
                item = await q.get()
                yield item
                if item.get("event") in ("workflow_completed", "workflow_failed", "workflow_rolled_back"):
                    break
        finally:
            async with self._lock:
                if job_id in self._event_queues and q in self._event_queues[job_id]:
                    self._event_queues[job_id].remove(q)
