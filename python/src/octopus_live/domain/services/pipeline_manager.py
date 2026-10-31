import asyncio
import collections
import dataclasses
import inspect
import logging
import time
import traceback
import uuid
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, Tuple
from ..models.models import ApiEndpointConfig, ExecutionState, ExecutionStatus, MiddlewareExceptionConfig, StepExecutionTrace, WorkflowBlueprint, WorkflowStep
from ..ports.outbound import IActivityExecutor, IStateStore
from .condition_evaluator import ConditionEvaluator
from .idempotency_manager import IdempotencyManager
from .masking_service import mask_payload
from .middleware import OctopusMiddlewareException
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
        self._replays = IdempotencyManager()
        # job_id|step_id -> (status, output, error, finished_at) of async steps
        self._async_outcomes: "collections.OrderedDict[str, Tuple[str, Dict[str, Any], Optional[str], str]]" = collections.OrderedDict()
        self._background: set = set()

    def _snapshot(self, state: ExecutionState, blueprint: Optional[WorkflowBlueprint]) -> ExecutionState:
        """A redacted copy of the state for storage, events and callers: it never
        shares memory with the running execution, and every field masked anywhere
        in the blueprint is hidden (decision D5). Async outcomes are applied."""
        fields, mask_val = blueprint.all_mask_fields() if blueprint else ([], "********")
        traces = []
        for t in state.traces:
            t = dataclasses.replace(
                t,
                input_payload=mask_payload(t.input_payload, fields, mask_val) or {},
                output_payload=mask_payload(t.output_payload, fields, mask_val) or {},
            )
            outcome = self._async_outcomes.get(f"{state.job_id}|{t.step_id}") if t.status == "ASYNC_DISPATCHED" else None
            if outcome:
                status, output, error, finished_at = outcome
                t.status, t.error_message, t.finished_at = status, error, finished_at
                if error is None:
                    t.output_payload = mask_payload(output, fields, mask_val) or {}
            traces.append(t)
        return dataclasses.replace(
            state,
            data=mask_payload(state.data, fields, mask_val) or {},
            output_data=mask_payload(state.output_data, fields, mask_val),
            traces=traces,
            route_output=None,
        )

    def _returned(self, state: ExecutionState, blueprint: Optional[WorkflowBlueprint]) -> ExecutionState:
        """What execute hands back: masked like the stored job (decision D5),
        plus the real output for a declared route's response. Activities always
        receive the real values."""
        cp = self._snapshot(state, blueprint)
        cp.route_output = dict(state.output_data) if state.output_data is not None else None
        return cp

    async def _save(self, state: ExecutionState, blueprint: Optional[WorkflowBlueprint]) -> None:
        await self.state_store.save_state(self._snapshot(state, blueprint))

    def _spawn(self, coro) -> None:
        # Keep a reference so the task is not garbage-collected mid-flight.
        task = asyncio.create_task(coro)
        self._background.add(task)
        task.add_done_callback(self._background.discard)

    def _record_async_outcome(self, job_id: str, step_id: str, output: Optional[Dict[str, Any]], error: Optional[Exception]) -> None:
        key = f"{job_id}|{step_id}"
        status = "ASYNC_FAILED" if error is not None else "ASYNC_COMPLETED"
        self._async_outcomes[key] = (status, output or {}, str(error) if error is not None else None, datetime.now(timezone.utc).isoformat())
        self._async_outcomes.move_to_end(key)
        while len(self._async_outcomes) > 2000:
            self._async_outcomes.popitem(last=False)

    async def _run_with_retries(self, step: WorkflowStep, activity_name: str, payload: Dict[str, Any]) -> Tuple[Optional[Dict[str, Any]], Optional[Exception], int]:
        """Runs an activity under the step's retry_policy (one attempt by
        default). Returns (output, error, attempts)."""
        policy = step.retry_policy or {}
        max_attempts = max(1, int(policy.get("max_attempts") or 1))
        delay = float(policy.get("initial_interval_ms") or 200) / 1000.0
        coefficient = float(policy.get("backoff_coefficient") or 2)
        coefficient = coefficient if coefficient >= 1 else 2.0
        max_delay = float(policy.get("maximum_interval_ms") or 0) / 1000.0
        attempt = 0
        while True:
            attempt += 1
            try:
                return await self._call_activity(step, activity_name, payload), None, attempt
            except Exception as ex:
                if attempt >= max_attempts or not _retryable(ex):
                    return None, ex, attempt
            await asyncio.sleep(delay)
            delay *= coefficient
            if max_delay > 0:
                delay = min(delay, max_delay)

    async def _call_activity(self, step: WorkflowStep, activity_name: str, payload: Dict[str, Any]) -> Any:
        """One attempt, bounded by the step's `timeout_seconds` when it declares
        one (decision D17). A plain function then runs in a worker thread, so
        the wait can end while it runs on; a coroutine is cancelled."""
        timeout = step.timeout_seconds or 0
        if timeout <= 0:
            return await self.activity_executor.execute_activity(activity_name, payload)
        acts = getattr(self.activity_executor, "_activities", None)
        fn = None
        if isinstance(acts, dict):
            fn = acts.get(activity_name) or acts.get(self.activity_executor._to_snake_case(activity_name))
        if fn is not None and not inspect.iscoroutinefunction(fn):
            call = asyncio.to_thread(fn, payload)
        else:
            fn = None
            call = self.activity_executor.execute_activity(activity_name, payload)
        try:
            result = await asyncio.wait_for(call, timeout)
        except asyncio.TimeoutError:
            raise ActivityTimeout(f"Activity '{activity_name}' did not finish within {timeout}s.") from None
        if fn is not None and not isinstance(result, dict):
            return {"result": result}
        return result

    async def _publish_state(self, state: ExecutionState, blueprint: Optional[WorkflowBlueprint], event: str) -> None:
        """A workflow event with the job's masked state (decision D15)."""
        snap = dataclasses.asdict(self._snapshot(state, blueprint))
        snap["status"] = state.status.value
        await self.state_store.publish_event(state.job_id, event, snap)

    @staticmethod
    def _pending_compensations(traces: List[StepExecutionTrace]) -> List[StepExecutionTrace]:
        """Traces whose compensation would run on failure: completed steps
        after the last pivot (decisions D9 and D10). A step that failed is not
        compensated."""
        last_pivot = max((i for i, t in enumerate(traces) if t.activity_name == "pivot" and t.output_payload.get("compensations_cleared")), default=-1)
        return [t for t in traces[last_pivot + 1:] if t.status == "COMPLETED" and t.input_payload.get("_compensation_activity")]

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
        job_id = payload.get("job_id") or f"job_{uuid.uuid4()}"
        exec_start_time = time.perf_counter()
        
        # Determine step sequence: endpoint-specific steps take precedence if present
        steps_to_run = (endpoint.steps if endpoint and endpoint.steps else blueprint.steps)
        req_route = route or (endpoint.route if endpoint else None)
        req_method = method or (endpoint.method if endpoint else None)
        client_host = str(payload.get("_client_host") or payload.get("client_ip") or payload.get("client_host") or "127.0.0.1")

        # Sending the same job_id again replays the first run's result; the
        # steps do not run again (decision D6). Keys are scoped by workflow and
        # route, so a reused id never replays another workflow's response.
        replay_key = f"{blueprint.workflow_name}|{(req_method or '').upper()}|{req_route or ''}|{job_id}"
        cached = self._replays.get(replay_key)
        if cached is not None:
            return self._returned(cached, blueprint)

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
        await self._save(state, blueprint)
        await self._publish_state(state, blueprint, "workflow_started")

        try:
            terminated = await self._run_step_sequence(steps_to_run, state, blueprint)
            if state.status == ExecutionStatus.RUNNING:
                state.status = ExecutionStatus.COMPLETED
            state.finished_at = datetime.now(timezone.utc).isoformat()
            state.duration_ms = round((time.perf_counter() - exec_start_time) * 1000.0, 2)
            await self._publish_state(state, blueprint, "workflow_completed")
        except Exception as ex:
            logger.error("💥 Pipeline execution failed for job '%s': %s", job_id, ex, exc_info=True)
            state.status = ExecutionStatus.FAILED
            state.error_type = ex.__class__.__name__
            state.finished_at = datetime.now(timezone.utc).isoformat()
            state.duration_ms = round((time.perf_counter() - exec_start_time) * 1000.0, 2)

            # Check if any middleware or global handler declared an exception policy for this error
            exc_cfg = self._find_blueprint_exception_config(blueprint, ex, state.current_step_id)
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
                        self._spawn(self._run_exception_activity_safe(exc_cfg.activity, act_payload))
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

            pending = self._pending_compensations(state.traces)
            if pending:
                if blueprint.compensation_mode == "async":
                    logger.info("⚡ Dispatching asynchronous Saga rollback in background...")
                    self._spawn(SagaManager.rollback(pending, self.activity_executor, dict(state.data)))
                    state.status = ExecutionStatus.ROLLED_BACK
                    await self._publish_state(state, blueprint, "workflow_rolled_back")
                else:
                    state.data = await SagaManager.rollback(pending, self.activity_executor, state.data)
                    state.status = ExecutionStatus.ROLLED_BACK
                    await self._publish_state(state, blueprint, "workflow_rolled_back")
            else:
                await self._publish_state(state, blueprint, "workflow_failed")

        if state.status == ExecutionStatus.COMPLETED:
            self._replays.put(replay_key, dataclasses.replace(state, data={"job_id": job_id}, output_data=dict(state.output_data) if state.output_data is not None else None, traces=[]))
        await self._save(state, blueprint)

        # Trigger non-blocking asynchronous durable replication (Temporal, DuckDB, Redis, SQLite)
        if self.replication_manager:
            self.replication_manager.dispatch_replication(self._snapshot(state, blueprint))

        return self._returned(state, blueprint)


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
            await self._save(state, blueprint)

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
                    await self._execute_activity_step(step, state, blueprint, idx)

            # A pivot is the point of no return: once it has run, earlier steps
            # are no longer compensated if a later step fails (decision D9).
            if step.pivot and self._pending_compensations(state.traces):
                state.traces.append(StepExecutionTrace(
                    step_id=f"{step.id}.pivot",
                    activity_name="pivot",
                    output_payload={"compensations_cleared": True}
                ))

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

    async def _execute_activity_step(self, step: WorkflowStep, state: ExecutionState, blueprint: Optional[WorkflowBlueprint] = None, index: int = 0) -> None:
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

        await self.state_store.publish_event(state.job_id, "step_started", {
            "step_id": step.id,
            "activity": activity_name,
            "index": index,
            "total_steps": state.total_steps,
        })
        output, ex, trace.attempts = await self._run_with_retries(step, activity_name, state.data)
        trace.duration_ms = round((time.perf_counter() - start_time) * 1000.0, 2)
        trace.finished_at = datetime.now(timezone.utc).isoformat()

        if ex is None:
            raw_out = output if isinstance(output, dict) else {"result": output}
            trace.output_payload = self._mask_payload(raw_out, blueprint, step) or {}
            trace.status = "COMPLETED"

            if isinstance(output, dict):
                state.data.update(output)

            if step.is_output or state.output_data is None:
                state.output_data = raw_out

            state.traces.append(trace)
            await self.state_store.publish_event(state.job_id, "step_completed", dataclasses.asdict(trace))
            return

        trace.status = "FAILED"
        trace.error_message = str(ex)
        state.traces.append(trace)

        # A declared step-level handler runs here, and leaves a trace so the
        # workflow-level handling in execute() does not run it again.
        matching_cfg = next((ec for ec in step.exceptions or [] if exception_matches(ec, ex)), None)
        if matching_cfg and matching_cfg.activity:
            exc_name = ex.__class__.__name__
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
            exc_trace = StepExecutionTrace(
                step_id=f"exc_{matching_cfg.activity.lower()}",
                activity_name=matching_cfg.activity,
                input_payload=self._mask_payload(exc_payload, blueprint, step) or {},
            )
            if matching_cfg.is_async:
                exc_trace.status, exc_trace.is_async = "DISPATCHED_ASYNC", True
                exc_trace.output_payload = {"status": "async_dispatched"}
                self._spawn(self._run_exception_activity_safe(matching_cfg.activity, exc_payload))
            else:
                exc_start = time.perf_counter()
                try:
                    exc_out = await self.activity_executor.execute_activity(matching_cfg.activity, exc_payload)
                    exc_trace.output_payload = self._mask_payload(exc_out if isinstance(exc_out, dict) else {"result": exc_out}, blueprint, step) or {}
                except Exception as act_err:
                    exc_trace.status, exc_trace.error_message = "FAILED", str(act_err)
                    logger.error("Sync exception handler activity '%s' failed: %s", matching_cfg.activity, act_err)
                exc_trace.duration_ms = round((time.perf_counter() - exc_start) * 1000.0, 2)
                exc_trace.finished_at = datetime.now(timezone.utc).isoformat()
            state.traces.append(exc_trace)
        raise ex

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

        # Dispatch without awaiting; the stored job records the outcome as
        # ASYNC_COMPLETED or ASYNC_FAILED (decision D7).
        payload_copy = dict(state.data)
        async def _run_bg():
            output, bg_err, _ = await self._run_with_retries(step, activity_name, payload_copy)
            if bg_err is not None:
                logger.error("Async background activity '%s' failed: %s", activity_name, bg_err)
            self._record_async_outcome(state.job_id, step.id, output, bg_err)
            await self._save(state, blueprint)

        self._spawn(_run_bg())
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

    def _find_blueprint_exception_config(self, blueprint: WorkflowBlueprint, exc: Exception, failing_step_id: Optional[str] = None) -> Optional[MiddlewareExceptionConfig]:
        # 1. Declared step-level exceptions, the failing step's first
        all_steps = list(blueprint.steps)
        for ep in blueprint.api:
            all_steps.extend(ep.steps)
        all_steps.sort(key=lambda st: st.id != failing_step_id)
        for st in all_steps:
            for ec in getattr(st, "exceptions", []) or []:
                if exception_matches(ec, exc):
                    return ec

        # 2. Declared middlewares
        for mw in blueprint.middlewares:
            for ec in mw.exceptions:
                if exception_matches(ec, exc):
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



def exception_matches(ec: MiddlewareExceptionConfig, exc: Exception) -> bool:
    """An exceptions entry matches the error's type name (or a base class's)
    or a case-insensitive substring of its message (decision D8)."""
    name = (ec.exception or "").strip()
    if not name:
        return False
    if any(base.__name__.lower() == name.lower() for base in type(exc).__mro__):
        return True
    return name.lower() in str(exc).lower()


class ActivityTimeout(OctopusMiddlewareException):
    """An activity ran past its step's `timeout_seconds` (decision D17)."""
    default_code = 504
    default_message = "Activity timed out"


def _retryable(exc: Exception) -> bool:
    """Client errors (a 4xx code) are not retried."""
    code = getattr(exc, "code", None) or getattr(exc, "status_code", None)
    return not (isinstance(code, int) and 400 <= code < 500)
