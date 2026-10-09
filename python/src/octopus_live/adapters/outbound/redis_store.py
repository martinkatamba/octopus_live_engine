import asyncio
import json
import logging
from typing import Any, AsyncIterator, Dict, List, Optional
from ...domain.models.models import ExecutionState, ExecutionStatus, StepExecutionTrace
from ...domain.ports.outbound import IStateStore

logger = logging.getLogger("OctopusLive.RedisStore")


class RedisStateStore(IStateStore):
    """Distributed Redis state store and pub/sub bus with differential retention and local fallback."""

    def __init__(
        self,
        redis_url: str = "redis://localhost:6379",
        ttl_seconds: int = 86400,
        ttl_success_seconds: int = 3600,
        ttl_failure_seconds: int = 604800,
        ttl_running_seconds: int = 7200,
        prefix: str = "octopus:live"
    ):
        self.redis_url = redis_url
        self.ttl = ttl_seconds
        self.ttl_success = ttl_success_seconds or ttl_seconds
        self.ttl_failure = ttl_failure_seconds or ttl_seconds
        self.ttl_running = ttl_running_seconds or ttl_seconds
        self.prefix = prefix
        self._lock = asyncio.Lock()
        self._fallback_states: Dict[str, ExecutionState] = {}
        self._event_queues: Dict[str, List[asyncio.Queue[Dict[str, Any]]]] = {}
        self._client = None
        self._available = False

        try:
            import redis.asyncio as aioredis
            self._aioredis = aioredis
            self._client = aioredis.from_url(self.redis_url, decode_responses=True)
            self._available = True
        except ImportError:
            logger.warning("redis package is not installed. Falling back to in-memory state tracking.")

    def _state_key(self, job_id: str) -> str:
        return f"{self.prefix}:state:{job_id}"

    def _trace_key(self, job_id: str) -> str:
        return f"{self.prefix}:traces:{job_id}"

    def _channel_key(self, job_id: str) -> str:
        return f"{self.prefix}:events:{job_id}"

    def _compute_ttl(self, status: Any) -> int:
        status_val = (status.value if hasattr(status, "value") else str(status)).upper()
        if status_val in ("FAILED", "ERROR", "TIMEOUT", "ROLLED_BACK"):
            return self.ttl_failure
        elif status_val in ("COMPLETED", "SUCCESS"):
            return self.ttl_success
        return self.ttl_running

    async def save_state(self, state: ExecutionState) -> None:
        if not self._available or self._client is None:
            async with self._lock:
                self._fallback_states[state.job_id] = state
            return

        try:
            effective_ttl = self._compute_ttl(state.status)
            state_dict = {
                "job_id": state.job_id,
                "workflow_name": state.workflow_name,
                "status": state.status.value if hasattr(state.status, "value") else str(state.status),
                "route": state.route or "",
                "method": state.method or "",
                "current_step_id": state.current_step_id or "",
                "current_index": state.current_index,
                "total_steps": state.total_steps,
                "data": json.dumps(state.data or {}),
                "output_data": json.dumps(state.output_data) if state.output_data is not None else "",
                "error": state.error or "",
                "started_at": state.started_at,
                "finished_at": state.finished_at or ""
            }
            await self._client.hset(self._state_key(state.job_id), mapping=state_dict)
            await self._client.expire(self._state_key(state.job_id), effective_ttl)

            if state.traces:
                traces_json = json.dumps([
                    {
                        "step_id": t.step_id,
                        "activity_name": t.activity_name,
                        "status": t.status,
                        "input_payload": t.input_payload,
                        "output_payload": t.output_payload,
                        "error_message": t.error_message,
                        "started_at": t.started_at,
                        "finished_at": t.finished_at,
                        "duration_ms": t.duration_ms
                    } for t in state.traces
                ])
                await self._client.set(self._trace_key(state.job_id), traces_json, ex=effective_ttl)
        except Exception as ex:
            logger.warning("Redis save_state error: %s. Falling back to memory.", ex)
            async with self._lock:
                self._fallback_states[state.job_id] = state

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        if not self._available or self._client is None:
            async with self._lock:
                return self._fallback_states.get(job_id)

        try:
            raw = await self._client.hgetall(self._state_key(job_id))
            if not raw:
                return None

            raw_traces = await self._client.get(self._trace_key(job_id))
            traces = []
            if raw_traces:
                parsed_t = json.loads(raw_traces)
                for t in parsed_t:
                    traces.append(StepExecutionTrace(
                        step_id=t.get("step_id", ""),
                        activity_name=t.get("activity_name"),
                        status=t.get("status", "COMPLETED"),
                        input_payload=t.get("input_payload") or {},
                        output_payload=t.get("output_payload") or {},
                        error_message=t.get("error_message"),
                        started_at=t.get("started_at", ""),
                        finished_at=t.get("finished_at", ""),
                        duration_ms=float(t.get("duration_ms") or 0.0)
                    ))

            status_str = raw.get("status", "COMPLETED")
            try:
                status_enum = ExecutionStatus(status_str)
            except Exception:
                status_enum = ExecutionStatus.COMPLETED

            return ExecutionState(
                job_id=raw["job_id"],
                workflow_name=raw.get("workflow_name", ""),
                status=status_enum,
                route=raw.get("route") or None,
                method=raw.get("method") or None,
                current_step_id=raw.get("current_step_id") or None,
                current_index=int(raw.get("current_index", 0)),
                total_steps=int(raw.get("total_steps", 0)),
                data=json.loads(raw["data"]) if raw.get("data") else {},
                output_data=json.loads(raw["output_data"]) if raw.get("output_data") else None,
                error=raw.get("error") or None,
                started_at=raw.get("started_at", ""),
                finished_at=raw.get("finished_at") or None,
                traces=traces
            )
        except Exception as ex:
            logger.warning("Redis get_state connection error: %s. Reading fallback memory.", ex)
            async with self._lock:
                return self._fallback_states.get(job_id)

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        if not self._available or self._client is None:
            async with self._lock:
                states = list(self._fallback_states.values())
                if workflow_name:
                    states = [s for s in states if s.workflow_name == workflow_name]
                states.sort(key=lambda s: s.started_at or "", reverse=True)
                return states[:limit]

        try:
            keys = await self._client.keys(f"{self.prefix}:state:*")
            states = []
            for k in keys:
                job_id = k.split(":")[-1]
                st = await self.get_state(job_id)
                if st:
                    if workflow_name and st.workflow_name != workflow_name:
                        continue
                    states.append(st)
            states.sort(key=lambda s: s.started_at or "", reverse=True)
            return states[:limit]
        except Exception as ex:
            logger.warning("Redis list_states connection error: %s. Reading fallback memory.", ex)
            async with self._lock:
                states = list(self._fallback_states.values())
                if workflow_name:
                    states = [s for s in states if s.workflow_name == workflow_name]
                states.sort(key=lambda s: s.started_at or "", reverse=True)
                return states[:limit]


    async def publish_event(self, job_id: str, event_type: str, data: Dict[str, Any]) -> None:
        payload = {"event": event_type, "data": data}
        async with self._lock:
            queues = self._event_queues.get(job_id, [])
            for q in queues:
                await q.put(payload)

        if self._available and self._client:
            try:
                await self._client.publish(self._channel_key(job_id), json.dumps(payload))
            except Exception as ex:
                logger.debug("Redis publish error: %s", ex)

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
