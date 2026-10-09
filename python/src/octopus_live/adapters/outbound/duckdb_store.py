import asyncio
import json
import logging
from typing import Any, AsyncIterator, Dict, List, Optional
from ...domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace
from ...domain.ports.outbound import IStateStore

logger = logging.getLogger("OctopusLive.DuckDbStore")


class DuckDbStateStore(IStateStore):
    """High-performance embedded DuckDB columnar state store for trace analytics and execution tracking."""

    def __init__(self, db_path: str = ":memory:"):
        self.db_path = db_path
        self._lock = asyncio.Lock()
        self._event_queues: Dict[str, List[asyncio.Queue[Dict[str, Any]]]] = {}
        
        try:
            import duckdb
            self._duckdb = duckdb
            try:
                self._conn = duckdb.connect(self.db_path)
            except Exception as lock_err:
                logger.warning(f"Could not open duckdb file '{self.db_path}': {lock_err}. Using in-memory store fallback.")
                self._conn = duckdb.connect(":memory:")
            self._available = True
            self._init_db()
        except ImportError:
            logger.warning("duckdb package is not installed. Falling back to in-memory state tracking.")
            self._duckdb = None
            self._conn = None
            self._available = False
            self._fallback_states: Dict[str, ExecutionState] = {}

    def _init_db(self) -> None:
        if not self._available or self._conn is None:
            return
        self._conn.execute("""
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
            )
        """)
        self._conn.execute("""
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
            )
        """)

    async def save_state(self, state: ExecutionState) -> None:
        async with self._lock:
            if not self._available or self._conn is None:
                self._fallback_states[state.job_id] = state
                return

            status_str = state.status.value if hasattr(state.status, "value") else str(state.status)
            data_json = json.dumps(state.data or {})
            output_json = json.dumps(state.output_data) if state.output_data is not None else None

            # Upsert into DuckDB
            self._conn.execute("DELETE FROM execution_states WHERE job_id = ?", [state.job_id])
            self._conn.execute("""
                INSERT INTO execution_states VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """, [
                state.job_id, state.workflow_name, status_str, state.route, state.method,
                state.current_step_id, state.current_index, state.total_steps,
                data_json, output_json, state.error, state.started_at, state.finished_at
            ])

            self._conn.execute("DELETE FROM step_traces WHERE job_id = ?", [state.job_id])
            for t in state.traces:
                self._conn.execute("""
                    INSERT INTO step_traces VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """, [
                    state.job_id, t.step_id, t.activity_name, t.status,
                    json.dumps(t.input_payload or {}),
                    json.dumps(t.output_payload or {}),
                    t.error_message, t.started_at, t.finished_at, t.duration_ms
                ])

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        async with self._lock:
            if not self._available or self._conn is None:
                return self._fallback_states.get(job_id)

            res = self._conn.execute("""
                SELECT job_id, workflow_name, status, route, method,
                       current_step_id, current_index, total_steps,
                       data, output_data, error, started_at, finished_at
                FROM execution_states WHERE job_id = ?
            """, [job_id]).fetchone()

            if not res:
                return None

            trace_rows = self._conn.execute("""
                SELECT step_id, activity_name, status, input_payload,
                       output_payload, error_message, started_at, finished_at, duration_ms
                FROM step_traces WHERE job_id = ?
            """, [job_id]).fetchall()

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

            status_val = res[2]
            try:
                status_enum = ExecutionStatus(status_val)
            except Exception:
                status_enum = ExecutionStatus.COMPLETED if status_val == "COMPLETED" else ExecutionStatus.FAILED

            return ExecutionState(
                job_id=res[0],
                workflow_name=res[1],
                status=status_enum,
                route=res[3],
                method=res[4],
                current_step_id=res[5],
                current_index=int(res[6] or 0),
                total_steps=int(res[7] or 0),
                data=json.loads(res[8]) if res[8] else {},
                output_data=json.loads(res[9]) if res[9] else None,
                error=res[10],
                started_at=res[11],
                finished_at=res[12],
                traces=traces
            )

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        async with self._lock:
            if not self._available or self._conn is None:
                res_states = list(self._fallback_states.values())
                if workflow_name:
                    res_states = [s for s in res_states if s.workflow_name == workflow_name]
                res_states.sort(key=lambda s: s.started_at or "", reverse=True)
                return res_states[:limit]

            if workflow_name:
                rows = self._conn.execute("""
                    SELECT job_id FROM execution_states WHERE workflow_name = ? ORDER BY started_at DESC LIMIT ?
                """, [workflow_name, limit]).fetchall()
            else:
                rows = self._conn.execute("""
                    SELECT job_id FROM execution_states ORDER BY started_at DESC LIMIT ?
                """, [limit]).fetchall()

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
