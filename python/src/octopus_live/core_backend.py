"""Running octopus_live on the Rust engine core (`octopus_core`).

Selected with `OctopusLiveEngine(engine="core")` or `OCTOPUS_LIVE_ENGINE=core`.
The core runs the steps, keeps the jobs, publishes live events, makes the
built-in middlewares' checks (D11), draws the visualizer (D12) and talks to
Temporal (D13). The FastAPI router, the Pydantic models and the middleware
classes stay in Python.

- `CorePipelineManager` stands in for `PipelineManager`: same `execute()`,
  same `ExecutionState` back, failures returned rather than raised. In
  `mode: hybrid` async steps go to Temporal through the core (D13).
- `CoreStateStore` is the `IStateStore` over the core's jobs.
- Activities stay in the engine's `activity_executor`; the core calls back
  into it for every activity, so sync and async functions, the snake_case
  alias and the `{"result": ...}` wrapping behave as before.
"""

import asyncio
import dataclasses
import inspect
import os
import threading
from typing import Any, AsyncIterator, Dict, List, Optional

from .domain.models.models import (
    ApiEndpointConfig,
    ExecutionState,
    ExecutionStatus,
    StepExecutionTrace,
    WorkflowBlueprint,
    WorkflowStep,
)
from .domain.ports.outbound import IActivityExecutor, IStateStore

ENGINE_ENV = "OCTOPUS_LIVE_ENGINE"


def core_requested(engine: Optional[str]) -> bool:
    """Whether to run on the core: the `engine` argument, else the
    `OCTOPUS_LIVE_ENGINE` environment variable. The default is Python."""
    choice = (engine or os.environ.get(ENGINE_ENV) or "python").strip().lower()
    if choice not in ("python", "core"):
        raise ValueError(f"engine must be 'python' or 'core', not {choice!r}")
    return choice == "core"


def new_core_engine(trusted_proxies: Optional[List[str]] = None):
    """`trusted_proxies`: the proxies whose `X-Forwarded-For` / `X-Real-IP`
    name the client (D11); nobody else's are believed."""
    import octopus_core

    return octopus_core.Engine("", {"trusted_proxies": list(trusted_proxies or [])})


# --- Blueprints ---------------------------------------------------------------

_STEP_DEFAULTS = {f.name: f.default for f in dataclasses.fields(WorkflowStep) if f.default is not dataclasses.MISSING}


def _clean(value: Any) -> Any:
    """`asdict` output without empty values, so the core applies its own
    defaults, and with `is_async` spelled `async` as in YAML."""
    if isinstance(value, dict):
        out = {}
        for k, v in value.items():
            v = _clean(v)
            if v is None or v == [] or v == {}:
                continue
            out["async" if k == "is_async" else k] = v
        return out
    if isinstance(value, list):
        return [_clean(v) for v in value]
    return value


def blueprint_to_raw(bp: WorkflowBlueprint) -> Dict[str, Any]:
    """The blueprint as the YAML-shaped dict the core parses.

    The Python engine never enforces `timeout_seconds`; its dataclass default
    (30) is left out so the core does not start enforcing it."""
    raw = dataclasses.asdict(bp)

    def strip_steps(steps):
        for st in steps or []:
            if st.get("timeout_seconds") == _STEP_DEFAULTS["timeout_seconds"]:
                st.pop("timeout_seconds")
            cond = st.get("condition") or {}
            # Several rules: the core reads them as the step's `conditions`
            # group; `then_steps`/`else_steps` stay under `condition`.
            rules = cond.pop("rules", None)
            match = cond.pop("match", None)
            cond.pop("rule_type", None)
            if rules:
                st["conditions"] = {"match": match or "all", "conditions": rules}
                for k in ("rule", "field", "operator", "value"):
                    cond.pop(k, None)
            strip_steps(cond.get("then_steps"))
            strip_steps(cond.get("else_steps"))

    strip_steps(raw.get("steps"))
    for ep in raw.get("api") or []:
        strip_steps(ep.get("steps"))
    # The router enforces initiator roles before a run starts; the core
    # refuses blueprints that ask it to.
    raw.pop("allowed_initiator_roles", None)
    return _clean(raw)


# --- States -------------------------------------------------------------------

_STATE_FIELDS = {f.name for f in dataclasses.fields(ExecutionState)}
_TRACE_FIELDS = {f.name for f in dataclasses.fields(StepExecutionTrace)}


def _status(value: Any) -> Any:
    try:
        return ExecutionStatus(value)
    except ValueError:
        return value


def state_from_core(d: Optional[Dict[str, Any]]) -> Optional[ExecutionState]:
    if d is None:
        return None
    fields = {k: v for k, v in d.items() if k in _STATE_FIELDS}
    fields["status"] = _status(fields.get("status", "IDLE"))
    if fields.get("error_code") == 0:
        fields["error_code"] = None
    fields["traces"] = [
        StepExecutionTrace(**{k: v for k, v in t.items() if k in _TRACE_FIELDS}) for t in d.get("traces") or []
    ]
    return ExecutionState(**fields)


def state_to_core(state: ExecutionState) -> Dict[str, Any]:
    d = dataclasses.asdict(state)
    d.pop("route_output", None)
    d["status"] = state.status.value if isinstance(state.status, ExecutionStatus) else str(state.status)
    return d


# --- Activities ---------------------------------------------------------------


class ActivityFailure(Exception):
    """An activity's exception in the core's error form, `[Type:code] message`.

    The core then records the type and message apart, as the Python engine
    does. Code 0 means the exception carries no HTTP status (`code` or
    `status_code`); `state_from_core` turns it back into None."""

    def __init__(self, ex: Exception):
        self.original = ex
        code = next(
            (
                c for c in (getattr(ex, "code", None), getattr(ex, "status_code", None))
                if isinstance(c, int) and not isinstance(c, bool) and 100 <= c < 600
            ),
            0,
        )
        message = getattr(ex, "message", None)
        if not isinstance(message, str):
            message = str(ex)
        super().__init__(f"[{type(ex).__name__}:{code}] {message}")


class _ActivityBridge:
    """Runs the core's activity calls through the engine's executor.

    The core calls from its own threads. A plain function runs right there;
    a coroutine runs on the event loop that started the job (the server's),
    or on a private loop when that one is gone."""

    def __init__(self, executor: IActivityExecutor):
        self.executor = executor
        self.loop: Optional[asyncio.AbstractEventLoop] = None

    def _function(self, name: str):
        acts = getattr(self.executor, "_activities", None)
        if acts is None or type(self.executor).execute_activity is not _callable_execute_activity():
            return None
        return acts.get(name) or acts.get(self.executor._to_snake_case(name))

    def call(self, name: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        try:
            fn = self._function(name)
            if fn is not None and not inspect.iscoroutinefunction(fn):
                result = fn(payload)
                return result if isinstance(result, dict) else {"result": result}
            return self._run(self.executor.execute_activity(name, payload))
        except Exception as ex:
            raise ActivityFailure(ex) from ex

    def has(self, name: str) -> bool:
        return self.executor.has_activity(name)

    def _run(self, coro):
        loop = self.loop
        if loop is not None and loop.is_running() and not _on_loop_thread(loop):
            return asyncio.run_coroutine_threadsafe(coro, loop).result()
        return asyncio.run(coro)


def _callable_execute_activity():
    from .adapters.outbound.callable_executor import CallableActivityExecutor

    return CallableActivityExecutor.execute_activity


def _on_loop_thread(loop: asyncio.AbstractEventLoop) -> bool:
    try:
        return asyncio.get_running_loop() is loop
    except RuntimeError:
        return False


# --- Pipeline -----------------------------------------------------------------


class CorePipelineManager:
    """`PipelineManager` on the core."""

    def __init__(self, activity_executor: IActivityExecutor, core: Any, temporal: Optional[Dict[str, Any]] = None):
        """`temporal`: for `mode: hybrid`, the Temporal server async steps go
        to (D13); None runs them in-process."""
        self.activity_executor = activity_executor
        self.core = core
        self.temporal = temporal
        self._bridge = _ActivityBridge(activity_executor)
        core.set_activity_resolver(self._bridge.call, self._bridge.has)
        self._registered: Dict[str, int] = {}
        self._lock = threading.Lock()

    def sync_blueprint(self, bp: WorkflowBlueprint, force: bool = False) -> None:
        """Registers `bp` with the core unless this same object already is.
        Edits made in place need `force` (`invalidate`)."""
        with self._lock:
            if not force and self._registered.get(bp.workflow_name) == id(bp):
                return
            raw = blueprint_to_raw(bp)
            persistence = dict(raw.get("persistence") or {})
            # Temporal is for `mode: hybrid` only, as with the Python engine;
            # there it takes the engine's settings unless the blueprint has its own.
            temporal = persistence.pop("temporal", None)
            if self.temporal is not None:
                persistence["temporal"] = temporal if isinstance(temporal, dict) and temporal.get("host") else self.temporal
            if persistence.get("redis_url") and not persistence.get("url"):
                persistence["url"] = persistence["redis_url"]
            if persistence:
                raw["persistence"] = persistence
            else:
                raw.pop("persistence", None)
            self.core.register_blueprint(raw)
            self._registered[bp.workflow_name] = id(bp)

    def invalidate(self) -> None:
        with self._lock:
            self._registered.clear()

    async def execute(
        self,
        blueprint: WorkflowBlueprint,
        payload: Dict[str, Any],
        endpoint: Optional[ApiEndpointConfig] = None,
        route: Optional[str] = None,
        method: Optional[str] = None,
    ) -> ExecutionState:
        import octopus_core

        self.sync_blueprint(blueprint)
        loop = asyncio.get_running_loop()
        self._bridge.loop = loop
        index = next((i for i, ep in enumerate(blueprint.api) if ep is endpoint), None)
        req_route = route or (endpoint.route if endpoint else "") or ""
        req_method = method or (endpoint.method if endpoint else "") or ""

        def run():
            try:
                return self.core.execute(
                    blueprint.workflow_name, dict(payload), endpoint_index=index, route=req_route, method=req_method
                )
            except octopus_core.OctopusError as err:
                state = err.args[1] if len(err.args) > 1 else None
                if state is None:
                    raise
                return state

        return state_from_core(await loop.run_in_executor(None, run))

    async def initiate_async(self, blueprint: WorkflowBlueprint, payload: Dict[str, Any]) -> ExecutionState:
        self.sync_blueprint(blueprint)
        self._bridge.loop = asyncio.get_running_loop()
        return state_from_core(self.core.initiate_async(blueprint.workflow_name, dict(payload)))


# --- State store --------------------------------------------------------------


_FINAL_EVENTS = ("workflow_completed", "workflow_failed", "workflow_rolled_back")


class CoreStateStore(IStateStore):
    """The core's jobs as an `IStateStore`."""

    def __init__(self, core: Any):
        self.core = core

    async def save_state(self, state: ExecutionState) -> None:
        self.core.save_state(state_to_core(state))

    def get_state_sync(self, job_id: str) -> Optional[ExecutionState]:
        return state_from_core(self.core.get_state(job_id))

    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        return self.get_state_sync(job_id)

    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        return [state_from_core(s) for s in self.core.list_states(workflow_name, limit)]

    async def publish_event(self, job_id: str, event_type: str, data: Dict[str, Any]) -> None:
        """Events come from the core's runs; there is nothing to publish here."""

    async def subscribe(self, job_id: str) -> AsyncIterator[Dict[str, Any]]:
        sub = self.core.subscribe(job_id)
        loop = asyncio.get_running_loop()
        try:
            while True:
                ev = await loop.run_in_executor(None, sub.next, 1.0)
                if ev is None:
                    continue
                yield {"event": ev["event"], "data": ev["data"]}
                if ev["event"] in _FINAL_EVENTS:
                    break
        finally:
            sub.close()
