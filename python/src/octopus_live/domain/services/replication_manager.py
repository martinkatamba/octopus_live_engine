import asyncio
import logging
from typing import Any, Dict, List, Optional
from ..models.models import ExecutionState
from ..ports.outbound import IStateStore
from ...adapters.outbound.temporal_replicator import TemporalReplicator

logger = logging.getLogger("OctopusLive.ReplicationManager")


class DurableReplicationManager:
    """Orchestrates asynchronous fan-out durable replication to Temporal and secondary state stores.
    
    Ensures synchronous fast-path API requests are returned immediately in <1ms while
    traces, states, and payloads are durably replicated in the background.
    """

    def __init__(
        self,
        durable_store: Optional[IStateStore] = None,
        temporal_replicator: Optional[TemporalReplicator] = None
    ):
        self.durable_store = durable_store
        self.temporal_replicator = temporal_replicator
        self.replicated_count: int = 0
        self._background_tasks: List[asyncio.Task] = []

    def dispatch_replication(self, state: ExecutionState) -> None:
        """Non-blocking dispatch: schedules asynchronous replication without delaying the HTTP caller."""
        if not self.durable_store and not self.temporal_replicator:
            return

        task = asyncio.create_task(self._do_replicate(state))
        self._background_tasks.append(task)
        task.add_done_callback(lambda t: self._background_tasks.remove(t) if t in self._background_tasks else None)

    async def _do_replicate(self, state: ExecutionState) -> None:
        try:
            if self.durable_store:
                await self.durable_store.save_state(state)

            if self.temporal_replicator:
                await self.temporal_replicator.replicate(state)

            self.replicated_count += 1
            logger.debug("✅ Async durability replication completed for job '%s'", state.job_id)
        except Exception as ex:
            logger.error("⚠️ Durable replication background error for job '%s': %s", state.job_id, ex)

    async def flush(self) -> None:
        """Wait for any pending background replication tasks to finish (useful for test assertions)."""
        if self._background_tasks:
            await asyncio.gather(*self._background_tasks, return_exceptions=True)
