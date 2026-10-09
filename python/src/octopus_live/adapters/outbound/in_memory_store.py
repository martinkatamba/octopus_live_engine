import asyncio
from typing import Any, AsyncIterator, Dict, List, Optional
from ...domain.models.models import ExecutionState
from ...domain.ports.outbound import IStateStore


class InMemoryStateStore(IStateStore):
    """Concurrent in-memory execution state store and SSE event distribution bus."""

    def __init__(self):
        self._states: Dict[str, ExecutionState] = {}
        self._event_queues: Dict[str, list[asyncio.Queue[Dict[str, Any]]]] = {}
        self._lock = asyncio.Lock()

    async def save_state(self, state: ExecutionState) -> None:
        async with self._lock:
            self._states[state.job_id] = state

    def get_state_sync(self, job_id: str) -> Optional[ExecutionState]:
        if job_id in self._states:
            return self._states[job_id]
        if job_id.startswith("wf-"):
            parts = job_id.split("-", 2)
            if len(parts) >= 3 and parts[2] in self._states:
                return self._states[parts[2]]
            if job_id[3:] in self._states:
                return self._states[job_id[3:]]
        return None

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        async with self._lock:
            return self.get_state_sync(job_id)

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        async with self._lock:
            states = list(self._states.values())
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
