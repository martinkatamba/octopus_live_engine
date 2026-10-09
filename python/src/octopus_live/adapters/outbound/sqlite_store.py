import asyncio
import json
import sqlite3
from typing import Any, AsyncIterator, Dict, List, Optional
from ...domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace
from ...domain.ports.outbound import IStateStore


class SqliteStateStore(IStateStore):
    """Zero-dependency embedded SQLite persistent state store and trace repository."""

    def __init__(self, db_path: str = ":memory:"):
        self.db_path = db_path
        self._lock = asyncio.Lock()
        self._event_queues: Dict[str, List[asyncio.Queue[Dict[str, Any]]]] = {}
        self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        self._init_db()

    def _init_db(self) -> None:
        cur = self._conn.cursor()
        cur.execute("""
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
        """)
        try:
            cur.execute("ALTER TABLE execution_states ADD COLUMN client_host TEXT")
        except Exception:
            pass
        try:
            cur.execute("ALTER TABLE execution_states ADD COLUMN duration_ms REAL")
        except Exception:
            pass
        try:
            cur.execute("ALTER TABLE execution_states ADD COLUMN error_code INTEGER")
        except Exception:
            pass
        try:
            cur.execute("ALTER TABLE execution_states ADD COLUMN error_type TEXT")
        except Exception:
            pass

        cur.execute("""
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
        """)
        self._conn.commit()

    async def save_state(self, state: ExecutionState) -> None:
        async with self._lock:
            cur = self._conn.cursor()
            cur.execute("""
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
            """, (
                state.job_id,
                state.workflow_name,
                state.status.value if hasattr(state.status, "value") else str(state.status),
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

            # Replace traces for this job
            cur.execute("DELETE FROM step_traces WHERE job_id = ?", (state.job_id,))
            for t in state.traces:
                cur.execute("""
                    INSERT INTO step_traces (
                        job_id, step_id, activity_name, status,
                        input_payload, output_payload, error_message,
                        started_at, finished_at, duration_ms
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """, (
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
            self._conn.commit()

    def _parse_row(self, row: Any, trace_rows: List[Any]) -> ExecutionState:
        traces = []
        for tr in trace_rows:
            traces.append(StepExecutionTrace(
                step_id=tr[0],
                activity_name=tr[1],
                status=tr[2],
                input_payload=json.loads(tr[3]) if tr[3] else {},
                output_payload=json.loads(tr[4]) if tr[4] else {},
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

        return ExecutionState(
            job_id=row[0],
            workflow_name=row[1],
            status=status_enum,
            route=row[3],
            method=row[4],
            current_step_id=row[5],
            current_index=int(row[6] or 0),
            total_steps=int(row[7] or 0),
            data=json.loads(row[8]) if row[8] else {},
            output_data=json.loads(row[9]) if row[9] else None,
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
        cur = self._conn.cursor()
        cur.execute("""
            SELECT job_id, workflow_name, status, route, method,
                   current_step_id, current_index, total_steps,
                   data, output_data, error, started_at, finished_at,
                   client_host, duration_ms, error_code, error_type
            FROM execution_states WHERE job_id = ?
        """, (job_id,))
        row = cur.fetchone()

        # If not found and starts with wf-, resolve Temporal workflow ID
        if not row and job_id.startswith("wf-"):
            parts = job_id.split("-", 2)
            if len(parts) >= 3:
                cur.execute("""
                    SELECT job_id, workflow_name, status, route, method,
                           current_step_id, current_index, total_steps,
                           data, output_data, error, started_at, finished_at,
                           client_host, duration_ms, error_code, error_type
                    FROM execution_states WHERE job_id = ?
                """, (parts[2],))
                row = cur.fetchone()
            if not row:
                cur.execute("""
                    SELECT job_id, workflow_name, status, route, method,
                           current_step_id, current_index, total_steps,
                           data, output_data, error, started_at, finished_at,
                           client_host, duration_ms, error_code, error_type
                    FROM execution_states WHERE job_id = ?
                """, (job_id[3:],))
                row = cur.fetchone()

        if not row:
            return None

        actual_job_id = row[0]
        cur.execute("""
            SELECT step_id, activity_name, status, input_payload,
                   output_payload, error_message, started_at, finished_at, duration_ms
            FROM step_traces WHERE job_id = ? ORDER BY id ASC
        """, (actual_job_id,))
        trace_rows = cur.fetchall()
        return self._parse_row(row, trace_rows)

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        async with self._lock:
            return self.get_state_sync(job_id)

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        async with self._lock:
            cur = self._conn.cursor()
            if workflow_name:
                cur.execute("""
                    SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?
                """, (workflow_name, limit))
            else:
                cur.execute("""
                    SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?
                """, (limit,))
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
