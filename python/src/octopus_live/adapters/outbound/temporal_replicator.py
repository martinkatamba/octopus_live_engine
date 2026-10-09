import asyncio
from datetime import timedelta
import inspect
import logging
import os
import sys
import time
from typing import Any, Callable, Dict, List, Optional, Set
from ...domain.models.models import ExecutionState
from ...domain.ports.outbound import IActivityExecutor

logger = logging.getLogger("OctopusLive.TemporalReplicator")

# Attempt lazy module-level workflow definition for Temporal SDK
_TEMPORAL_AVAILABLE = False
try:
    from temporalio import activity, workflow
    from temporalio.client import Client
    from temporalio.common import RetryPolicy
    from temporalio.worker import UnsandboxedWorkflowRunner, Worker

    async def _run_workflow_logic(payload: Dict[str, Any]) -> Dict[str, Any]:
        # 1. Single discrete activity execution
        act_name = payload.get("activity_name")
        if act_name:
            res = await workflow.execute_activity(
                act_name,
                payload,
                start_to_close_timeout=timedelta(seconds=60),
                retry_policy=RetryPolicy(
                    initial_interval=timedelta(seconds=1),
                    maximum_interval=timedelta(seconds=10),
                    backoff_coefficient=2.0,
                    maximum_attempts=3
                )
            )
            return res if isinstance(res, dict) else {"result": res}

        # 2. Multi-trace execution
        traces = payload.get("traces", [])
        for t in traces:
            t_act = t.get("activity_name") or t.get("step_id")
            if t_act:
                act_payload = t.get("input_payload") or t.get("output_payload") or payload
                await workflow.execute_activity(
                    t_act,
                    act_payload,
                    start_to_close_timeout=timedelta(seconds=60),
                    retry_policy=RetryPolicy(
                        initial_interval=timedelta(seconds=1),
                        maximum_interval=timedelta(seconds=10),
                        backoff_coefficient=2.0,
                        maximum_attempts=3
                    )
                )
        return payload.get("output_data") or payload

    @workflow.defn(name="IOctopusWorkflow")
    class IOctopusWorkflow:
        """Dynamic single-activity or pipeline workflow orchestrator."""
        @workflow.run
        async def run(self, payload: Dict[str, Any]) -> Dict[str, Any]:
            return await _run_workflow_logic(payload)

    @workflow.defn(name="OctopusReplicatedWorkflow")
    class OctopusReplicatedWorkflow:
        """Backward-compatible alias for OctopusReplicatedWorkflow."""
        @workflow.run
        async def run(self, payload: Dict[str, Any]) -> Dict[str, Any]:
            return await _run_workflow_logic(payload)

    _TEMPORAL_AVAILABLE = True
except ImportError:
    IOctopusWorkflow = None
    OctopusReplicatedWorkflow = None


def _create_activity_handler(activity_name: str, executor: Optional[IActivityExecutor] = None) -> Callable:
    """Create a named Temporal activity handler that delegates execution to the engine's IActivityExecutor."""
    from temporalio import activity

    @activity.defn(name=activity_name)
    async def _handler(payload: Dict[str, Any]) -> Dict[str, Any]:
        if executor is not None:
            try:
                if hasattr(executor, "execute_activity"):
                    res = await executor.execute_activity(activity_name, payload)
                elif hasattr(executor, "execute_async"):
                    res = await executor.execute_async(activity_name, payload)
                elif hasattr(executor, "execute"):
                    res = executor.execute(activity_name, payload)
                    if inspect.isawaitable(res):
                        res = await res
                elif callable(executor):
                    res = executor(activity_name, payload)
                    if inspect.isawaitable(res):
                        res = await res
                else:
                    res = payload
                if isinstance(res, dict):
                    return res
                return {"result": res} if res is not None else {}
            except Exception as ex:
                logger.error("❌ [Temporal Activity] '%s' execution failed: %s", activity_name, ex)
                raise ex
        return payload

    return _handler


class TemporalReplicator:
    """Zero-Import Temporal.io Durability Replicator & Embedded SDK Worker.
    
    Asynchronously executes or mirrors live activities and execution traces
    into Temporal.io for durable auditability, replay, visualizer inspection, and worker crash resilience.
    Each individual activity step is executed as a discrete ActivityTask on Temporal,
    providing distinct activity blocks on the Temporal Web UI timeline and transitioning
    workflows to 'Completed' (green badge).
    """

    def __init__(
        self,
        host: str = "127.0.0.1:7233",
        namespace: str = "default",
        task_queue: str = "octopus-hybrid",
        api_key: Optional[str] = None,
        tls: bool = False,
        activity_executor: Optional[IActivityExecutor] = None
    ):
        self.host = host
        self.namespace = namespace
        self.task_queue = task_queue
        self.api_key = api_key
        self.tls = tls
        self.activity_executor = activity_executor
        self._client: Optional[Any] = None
        self._worker: Optional[Any] = None
        self._worker_task: Optional[asyncio.Task] = None
        self._connected: bool = False
        self._registered_activities: Dict[str, Callable] = {}
        self._replicated_jobs: list[str] = []
        self._lock = asyncio.Lock()
        self._active_activities_count: Optional[int] = None

    def set_activity_executor(self, executor: IActivityExecutor) -> None:
        self.activity_executor = executor

    def register_activity_names(self, names: List[str]) -> None:
        """Pre-register activity names for Temporal worker registration."""
        if not _TEMPORAL_AVAILABLE:
            return
        for name in names:
            if name and (name not in self._registered_activities or self._registered_activities[name] is None):
                self._registered_activities[name] = _create_activity_handler(name, self.activity_executor)

    async def connect(self) -> bool:
        if not _TEMPORAL_AVAILABLE:
            logger.debug("temporalio package is not installed. Temporal replication running in simulation mode.")
            return False

        if self._connected and self._client is not None:
            return True

        async with self._lock:
            if self._connected and self._client is not None:
                return True
            try:
                self._client = await Client.connect(
                    self.host,
                    namespace=self.namespace,
                    api_key=self.api_key,
                    tls=self.tls
                )
                self._connected = True
                logger.info("🛡️ Temporal SDK Client connected -> Host: %s, Namespace: %s, TaskQueue: %s", self.host, self.namespace, self.task_queue)
                await self.ensure_worker()
                return True
            except Exception as ex:
                logger.warning("Could not connect to Temporal at %s: %s", self.host, ex)
                return False

    async def ensure_worker(self) -> None:
        """Starts or refreshes the background Temporal SDK worker."""
        if not self._connected or self._client is None or IOctopusWorkflow is None:
            return

        from temporalio.worker import UnsandboxedWorkflowRunner, Worker

        # Refresh handlers with latest activity_executor
        if self.activity_executor and hasattr(self.activity_executor, "_activities"):
            for name in self.activity_executor._activities.keys():
                if name not in self._registered_activities:
                    self._registered_activities[name] = _create_activity_handler(name, self.activity_executor)

        activities_list = list(self._registered_activities.values())
        if self._worker is not None:
            if self._active_activities_count == len(activities_list):
                return
            # Restart worker to register new activities
            try:
                await self._worker.shutdown()
                if self._worker_task:
                    await asyncio.wait_for(self._worker_task, timeout=2.0)
            except Exception:
                pass
            self._worker = None

        self._active_activities_count = len(activities_list)
        self._worker = Worker(
            self._client,
            task_queue=self.task_queue,
            workflows=[IOctopusWorkflow, OctopusReplicatedWorkflow],
            activities=activities_list,
            workflow_runner=UnsandboxedWorkflowRunner()
        )
        self._worker_task = asyncio.create_task(self._worker.run())
        logger.info(
            "👷 [TEMPORAL WORKER] Worker actively polling TaskQueue '%s' on Temporal Server (%s) with %d activities",
            self.task_queue,
            self.host,
            len(activities_list)
        )

    async def run_worker_forever(self) -> None:
        """Run standalone worker loop indefinitely for dedicated worker processes."""
        conn_ok = await self.connect()
        if not conn_ok or not self._client:
            raise RuntimeError(f"Failed to connect to Temporal server at {self.host}")

        await self.ensure_worker()
        if self._worker_task:
            await self._worker_task

    async def dispatch_activity(
        self,
        activity_name: str,
        payload: Dict[str, Any],
        workflow_id: Optional[str] = None
    ) -> bool:
        """Dispatches a single discrete activity execution directly as an IOctopusWorkflow."""
        if not activity_name:
            return False

        if activity_name not in self._registered_activities and _TEMPORAL_AVAILABLE:
            self._registered_activities[activity_name] = _create_activity_handler(activity_name, self.activity_executor)

        conn_ok = await self.connect()
        if not conn_ok or self._client is None or IOctopusWorkflow is None:
            return False

        await self.ensure_worker()

        try:
            job_id = payload.get("job_id") or f"req_{int(time.time() * 1000)}_{os.urandom(3).hex()}"
            wf_id = workflow_id or f"octopus_act_{job_id}_{activity_name}"
            
            call_payload = dict(payload)
            call_payload["activity_name"] = activity_name
            call_payload["job_id"] = job_id
            call_payload["temporal_task_queue"] = self.task_queue
            call_payload["temporal_workflow_id"] = wf_id

            logger.info("⏳ [TEMPORAL DISPATCH] Queuing durable activity '%s' -> WorkflowID: %s", activity_name, wf_id)
            await self._client.start_workflow(
                IOctopusWorkflow.run,
                call_payload,
                id=wf_id,
                task_queue=self.task_queue
            )
            logger.info("📡 [TEMPORAL DISPATCH] Dispatched workflow '%s' to Temporal Server", wf_id)
            return True
        except Exception as ex:
            logger.warning("Temporal single activity dispatch failed for '%s': %s", activity_name, ex)
            return False

    async def replicate(self, state: ExecutionState) -> bool:
        """Asynchronously replicates an execution state trace into Temporal without blocking fast-path."""
        self._replicated_jobs.append(state.job_id)

        # Extract asynchronous / durable activities for Temporal offload
        async_traces = [
            t for t in state.traces
            if getattr(t, "is_async", False) or t.status == "ASYNC_DISPATCHED" or t.step_id.startswith("exc_")
        ]

        if not async_traces:
            logger.info("⏭️ [Temporal Replicator] Fast-path job '%s' has 0 async activities - Skipping Temporal task creation.", state.job_id)
            return True

        # Register any new activity names
        for t in async_traces:
            act_name = t.activity_name or t.step_id
            if act_name and act_name not in self._registered_activities and _TEMPORAL_AVAILABLE:
                self._registered_activities[act_name] = _create_activity_handler(act_name, self.activity_executor)

        conn_ok = await self.connect()
        if not conn_ok or self._client is None or IOctopusWorkflow is None:
            return False

        await self.ensure_worker()

        try:
            payload = {
                "job_id": state.job_id,
                "workflow_name": state.workflow_name,
                "status": state.status.value if hasattr(state.status, "value") else str(state.status),
                "route": state.route,
                "method": state.method,
                "data": state.data,
                "output_data": state.output_data,
                "error": state.error,
                "started_at": state.started_at,
                "finished_at": state.finished_at,
                "traces": [
                    {
                        "step_id": t.step_id,
                        "activity_name": t.activity_name or t.step_id,
                        "status": t.status,
                        "duration_ms": t.duration_ms,
                        "input_payload": t.input_payload,
                        "output_payload": t.output_payload,
                        "error_message": t.error_message,
                        "started_at": t.started_at,
                        "finished_at": t.finished_at
                    }
                    for t in async_traces
                ]
            }

            workflow_id = f"wf-{state.workflow_name}-{state.job_id}"
            await self._client.start_workflow(
                IOctopusWorkflow.run,
                payload,
                id=workflow_id,
                task_queue=self.task_queue
            )
            logger.info("🛡️ [Temporal Replicator] Replicated job '%s' with %d async activities to Temporal (WorkflowID=%s)", state.job_id, len(async_traces), workflow_id)
            return True
        except Exception as ex:
            logger.warning("Temporal replication dispatch failed for job '%s': %s", state.job_id, ex)
            return False

    async def shutdown(self) -> None:
        if self._worker:
            try:
                await self._worker.shutdown()
            except Exception:
                pass
        if self._worker_task and not self._worker_task.done():
            self._worker_task.cancel()
            try:
                await self._worker_task
            except asyncio.CancelledError:
                pass
