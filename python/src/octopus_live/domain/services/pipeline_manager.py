import asyncio
import logging
import time
import traceback
import uuid
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional
from ..models.models import ApiEndpointConfig, ExecutionState, ExecutionStatus, MiddlewareExceptionConfig, StepExecutionTrace, WorkflowBlueprint, WorkflowStep
from ..ports.outbound import IActivityExecutor, IStateStore
from .condition_evaluator import ConditionEvaluator
from .masking_service import mask_payload
from .saga_manager import SagaManager

logger = logging.getLogger("OctopusLive.PipelineManager")


class PipelineManager:
    """Core domain service for executing declarative workflow pipelines synchronously in-memory."""

    def __init__(
        self,
        activity_executor: IActivityExecutor,
        state_store: IStateStore,
        replication_manager: Optional[Any] = None
    ):
        self.activity_executor = activity_executor
        self.state_store = state_store
        self.replication_manager = replication_manager

    def _mask_payload(
        self,
        payload: Optional[Dict[str, Any]],
        blueprint: Optional[WorkflowBlueprint],
        step: Optional[WorkflowStep] = None
    ) -> Optional[Dict[str, Any]]:
        if not payload:
            return payload
        if not blueprint:
            return dict(payload)
        fields, mask_val = blueprint.get_mask_config(step)
        return mask_payload(payload, fields, mask_val)

    async def execute(
        self,
        blueprint: WorkflowBlueprint,
        payload: Dict[str, Any],
        endpoint: Optional[ApiEndpointConfig] = None,
        route: Optional[str] = None,
        method: Optional[str] = None
    ) -> ExecutionState:
        job_id = payload.get("job_id") or f"live-{uuid.uuid4().hex[:12]}"
        exec_start_time = time.perf_counter()
        
        # Determine step sequence: endpoint-specific steps take precedence if present
        steps_to_run = (endpoint.steps if endpoint and endpoint.steps else blueprint.steps)
        req_route = route or (endpoint.route if endpoint else None)
        req_method = method or (endpoint.method if endpoint else None)
        client_host = str(payload.get("_client_host") or payload.get("client_ip") or payload.get("client_host") or "127.0.0.1")

        state = ExecutionState(
            job_id=job_id,
            workflow_name=blueprint.workflow_name,
            status=ExecutionStatus.RUNNING,
            route=req_route,
            method=req_method,
            total_steps=len(steps_to_run),
            data=dict(payload),
            client_host=client_host,
            started_at=datetime.now(timezone.utc).isoformat()
        )
        state.data["job_id"] = job_id
        await self.state_store.save_state(state)
        await self.state_store.publish_event(job_id, "workflow_started", {"status": "RUNNING", "job_id": job_id, "route": req_route, "method": req_method})

        try:
            terminated = await self._run_step_sequence(steps_to_run, state, blueprint)
            if state.status == ExecutionStatus.RUNNING:
                state.status = ExecutionStatus.COMPLETED
            state.finished_at = datetime.now(timezone.utc).isoformat()
            state.duration_ms = round((time.perf_counter() - exec_start_time) * 1000.0, 2)
            await self.state_store.publish_event(job_id, "workflow_completed", {"status": state.status.value, "data": state.data, "job_id": job_id})
        except Exception as ex:
            logger.error("💥 Pipeline execution failed for job '%s': %s", job_id, ex, exc_info=True)
            state.status = ExecutionStatus.FAILED
            state.error_type = ex.__class__.__name__
            state.finished_at = datetime.now(timezone.utc).isoformat()
            state.duration_ms = round((time.perf_counter() - exec_start_time) * 1000.0, 2)

            # Check if any middleware or global handler declared an exception policy for this error
            exc_cfg = self._find_blueprint_exception_config(blueprint, ex)
            if exc_cfg:
                state.error = exc_cfg.message or getattr(ex, "message", str(ex))
                state.error_code = exc_cfg.code or getattr(ex, "code", getattr(ex, "status_code", 500 if exc_cfg.exception == "*" else 400))
                already_executed = any(t.activity_name == exc_cfg.activity for t in state.traces)
                if exc_cfg.activity and self.activity_executor and not already_executed:
                    formatted_tb = traceback.format_exc()
                    act_payload = {
                        "exception": state.error_type,
                        "error": state.error_type,
                        "error_type": state.error_type,
                        "error_message": getattr(ex, "message", str(ex)),
                        "message": state.error,
                        "status_code": state.error_code,
                        "job_id": job_id,
                        "workflow_name": blueprint.workflow_name,
                        "failing_step_id": state.current_step_id or "",
                        "route": state.route or state.data.get("route", ""),
                        "path": state.route or state.data.get("route", ""),
                        "method": state.method or state.data.get("method", "POST"),
                        "client_host": client_host,
                        "stack_trace": formatted_tb,
                        "traceback": formatted_tb,
                        "timestamp": datetime.now(timezone.utc).isoformat(),
                        "data": dict(state.data),
                    }
                    if exc_cfg.is_async:
                        asyncio.create_task(self._run_exception_activity_safe(exc_cfg.activity, act_payload))
                        trace = StepExecutionTrace(
                            step_id=f"exc_{exc_cfg.activity.lower()}",
                            activity_name=exc_cfg.activity,
                            status="DISPATCHED_ASYNC",
                            is_async=True,
                            input_payload=self._mask_payload(act_payload, blueprint),
                            output_payload={"status": "async_dispatched"},
                            duration_ms=0.1
                        )
                        state.traces.append(trace)
                    else:
                        start_act = time.perf_counter()
                        try:
                            act_res = await self.activity_executor.execute_activity(exc_cfg.activity, act_payload)
                            dur_ms = round((time.perf_counter() - start_act) * 1000.0, 3)
                            trace = StepExecutionTrace(
                                step_id=f"exc_{exc_cfg.activity.lower()}",
                                activity_name=exc_cfg.activity,
                                status="COMPLETED",
                                is_async=False,
                                input_payload=self._mask_payload(act_payload, blueprint),
                                output_payload=self._mask_payload(act_res if isinstance(act_res, dict) else {"result": act_res}, blueprint),
                                duration_ms=dur_ms
                            )
                            state.traces.append(trace)
                        except Exception as act_err:
                            logger.warning("Exception activity '%s' execution failed: %s", exc_cfg.activity, act_err)
            else:
                state.error = getattr(ex, "message", str(ex))
                state.error_code = getattr(ex, "code", getattr(ex, "status_code", None))

            has_compensations = any(t.input_payload.get("_compensation_activity") for t in state.traces)
            if has_compensations:
                if blueprint.compensation_mode == "async":
                    logger.info("⚡ Dispatching asynchronous Saga rollback in background...")
                    asyncio.create_task(SagaManager.rollback(list(state.traces), self.activity_executor, dict(state.data)))
                    state.status = ExecutionStatus.ROLLED_BACK
                    await self.state_store.publish_event(job_id, "workflow_rolled_back", {"status": "ROLLED_BACK", "mode": "async", "error": str(ex)})
                else:
                    state.data = await SagaManager.rollback(state.traces, self.activity_executor, state.data)
                    state.status = ExecutionStatus.ROLLED_BACK
                    await self.state_store.publish_event(job_id, "workflow_rolled_back", {"status": "ROLLED_BACK", "mode": "sync", "error": str(ex)})
            else:
                await self.state_store.publish_event(job_id, "workflow_failed", {"status": "FAILED", "error": str(ex)})

        await self.state_store.save_state(state)

        # Trigger non-blocking asynchronous durable replication (Temporal, DuckDB, Redis, SQLite)
        if self.replication_manager:
            self.replication_manager.dispatch_replication(state)

        return state


    async def _run_step_sequence(self, steps: List[WorkflowStep], state: ExecutionState, blueprint: WorkflowBlueprint) -> bool:
        idx = 0
        jump_target: Optional[str] = None

        while idx < len(steps):
            step = steps[idx]

            # Handle named jumps
            if jump_target:
                if step.id == jump_target:
                    jump_target = None
                else:
                    idx += 1
                    continue

            state.current_step_id = step.id
            state.current_index = idx + 1
            await self.state_store.save_state(state)

            # Pre-execution delay
            if step.delay_seconds > 0:
                await asyncio.sleep(step.delay_seconds)

            # Early termination / settlement
            if step.terminate or step.settle:
                trace = StepExecutionTrace(
                    step_id=step.id,
                    activity_name="terminate_settle",
                    status="COMPLETED",
                    output_payload={"settled": True, "reason": step.reason or "Workflow settled gracefully"}
                )
                state.traces.append(trace)
                state.status = ExecutionStatus.COMPLETED
                return True

            # Declarative compensation trigger
            if step.compensate or step.rollback or step.trigger_compensation:
                logger.warning("Declarative compensation triggered at step '%s'", step.id)
                raise RuntimeError(step.reason or f"Declarative compensation triggered at step '{step.id}'")

            # 1. Condition evaluation & Branching
            if step.condition:
                eval_start = time.perf_counter()
                cond_details = ConditionEvaluator.evaluate_detailed(step.condition, state.data)
                cond_result = cond_details["matched"]
                eval_duration = round((time.perf_counter() - eval_start) * 1000.0, 3)

                has_branches = bool(step.condition.then_steps or step.condition.else_steps)

                if has_branches or not step.activity:
                    then_first_step = step.condition.then_steps[0].id if (step.condition.then_steps and hasattr(step.condition.then_steps[0], 'id')) else "then_steps"
                    else_first_step = step.condition.else_steps[0].id if (step.condition.else_steps and hasattr(step.condition.else_steps[0], 'id')) else "else_steps"
                    branch_taken = then_first_step if cond_result else else_first_step

                    trace = StepExecutionTrace(
                        step_id=step.id,
                        activity_name=step.id,
                        status="COMPLETED",
                        input_payload=self._mask_payload(cond_details.get("evaluated_parameters", {}), blueprint, step) or {},
                        output_payload=self._mask_payload({
                            "condition_matched": cond_result,
                            "outcome": "TRUE (Match)" if cond_result else "FALSE (Else)",
                            "branch_taken": branch_taken,
                            "evaluated_rules": cond_details.get("rules", []),
                            "evaluation_details": cond_details
                        }, blueprint, step) or {},
                        duration_ms=eval_duration
                    )
                    state.traces.append(trace)

                if cond_result:
                    if step.condition.then_steps:
                        terminated = await self._run_step_sequence(step.condition.then_steps, state, blueprint)
                        if terminated:
                            return True
                else:
                    if step.condition.else_steps:
                        terminated = await self._run_step_sequence(step.condition.else_steps, state, blueprint)
                        if terminated:
                            return True
                    idx += 1
                    continue

            # 2. Loop handling
            if step.loop or step.for_each:
                await self._execute_loop_step(step, state, blueprint)
                idx += 1
                continue

            # 3. Activity Execution (Sync or Async background)
            if step.activity:
                if step.is_async:
                    await self._dispatch_async_activity(step, state, blueprint)
                else:
                    await self._execute_activity_step(step, state, blueprint)

            # 4. Jump instructions
            if step.jump_to_step_index is not None and 0 <= step.jump_to_step_index < len(steps):
                idx = step.jump_to_step_index
                continue

            if step.rewind_to_step_index is not None and 0 <= step.rewind_to_step_index < len(steps):
                idx = step.rewind_to_step_index
                continue

            if step.jump_to:
                jump_target = step.jump_to
                target_idx = next((i for i, s in enumerate(steps) if s.id == jump_target), None)
                if target_idx is not None and target_idx <= idx:
                    idx = target_idx
                    jump_target = None
                    continue

            idx += 1

        return False

    async def _execute_activity_step(self, step: WorkflowStep, state: ExecutionState, blueprint: Optional[WorkflowBlueprint] = None) -> None:
        activity_name = step.activity
        if not activity_name:
            return

        start_time = time.perf_counter()
        trace_input = dict(state.data)
        if step.compensation:
            trace_input["_compensation_activity"] = step.compensation

        trace = StepExecutionTrace(
            step_id=step.id,
            activity_name=activity_name,
            input_payload=self._mask_payload(trace_input, blueprint, step) or {},
            started_at=datetime.now(timezone.utc).isoformat()
        )

        try:
            output = await self.activity_executor.execute_activity(activity_name, state.data)
            duration_ms = (time.perf_counter() - start_time) * 1000.0

            raw_out = output if isinstance(output, dict) else {"result": output}
            trace.output_payload = self._mask_payload(raw_out, blueprint, step) or {}
            trace.duration_ms = round(duration_ms, 2)
            trace.status = "COMPLETED"
            trace.finished_at = datetime.now(timezone.utc).isoformat()

            if isinstance(output, dict):
                state.data.update(output)

            if step.is_output or state.output_data is None:
                state.output_data = output if isinstance(output, dict) else {"result": output}

            state.traces.append(trace)
            await self.state_store.publish_event(
                state.job_id,
                "step_progress",
                {
                    "step_id": step.id,
                    "activity": activity_name,
                    "status": "COMPLETED",
                    "duration_ms": trace.duration_ms,
                    "output": trace.output_payload
                }
            )
        except Exception as ex:
            duration_ms = (time.perf_counter() - start_time) * 1000.0
            trace.status = "FAILED"
            trace.error_message = str(ex)
            trace.duration_ms = round(duration_ms, 2)
            trace.finished_at = datetime.now(timezone.utc).isoformat()
            state.traces.append(trace)
            # Check for declared step-level exception handlers
            if getattr(step, "exceptions", None):
                exc_name = ex.__class__.__name__
                matching_cfg = None
                for ec in step.exceptions:
                    if ec.exception == exc_name or ec.exception.lower() == exc_name.lower():
                        matching_cfg = ec
                        break
                    for base in ex.__class__.__mro__:
                        if base.__name__ == ec.exception:
                            matching_cfg = ec
                            break
                if matching_cfg and matching_cfg.activity:
                    exc_payload = {
                        "exception": exc_name,
                        "error": exc_name,
                        "message": matching_cfg.message or str(ex),
                        "status_code": matching_cfg.code or 400,
                        "step_id": step.id,
                        "failed_activity": activity_name,
                        "job_id": state.job_id,
                        "input_data": dict(state.data)
                    }
                    if matching_cfg.is_async:
                        async def _run_exc_bg(act_name, p_copy, j_id):
                            try:
                                exc_start = time.perf_counter()
                                exc_out = await self.activity_executor.execute_activity(act_name, p_copy)
                                exc_dur = round((time.perf_counter() - exc_start) * 1000.0, 2)
                                exc_trace = StepExecutionTrace(
                                    step_id=f"exc_{act_name.lower()}",
                                    activity_name=act_name,
                                    input_payload=self._mask_payload(p_copy, blueprint, step) or {},
                                    output_payload=self._mask_payload(exc_out if isinstance(exc_out, dict) else {"result": exc_out}, blueprint, step) or {},
                                    status="COMPLETED",
                                    duration_ms=exc_dur,
                                    started_at=datetime.now(timezone.utc).isoformat(),
                                    finished_at=datetime.now(timezone.utc).isoformat()
                                )
                                state.traces.append(exc_trace)
                                await self.state_store.save_state(state)
                            except Exception as act_err:
                                logger.error("Exception handler activity '%s' failed: %s", act_name, act_err)

                        asyncio.create_task(_run_exc_bg(matching_cfg.activity, exc_payload, state.job_id))
                    else:
                        try:
                            await self.activity_executor.execute_activity(matching_cfg.activity, exc_payload)
                        except Exception as act_err:
                            logger.error("Sync exception handler activity '%s' failed: %s", matching_cfg.activity, act_err)
            raise

    async def _dispatch_async_activity(self, step: WorkflowStep, state: ExecutionState, blueprint: Optional[WorkflowBlueprint] = None) -> None:
        activity_name = step.activity
        if not activity_name:
            return

        trace = StepExecutionTrace(
            step_id=step.id,
            activity_name=activity_name,
            input_payload=self._mask_payload(dict(state.data), blueprint, step) or {},
            status="ASYNC_DISPATCHED",
            is_async=True,
            started_at=datetime.now(timezone.utc).isoformat(),
            finished_at=datetime.now(timezone.utc).isoformat(),
            output_payload=self._mask_payload({"async_dispatched": True}, blueprint, step) or {"async_dispatched": True}
        )
        state.traces.append(trace)

        # Dispatch background task without awaiting
        payload_copy = dict(state.data)
        async def _run_bg():
            try:
                await self.activity_executor.execute_activity(activity_name, payload_copy)
            except Exception as bg_err:
                logger.error("Async background activity '%s' failed: %s", activity_name, bg_err)

        asyncio.create_task(_run_bg())
        logger.info("🚀 Dispatched async non-blocking activity '%s' for step '%s'", activity_name, step.id)

    async def _execute_loop_step(self, step: WorkflowStep, state: ExecutionState, blueprint: Optional[WorkflowBlueprint] = None) -> None:
        loop_cfg = step.loop or step.for_each or {}
        items_field = loop_cfg.get("items_field") or loop_cfg.get("items") or loop_cfg.get("collection")
        items = []
        if isinstance(items_field, str) and items_field in state.data:
            val = state.data[items_field]
            if isinstance(val, list):
                items = val
        elif isinstance(items_field, list):
            items = items_field
        elif isinstance(loop_cfg.get("count"), int):
            items = list(range(loop_cfg["count"]))

        max_iter = loop_cfg.get("max_iterations", 100)
        item_var_name = loop_cfg.get("item_variable") or "item"
        logger.info("🔁 Executing loop step '%s' over %d items", step.id, len(items))
        loop_results = []

        for i, item in enumerate(items[:max_iter]):
            item_payload = dict(state.data)
            item_payload["_item"] = item
            item_payload["_index"] = i
            item_payload[item_var_name] = item

            if step.activity:
                res = await self.activity_executor.execute_activity(step.activity, item_payload)
                loop_results.append(res)
                if isinstance(res, dict):
                    state.data.update(res)

        state.data[f"{step.id}_results"] = loop_results
        trace = StepExecutionTrace(
            step_id=step.id,
            activity_name=step.activity or "loop",
            status="COMPLETED",
            input_payload=self._mask_payload(dict(state.data), blueprint, step) or {},
            output_payload=self._mask_payload({"iterations": len(loop_results), "results": loop_results}, blueprint, step) or {"iterations": len(loop_results), "results": loop_results}
        )
        state.traces.append(trace)

    def _find_blueprint_exception_config(self, blueprint: WorkflowBlueprint, exc: Exception) -> Optional[MiddlewareExceptionConfig]:
        exc_cls_name = exc.__class__.__name__
        # 1. Exact or inheritance match from declared step-level exceptions
        all_steps = list(blueprint.steps)
        for ep in blueprint.api:
            all_steps.extend(ep.steps)
        for st in all_steps:
            for ec in getattr(st, "exceptions", []) or []:
                if ec.exception == exc_cls_name or ec.exception.lower() == exc_cls_name.lower():
                    return ec
                for base in exc.__class__.__mro__:
                    if base.__name__ == ec.exception:
                        return ec

        # 2. Exact or inheritance match from declared middlewares
        for mw in blueprint.middlewares:
            for ec in mw.exceptions:
                if ec.exception == exc_cls_name or ec.exception.lower() == exc_cls_name.lower():
                    return ec
                for base in exc.__class__.__mro__:
                    if base.__name__ == ec.exception:
                        return ec

        # 3. Wildcard '*' or 'Exception' match in middlewares
        for mw in blueprint.middlewares:
            for ec in mw.exceptions:
                if ec.exception == "*" or ec.exception.lower() == "exception":
                    return ec

        # 3. Global Exception Handler (Declarative Blueprint safety net)
        gh = None
        if blueprint.error_handling and blueprint.error_handling.global_handler:
            gh = blueprint.error_handling.global_handler
        elif blueprint.global_exception_handler:
            gh = blueprint.global_exception_handler

        if gh and gh.activity:
            return MiddlewareExceptionConfig(
                exception="*",
                code=gh.code or 500,
                message=gh.message,
                activity=gh.activity,
                is_async=gh.is_async,
                headers=gh.headers
            )

        return None

    async def _run_exception_activity_safe(self, activity_name: str, payload: Dict[str, Any]) -> None:
        try:
            await self.activity_executor.execute_activity(activity_name, payload)
        except Exception as act_err:
            logger.warning("Background exception activity '%s' execution failed: %s", activity_name, act_err)

