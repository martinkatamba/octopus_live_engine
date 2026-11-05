import asyncio
import re
import logging
import os
import uuid
from typing import Any, AsyncIterator, Callable, Dict, List, Optional, Type, Union
from fastapi import FastAPI
from pydantic import BaseModel

from .adapters.inbound.rest_router import create_live_router
from .adapters.inbound.visualizer_html import VisualizerHtmlRenderer
from .adapters.outbound.callable_executor import CallableActivityExecutor
from .adapters.outbound.in_memory_store import InMemoryStateStore
from .adapters.outbound.store_factory import StoreFactory
from .adapters.outbound.temporal_replicator import TemporalReplicator
from .adapters.outbound.yaml_repository import YamlBlueprintRepository
from .domain.models.models import ApiEndpointConfig, ErrorHandlingConfig, ExecutionState, ExecutionStatus, GlobalExceptionHandlerConfig, WorkflowBlueprint
from .domain.ports.inbound import ILiveWorkflowUseCase, IVisualizerUseCase
from .domain.ports.outbound import IActivityExecutor, IBlueprintRepository, IStateStore
from .domain.services.middleware import (
    ApiKeyAuthMiddleware,
    BadRequestException,
    BearerAuthAndClaimsMiddleware,
    CorrelationIdMiddleware,
    InputValidationMiddleware,
    InvalidApiKeyException,
    InvalidClaimException,
    IpAccessDenied,
    IpWhitelistMiddleware,
    LiveMiddleware,
    MalformedJsonException,
    MissingRequiredFieldException,
    OctopusMiddlewareException,
    PayloadTooLargeException,
    PayloadValidationMiddleware,
    RateLimitExceeded,
    RateLimitMiddleware,
    RedisCacheMiddleware,
    ResponseAuditMiddleware,
    RoleAuthorizationMiddleware,
    UserNotAuthenticated,
    UserNotAuthorized,
    ValidationException,
    ValidationMiddleware,
)
from .core_backend import CorePipelineManager, CoreStateStore, core_available, engine_choice, new_core_engine
from .domain.services.pipeline_manager import PipelineManager
from .domain.services.replication_manager import DurableReplicationManager

logger = logging.getLogger(__name__)


def _implied_store(store_cfg: Dict[str, Any]) -> Optional[str]:
    """The store a constructor's `sqlite_path=` / `redis_url=` / `duckdb_path=` implies."""
    for key, store in (("sqlite_path", "sqlite"), ("redis_url", "redis"), ("duckdb_path", "duckdb")):
        if store_cfg.get(key):
            return store
    return None


def _core_store_config(store_cfg: Dict[str, Any]) -> Dict[str, Any]:
    """`persistence:` settings with Python's key names, as the core reads them."""
    cfg = {k: v for k, v in store_cfg.items() if k != "temporal"}
    if cfg.get("redis_url") and not cfg.get("url"):
        cfg["url"] = cfg["redis_url"]
    return cfg


# `store:` spellings, as the core's names in `octopus_core.STORES`.
_STORE_NAMES = {
    "sqlite": "sqlite", "sqlite3": "sqlite",
    "redis": "redis", "redis_cluster": "redis",
    "postgres": "postgres", "postgresql": "postgres", "pgsql": "postgres",
    "mysql": "mysql", "mariadb": "mysql",
    "sqlserver": "sqlserver", "mssql": "sqlserver", "tsql": "sqlserver",
    "duckdb": "duckdb", "duck": "duckdb",
}

_LOGO_BADGE = re.compile(r'<div class="logo-badge"[^>]*>.*?</div>', re.DOTALL)


def _engine_badge(backend: str) -> str:
    label, title, color = (
        ("Rust core", "Jobs run on the OCTOPUS engine core (Rust)", "249, 115, 22")
        if backend == "core"
        else ("Python engine", "Jobs run on the pure-Python engine", "56, 189, 248")
    )
    return (
        f'<span id="engineBackendBadge" data-backend="{backend}" title="{title}" '
        f'style="margin-left:8px;padding:3px 8px;border-radius:6px;font-size:11px;font-weight:600;'
        f'white-space:nowrap;color:rgb({color});background:rgba({color},0.12);'
        f'border:1px solid rgba({color},0.45);">{label}</span>'
    )


class OctopusLiveEngine(ILiveWorkflowUseCase, IVisualizerUseCase):
    """Facade orchestrating Hexagonal Ports & Adapters for pure in-memory live workflow execution,
    with Zero-Import pluggable persistence (SQLite, DuckDB, Redis) and non-blocking Temporal durability.
    """

    def __init__(
        self,
        config_dir: str = "config",
        activity_executor: Optional[IActivityExecutor] = None,
        blueprint_repo: Optional[IBlueprintRepository] = None,
        state_store: Optional[Union[IStateStore, str]] = None,
        mode: str = "sync",
        persistence: Optional[Dict[str, Any]] = None,
        temporal_host: Optional[str] = None,
        temporal_namespace: Optional[str] = None,
        temporal_task_queue: Optional[str] = None,
        sqlite_path: Optional[str] = None,
        duckdb_path: Optional[str] = None,
        redis_url: Optional[str] = None,
        embedded_worker_enabled: bool = True,
        engine: Optional[str] = None,
        trusted_proxies: Optional[List[str]] = None,
        **kwargs: Any
    ):
        """`engine` is "auto" (the default: the Rust engine core when
        `octopus_core` is installed, else Python), "core" or "python";
        `OCTOPUS_LIVE_ENGINE` sets the default (see `core_backend`).
        `trusted_proxies` (IPs or CIDRs) are the proxies allowed to name the
        client with `X-Forwarded-For` on the core; the Python engine believes
        that header from anyone."""
        self.blueprint_repo = blueprint_repo or YamlBlueprintRepository(config_dir)
        self.activity_executor = activity_executor or CallableActivityExecutor()
        self.embedded_worker_enabled = embedded_worker_enabled

        # Configure durable store and replication
        self.mode = mode
        self.durable_store: Optional[IStateStore] = None
        self.replication_manager: Optional[DurableReplicationManager] = None

        pers_cfg = persistence or {}
        store_cfg = dict(pers_cfg)
        if sqlite_path:
            store_cfg["sqlite_path"] = sqlite_path
        if duckdb_path:
            store_cfg["duckdb_path"] = duckdb_path
        if redis_url:
            store_cfg["redis_url"] = redis_url

        use_core = False
        choice = engine_choice(engine)
        if choice != "python":
            if core_available():
                reason = self._core_unsupported(state_store, store_cfg)
            else:
                reason = "octopus_core is not installed (pip install 'octopus-live-engine[core]')"
            if reason:
                # Asked for by name, falling back is worth a warning.
                log = logger.warning if choice == "core" else logger.info
                log("OCTOPUS: running on the Python engine, not the core: %s", reason)
            else:
                use_core = True

        if use_core:
            # The core keeps the jobs and runs Temporal itself (D13): no Python
            # replication, no second store.
            self.state_store = None
        elif mode == "hybrid":
            # Fast-path in-memory state store for sub-millisecond API responses
            self.state_store = InMemoryStateStore()
            
            # Secondary durable store for asynchronous persistence
            resolved_store_type = state_store if isinstance(state_store, str) else store_cfg.get("store", "sqlite")
            self.durable_store = StoreFactory.create(
                store_type=resolved_store_type,
                config=store_cfg,
                **kwargs
            )

            temp_cfg = pers_cfg.get("temporal") or {} if isinstance(pers_cfg.get("temporal"), dict) else {}
            host = temporal_host or temp_cfg.get("host") or "127.0.0.1:7233"
            ns = temporal_namespace or temp_cfg.get("namespace") or "default"
            tq = temporal_task_queue or temp_cfg.get("task_queue") or "octopus-hybrid"
            temporal_rep = TemporalReplicator(host=host, namespace=ns, task_queue=tq, activity_executor=self.activity_executor)

            self.replication_manager = DurableReplicationManager(
                durable_store=self.durable_store,
                temporal_replicator=temporal_rep
            )
        else:
            if isinstance(state_store, str) or persistence or sqlite_path or duckdb_path or redis_url:
                self.state_store = StoreFactory.create(
                    store_type=state_store if isinstance(state_store, str) else store_cfg.get("store"),
                    config=store_cfg,
                    **kwargs
                )
            elif isinstance(state_store, IStateStore):
                self.state_store = state_store
            else:
                self.state_store = InMemoryStateStore()

        self.core = None
        self.engine_backend = "python"
        core_temporal = None
        if use_core:
            self.core = new_core_engine(trusted_proxies)
            self.engine_backend = "core"
            self.state_store = CoreStateStore(self.core)
            core_store = state_store if isinstance(state_store, str) else store_cfg.get("store") or _implied_store(store_cfg)
            if isinstance(state_store, IStateStore):
                # The router reads it directly; the core keeps its jobs there
                # (`use_python_store`, below).
                self.state_store = state_store
            elif core_store:
                self.core.set_state_store(str(core_store), _core_store_config(store_cfg))
            if mode == "hybrid":
                temp_cfg = pers_cfg.get("temporal") if isinstance(pers_cfg.get("temporal"), dict) else {}
                core_temporal = {
                    "host": temporal_host or temp_cfg.get("host") or "127.0.0.1:7233",
                    "namespace": temporal_namespace or temp_cfg.get("namespace") or "default",
                    "task_queue": temporal_task_queue or temp_cfg.get("task_queue") or "octopus-hybrid",
                }

        if self.core is not None:
            self.pipeline_manager = CorePipelineManager(
                self.activity_executor,
                self.core,
                temporal=core_temporal,
            )
            if isinstance(state_store, IStateStore):
                self.pipeline_manager.use_python_store(state_store)
        else:
            self.pipeline_manager = PipelineManager(
                self.activity_executor,
                self.state_store,
                replication_manager=self.replication_manager
            )
        self._models: Dict[str, Type[BaseModel]] = {}
        self._middlewares: List[Any] = []
        self._middleware_registry: Dict[str, Any] = {
            "BearerAuthAndClaimsMiddleware": BearerAuthAndClaimsMiddleware,
            "BearerAuthMiddleware": BearerAuthAndClaimsMiddleware,
            "ApiKeyAuthMiddleware": ApiKeyAuthMiddleware,
            "RoleAuthorizationMiddleware": RoleAuthorizationMiddleware,
            "CorrelationIdMiddleware": CorrelationIdMiddleware,
            "RateLimitMiddleware": RateLimitMiddleware,
            "IpWhitelistMiddleware": IpWhitelistMiddleware,
            "WhitelistMiddleware": IpWhitelistMiddleware,
            "IpFilterMiddleware": IpWhitelistMiddleware,
            "ResponseAuditMiddleware": ResponseAuditMiddleware,
            "RedisCacheMiddleware": RedisCacheMiddleware,
            "CacheMiddleware": RedisCacheMiddleware,
            "PayloadValidationMiddleware": PayloadValidationMiddleware,
            "BadRequestMiddleware": PayloadValidationMiddleware,
            "ValidationMiddleware": PayloadValidationMiddleware,
            "InputValidationMiddleware": PayloadValidationMiddleware,
        }

    @staticmethod
    def _core_unsupported(state_store, store_cfg) -> Optional[str]:
        """Why the core can't serve this configuration, or None: only a
        store the installed `octopus_core` was built without."""
        import octopus_core

        store_type = state_store if isinstance(state_store, str) else store_cfg.get("store") or _implied_store(store_cfg)
        kind = _STORE_NAMES.get(str(store_type or "").lower())
        # A core from before `STORES` had every store but DuckDB.
        compiled = getattr(octopus_core, "STORES", ("sqlite", "redis", "postgres", "mysql", "sqlserver"))
        if kind and kind not in compiled:
            return f"this octopus_core was built without the {kind} store"
        return None

    @classmethod
    def from_yaml(
        cls,
        yaml_file: str,
        activity_instance: Optional[Any] = None,
        **kwargs: Any
    ) -> "OctopusLiveEngine":
        """Zero-Import convenience initializer loading a blueprint YAML file with auto-configured persistence."""
        repo = YamlBlueprintRepository(os.path.dirname(yaml_file) or ".")
        bp = repo.load_file(yaml_file)

        # Merge YAML persistence settings with kwargs
        pers_cfg = bp.persistence if bp and bp.persistence else {}
        mode = kwargs.pop("mode", bp.mode if bp else "sync")
        
        engine = cls(
            blueprint_repo=repo,
            mode=mode,
            persistence=pers_cfg,
            **kwargs
        )

        if activity_instance:
            engine.register_activities(activity_instance)

        if bp and engine.replication_manager and engine.replication_manager.temporal_replicator:
            all_names = set()
            for step in bp.steps:
                all_names.add(step.id)
                if step.activity:
                    all_names.add(step.activity)
            for ep in bp.api:
                for step in ep.steps:
                    all_names.add(step.id)
                    if step.activity:
                        all_names.add(step.activity)
                    if step.condition:
                        for s in step.condition.then_steps:
                            all_names.add(s.id)
                            if s.activity:
                                all_names.add(s.activity)
                        for s in step.condition.else_steps:
                            all_names.add(s.id)
                            if s.activity:
                                all_names.add(s.activity)
            for m in bp.middlewares:
                for exc in m.exceptions:
                    if exc.activity:
                        all_names.add(exc.activity)
            engine.replication_manager.temporal_replicator.register_activity_names(list(all_names))

        return engine

    # Model Registration
    def register_model(self, name: str, model_cls: Type[BaseModel]) -> None:
        """Register a client-side Pydantic model for request/response validation and OpenAPI docs."""
        self._models[name] = model_cls

    def get_model(self, name: str) -> Optional[Type[BaseModel]]:
        return self._models.get(name)

    # Activity Registration
    def register_activity(self, name: str, func: Callable[..., Any]) -> None:
        if isinstance(self.activity_executor, CallableActivityExecutor):
            self.activity_executor.register(name, func)
        if self.replication_manager and self.replication_manager.temporal_replicator:
            self.replication_manager.temporal_replicator.register_activity_names([name])

    def activity(self, name: Optional[str] = None):
        def decorator(func: Callable[..., Any]):
            act_name = name or func.__name__
            self.register_activity(act_name, func)
            return func
        return decorator

    def register_blueprint(self, blueprint: WorkflowBlueprint) -> None:
        if isinstance(self.blueprint_repo, YamlBlueprintRepository):
            self.blueprint_repo.save(blueprint)

    def load_blueprints_from_directory(self, dir_path: str) -> None:
        if isinstance(self.blueprint_repo, YamlBlueprintRepository):
            self.blueprint_repo.load_directory(dir_path)

    def load_blueprint_file(self, file_path: str) -> Optional[WorkflowBlueprint]:
        if isinstance(self.blueprint_repo, YamlBlueprintRepository):
            return self.blueprint_repo.load_file(file_path)
        return None

    def register_activities(self, instance: Any) -> None:
        if isinstance(self.activity_executor, CallableActivityExecutor):
            self.activity_executor.register_instance(instance)
            if self.replication_manager and self.replication_manager.temporal_replicator:
                self.replication_manager.temporal_replicator.register_activity_names(
                    list(self.activity_executor._activities.keys())
                )

    # Inbound Use Case: ILiveWorkflowUseCase
    def set_global_exception_handler(
        self,
        activity: str,
        code: int = 500,
        message: Optional[str] = None,
        is_async: bool = False,
        include_stack_trace: bool = True,
        workflow_name: Optional[str] = None
    ) -> None:
        """Configures a fallback Global Exception Handler activity across all workflows or a specific workflow."""
        cfg = GlobalExceptionHandlerConfig(
            activity=activity,
            code=code,
            message=message,
            is_async=is_async,
            include_stack_trace=include_stack_trace
        )
        eh_cfg = ErrorHandlingConfig(global_handler=cfg)
        bps = self.blueprint_repo.list_all()
        for bp in bps:
            if workflow_name is None or bp.workflow_name == workflow_name:
                bp.error_handling = eh_cfg
                bp.global_exception_handler = cfg
        if isinstance(self.pipeline_manager, CorePipelineManager):
            self.pipeline_manager.invalidate()

    async def execute_sync(
        self,
        workflow_name: str,
        payload: Dict[str, Any],
        endpoint: Optional[ApiEndpointConfig] = None,
        route: Optional[str] = None,
        method: Optional[str] = None
    ) -> ExecutionState:
        bp = self.blueprint_repo.get(workflow_name)
        if not bp:
            raise ValueError(f"Workflow blueprint '{workflow_name}' not found")
        return await self.pipeline_manager.execute(bp, payload, endpoint=endpoint, route=route, method=method)

    async def initiate_async(self, workflow_name: str, payload: Dict[str, Any]) -> ExecutionState:
        bp = self.blueprint_repo.get(workflow_name)
        if not bp:
            raise ValueError(f"Workflow blueprint '{workflow_name}' not found")

        if isinstance(self.pipeline_manager, CorePipelineManager):
            return await self.pipeline_manager.initiate_async(bp, payload)

        job_id = payload.get("job_id") or f"job_{uuid.uuid4()}"
        initial_state = ExecutionState(
            job_id=job_id,
            workflow_name=workflow_name,
            status=ExecutionStatus.RUNNING,
            data=dict(payload)
        )
        await self.state_store.save_state(initial_state)

        # Spawn in background task
        asyncio.create_task(self.pipeline_manager.execute(bp, payload))
        return initial_state

    async def get_execution_state(self, job_id: str) -> Optional[ExecutionState]:
        if self.durable_store:
            st = await self.durable_store.get_state(job_id)
            if st is not None:
                return st
        return await self.state_store.get_state(job_id)

    async def list_execution_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        if self.durable_store and hasattr(self.durable_store, "list_states"):
            return await self.durable_store.list_states(workflow_name=workflow_name, limit=limit)
        if hasattr(self.state_store, "list_states"):
            return await self.state_store.list_states(workflow_name=workflow_name, limit=limit)
        return []


    async def stream_events(self, job_id: str) -> AsyncIterator[Dict[str, Any]]:
        if hasattr(self.state_store, "subscribe"):
            async for ev in self.state_store.subscribe(job_id):
                yield ev

    def get_workflow_use_case(self) -> ILiveWorkflowUseCase:
        return self

    def get_visualizer_use_case(self) -> IVisualizerUseCase:
        return self

    # Inbound Use Case: IVisualizerUseCase
    def get_blueprint(self, workflow_name: str) -> Optional[WorkflowBlueprint]:
        return self.blueprint_repo.get(workflow_name)

    def list_blueprints(self) -> List[WorkflowBlueprint]:
        return self.blueprint_repo.list_all()

    def get_available_activities(self) -> Dict[str, Any]:
        avail: Dict[str, Any] = {}
        if isinstance(self.activity_executor, CallableActivityExecutor):
            import inspect
            for name, fn in self.activity_executor._activities.items():
                doc = inspect.getdoc(fn) or f"Live In-Memory Activity '{name}'"
                mod = getattr(fn, "__module__", "in_memory")
                cls_name = getattr(fn, "__qualname__", fn.__name__ if hasattr(fn, "__name__") else name)
                avail[name] = {
                    "name": name,
                    "class": cls_name,
                    "module": mod,
                    "file": "in_memory",
                    "doc": doc
                }
        return avail

    def visualizer_page(self, domain: str, prefix: str, domains_registry: List[Dict[str, Any]]) -> str:
        """The visualizer's HTML: the core's page on the core (D12), with a
        badge after the logo naming the engine that runs the jobs."""
        if self.core is not None:
            import octopus_core

            html = octopus_core.visualizer_html(prefix, domain, "Python")
        else:
            html = VisualizerHtmlRenderer.render_html(domain, prefix, domains_registry)
        return _LOGO_BADGE.sub(lambda m: m.group(0) + _engine_badge(self.engine_backend), html, count=1)

    def _core_diagram(self, workflow_name: str, job_id: Optional[str], endpoint_index: Optional[int],
                      route: Optional[str], method: Optional[str]) -> Dict[str, Any]:
        """The diagram from the core (D12). A Temporal workflow ID
        (`wf-<workflow>-<job>`) names its job."""
        bp = self.blueprint_repo.get(workflow_name)
        if not bp:
            return {"nodes": [], "edges": [], "workflow_name": workflow_name}
        self.pipeline_manager.sync_blueprint(bp)
        if endpoint_index is not None and 0 <= endpoint_index < len(bp.api) and not route:
            route, method = bp.api[endpoint_index].route, bp.api[endpoint_index].method
        if job_id:
            for cid in self._job_id_candidates(workflow_name, job_id):
                if self.core.get_state(cid) is not None:
                    job_id = cid
                    break
        return self.core.compile_diagram(workflow_name, job_id, route, method)

    @staticmethod
    def _job_id_candidates(workflow_name: str, job_id: str) -> List[str]:
        candidate_ids = [job_id]
        if job_id.startswith(f"wf-{workflow_name}-"):
            candidate_ids.append(job_id[len(f"wf-{workflow_name}-"):])
        elif job_id.startswith("wf-"):
            parts = job_id.split("-", 2)
            if len(parts) == 3:
                candidate_ids.append(parts[2])
        return candidate_ids

    async def compile_x6_diagram_async(
        self,
        workflow_name: str,
        job_id: Optional[str] = None,
        endpoint_index: Optional[int] = None,
        route: Optional[str] = None,
        method: Optional[str] = None
    ) -> Dict[str, Any]:
        if self.core is not None:
            # Off the loop: the core may read the job from a Python store.
            loop = asyncio.get_running_loop()
            self.pipeline_manager.bind_loop(loop)
            return await loop.run_in_executor(
                None, self._core_diagram, workflow_name, job_id, endpoint_index, route, method
            )
        bp = self.blueprint_repo.get(workflow_name)
        if not bp:
            return {"nodes": [], "edges": [], "workflow_name": workflow_name}

        state = None
        if job_id:
            candidate_ids = [job_id]
            if job_id.startswith(f"wf-{workflow_name}-"):
                candidate_ids.append(job_id[len(f"wf-{workflow_name}-"):])
            elif job_id.startswith("wf-"):
                parts = job_id.split("-", 2)
                if len(parts) == 3:
                    candidate_ids.append(parts[2])

            for cid in candidate_ids:
                state = await self.get_execution_state(cid)
                if state:
                    break

        return VisualizerHtmlRenderer.compile_x6_diagram(
            bp,
            state,
            endpoint_index=endpoint_index,
            route=route,
            method=method,
            available_activities=self.get_available_activities()
        )

    def compile_x6_diagram(
        self,
        workflow_name: str,
        job_id: Optional[str] = None,
        endpoint_index: Optional[int] = None,
        route: Optional[str] = None,
        method: Optional[str] = None
    ) -> Dict[str, Any]:
        if self.core is not None:
            return self._core_diagram(workflow_name, job_id, endpoint_index, route, method)
        bp = self.blueprint_repo.get(workflow_name)
        if not bp:
            return {"nodes": [], "edges": [], "workflow_name": workflow_name}

        state = None
        if job_id:
            candidate_ids = [job_id]
            if job_id.startswith(f"wf-{workflow_name}-"):
                candidate_ids.append(job_id[len(f"wf-{workflow_name}-"):])
            elif job_id.startswith("wf-"):
                parts = job_id.split("-", 2)
                if len(parts) == 3:
                    candidate_ids.append(parts[2])

            for cid in candidate_ids:
                if hasattr(self.state_store, "get_state_sync"):
                    state = self.state_store.get_state_sync(cid)
                if not state and self.durable_store and hasattr(self.durable_store, "get_state_sync"):
                    state = self.durable_store.get_state_sync(cid)
                if not state and hasattr(self.state_store, "_states"):
                    state = self.state_store._states.get(cid)
                if not state and hasattr(self.state_store, "_fallback_states"):
                    state = self.state_store._fallback_states.get(cid)
                if not state and self.durable_store and hasattr(self.durable_store, "_fallback_states"):
                    state = self.durable_store._fallback_states.get(cid)
                if state:
                    break

        return VisualizerHtmlRenderer.compile_x6_diagram(
            bp,
            state,
            endpoint_index=endpoint_index,
            route=route,
            method=method,
            available_activities=self.get_available_activities()
        )

    # Middleware Registration & Management
    def register_middleware(self, name_or_instance: Union[str, Any], middleware_or_factory: Optional[Any] = None) -> None:
        """Register a middleware implementation matching a YAML name, or register an instance directly."""
        if isinstance(name_or_instance, str) and middleware_or_factory is not None:
            if hasattr(middleware_or_factory, "activity_executor") and not getattr(middleware_or_factory, "activity_executor", None):
                middleware_or_factory.activity_executor = self.activity_executor
            if hasattr(middleware_or_factory, "state_store") and not getattr(middleware_or_factory, "state_store", None):
                middleware_or_factory.state_store = self.state_store
            if hasattr(middleware_or_factory, "durable_store") and not getattr(middleware_or_factory, "durable_store", None):
                middleware_or_factory.durable_store = self.durable_store
            self._middleware_registry[name_or_instance] = middleware_or_factory
        else:
            self.add_middleware(name_or_instance)

    def add_middleware(self, middleware: Any) -> None:
        """Add an active middleware instance or callable to the engine."""
        if hasattr(middleware, "activity_executor") and not getattr(middleware, "activity_executor", None):
            middleware.activity_executor = self.activity_executor
        if hasattr(middleware, "state_store") and not getattr(middleware, "state_store", None):
            middleware.state_store = self.state_store
        if hasattr(middleware, "durable_store") and not getattr(middleware, "durable_store", None):
            middleware.durable_store = self.durable_store
        if middleware not in self._middlewares:
            self._middlewares.append(middleware)

    def use(self, middleware: Any) -> None:
        """Fluent alias for add_middleware."""
        self.add_middleware(middleware)

    def _resolve_middlewares(self) -> List[Any]:
        """Collects explicitly added middlewares and resolves declared middlewares from blueprints."""
        resolved: List[Any] = list(self._middlewares)
        temporal_rep = self.replication_manager.temporal_replicator if self.replication_manager else None

        for bp in self.blueprint_repo.list_all():
            for mw_cfg in bp.middlewares:
                mw_name = mw_cfg.name
                # Check if already in resolved by matching name or instance
                already_present = any(
                    getattr(m, "name", None) == mw_name or m.__class__.__name__ == mw_name
                    for m in resolved
                )
                if already_present:
                    # Update excludes/includes/exceptions on existing instance if provided in YAML
                    for m in resolved:
                        if getattr(m, "name", None) == mw_name or m.__class__.__name__ == mw_name:
                            if hasattr(m, "excludes") and mw_cfg.excludes:
                                m.excludes = list(set(m.excludes + mw_cfg.excludes))
                            if hasattr(m, "includes") and mw_cfg.includes:
                                m.includes = list(set(m.includes + mw_cfg.includes))
                            if hasattr(m, "exceptions") and mw_cfg.exceptions:
                                m.exceptions = list(m.exceptions + mw_cfg.exceptions)
                            if hasattr(m, "activity_executor") and not m.activity_executor:
                                m.activity_executor = self.activity_executor
                            if hasattr(m, "temporal_replicator") and not m.temporal_replicator:
                                m.temporal_replicator = temporal_rep
                            if hasattr(m, "state_store") and not m.state_store:
                                m.state_store = self.state_store
                            if hasattr(m, "durable_store") and not m.durable_store:
                                m.durable_store = self.durable_store
                    continue

                # Look up from registry
                factory_or_cls = self._middleware_registry.get(mw_name)
                if factory_or_cls:
                    if isinstance(factory_or_cls, type):
                        import inspect
                        try:
                            sig = inspect.signature(factory_or_cls.__init__)
                            has_varkw = any(p.kind == inspect.Parameter.VAR_KEYWORD for p in sig.parameters.values())
                            call_kwargs = dict(mw_cfg.config)
                            call_kwargs.update({
                                "excludes": mw_cfg.excludes,
                                "includes": mw_cfg.includes,
                                "exceptions": mw_cfg.exceptions,
                                "activity_executor": self.activity_executor,
                                "temporal_replicator": temporal_rep,
                                "state_store": self.state_store,
                                "durable_store": self.durable_store,
                            })
                            if not has_varkw:
                                call_kwargs = {k: v for k, v in call_kwargs.items() if k in sig.parameters}
                            inst = factory_or_cls(**call_kwargs)
                            if hasattr(inst, "state_store") and not inst.state_store:
                                inst.state_store = self.state_store
                            if hasattr(inst, "durable_store") and not inst.durable_store:
                                inst.durable_store = self.durable_store
                            resolved.append(inst)
                        except Exception:
                            # Fallback standard instantiation
                            inst = factory_or_cls(
                                excludes=mw_cfg.excludes,
                                **mw_cfg.config
                            )
                            if hasattr(inst, "state_store") and not inst.state_store:
                                inst.state_store = self.state_store
                            if hasattr(inst, "durable_store") and not inst.durable_store:
                                inst.durable_store = self.durable_store
                            resolved.append(inst)
                    elif isinstance(factory_or_cls, LiveMiddleware) or hasattr(factory_or_cls, "excludes"):
                        if mw_cfg.excludes:
                            factory_or_cls.excludes = list(set(getattr(factory_or_cls, "excludes", []) + mw_cfg.excludes))
                        if mw_cfg.includes:
                            factory_or_cls.includes = list(set(getattr(factory_or_cls, "includes", []) + mw_cfg.includes))
                        if mw_cfg.exceptions and hasattr(factory_or_cls, "exceptions"):
                            factory_or_cls.exceptions = list(getattr(factory_or_cls, "exceptions", []) + mw_cfg.exceptions)
                        if hasattr(factory_or_cls, "activity_executor") and not factory_or_cls.activity_executor:
                            factory_or_cls.activity_executor = self.activity_executor
                        if hasattr(factory_or_cls, "temporal_replicator") and not factory_or_cls.temporal_replicator:
                            factory_or_cls.temporal_replicator = temporal_rep
                        resolved.append(factory_or_cls)
                    elif callable(factory_or_cls):
                        import inspect
                        try:
                            sig = inspect.signature(factory_or_cls)
                            has_varkw = any(p.kind == inspect.Parameter.VAR_KEYWORD for p in sig.parameters.values())
                            call_kwargs = dict(mw_cfg.config)
                            call_kwargs.update({
                                "excludes": mw_cfg.excludes,
                                "includes": mw_cfg.includes,
                                "exceptions": mw_cfg.exceptions,
                                "activity_executor": self.activity_executor,
                                "temporal_replicator": temporal_rep,
                            })
                            if not has_varkw:
                                call_kwargs = {k: v for k, v in call_kwargs.items() if k in sig.parameters}
                            inst = factory_or_cls(**call_kwargs)
                            resolved.append(inst)
                        except Exception:
                            resolved.append(factory_or_cls)
                    else:
                        resolved.append(factory_or_cls)
        return resolved

    # FastAPI Application Factory
    def create_api_app(self, title: str = "OCTOPUS Live Engine", api_key: Optional[str] = None) -> FastAPI:
        app = FastAPI(
            title=title,
            description="Declarative model-driven in-memory synchronous & live workflow execution engine with interactive visualizer",
            version="1.0.0",
            docs_url="/docs",
            openapi_url="/openapi.json"
        )

        @app.on_event("startup")
        async def _on_startup():
            if getattr(self, "embedded_worker_enabled", True):
                await self.start_embedded_worker()

        @app.on_event("shutdown")
        async def _on_shutdown():
            await self.stop_embedded_worker()

        # Wire up resolved middlewares in intuitive top-to-bottom declaration order
        resolved_middlewares = self._resolve_middlewares()
        if self.core is not None:
            for mw in resolved_middlewares:
                if isinstance(mw, LiveMiddleware):
                    mw.core_engine = self.core
        for mw in reversed(resolved_middlewares):
            if hasattr(mw, "dispatch") and callable(getattr(mw, "dispatch")):
                app.middleware("http")(mw.dispatch)
            elif callable(mw):
                app.middleware("http")(mw)

        router = create_live_router(self, self, api_key=api_key, model_registry=self._models)
        app.include_router(router)

        # Custom OpenAPI schema generator configuring Security Schemes dynamically based on active middlewares
        from fastapi.openapi.utils import get_openapi
        from .domain.services.middleware import RouteMatcher

        def custom_openapi():
            if app.openapi_schema:
                return app.openapi_schema

            openapi_schema = get_openapi(
                title=app.title,
                version=app.version,
                description=app.description,
                routes=app.routes,
            )

            # Detect auth mechanisms from resolved middlewares or api_key
            security_schemes: Dict[str, Any] = {}
            bearer_mws = [m for m in resolved_middlewares if "Bearer" in getattr(m, "name", m.__class__.__name__)]
            api_key_mws = [m for m in resolved_middlewares if "ApiKey" in getattr(m, "name", m.__class__.__name__)]

            if bearer_mws:
                security_schemes["HTTPBearer"] = {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT",
                    "description": "Enter your Bearer / JWT token (e.g. obtained via /auth/login)"
                }
            elif api_key or api_key_mws:
                security_schemes["ApiKeyAuth"] = {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-API-Key",
                    "description": "Enter your API Key"
                }

            if security_schemes:
                components = openapi_schema.setdefault("components", {})
                components["securitySchemes"] = security_schemes

                paths = openapi_schema.get("paths", {})
                for path, path_item in paths.items():
                    requires_auth = False
                    path_sec: List[Dict[str, List[str]]] = []

                    if bearer_mws:
                        for m in bearer_mws:
                            ex = getattr(m, "excludes", [])
                            inc = getattr(m, "includes", [])
                            if not RouteMatcher.should_skip(path, ex, inc):
                                requires_auth = True
                                path_sec.append({"HTTPBearer": []})
                                break
                    elif api_key_mws:
                        for m in api_key_mws:
                            ex = getattr(m, "excludes", [])
                            inc = getattr(m, "includes", [])
                            if not RouteMatcher.should_skip(path, ex, inc):
                                requires_auth = True
                                path_sec.append({"ApiKeyAuth": []})
                                break
                    elif api_key:
                        requires_auth = True
                        path_sec.append({"ApiKeyAuth": []})

                    for method_name, op in path_item.items():
                        if isinstance(op, dict):
                            if requires_auth:
                                op["security"] = path_sec
                            else:
                                op["security"] = []

            app.openapi_schema = openapi_schema
            return app.openapi_schema

        app.openapi = custom_openapi
        return app

    def get_fastapi_app(self, title: str = "OCTOPUS Live Engine", version: str = "1.0.0", api_key: Optional[str] = None) -> FastAPI:
        return self.create_api_app(title=title, api_key=api_key)

    def set_embedded_worker_enabled(self, enabled: bool) -> None:
        """Configure whether the embedded Temporal worker runs alongside the API server."""
        self.embedded_worker_enabled = enabled

    def _collect_all_activity_names(self) -> List[str]:
        all_names = set()
        for bp in self.blueprint_repo.list_all():
            for step in bp.steps:
                if step.id:
                    all_names.add(step.id)
                if step.activity:
                    all_names.add(step.activity)
            for ep in bp.api:
                for step in ep.steps:
                    if step.id:
                        all_names.add(step.id)
                    if step.activity:
                        all_names.add(step.activity)
                    if step.condition:
                        for s in step.condition.then_steps:
                            if s.id:
                                all_names.add(s.id)
                            if s.activity:
                                all_names.add(s.activity)
                        for s in step.condition.else_steps:
                            if s.id:
                                all_names.add(s.id)
                            if s.activity:
                                all_names.add(s.activity)
            for m in bp.middlewares:
                if getattr(m, "activity", None):
                    all_names.add(m.activity)
                for exc in getattr(m, "exceptions", []):
                    if exc.activity:
                        all_names.add(exc.activity)
        if isinstance(self.activity_executor, CallableActivityExecutor):
            for name in self.activity_executor._activities.keys():
                all_names.add(name)
        return list(all_names)

    async def start_embedded_worker(self) -> None:
        """Starts the embedded Temporal SDK worker inside the running application event loop."""
        if self.core is not None:
            # A worker only when a blueprint uses Temporal (D13), so the core
            # needs the blueprints first; an unreachable server leaves async
            # steps running in-process.
            for bp in self.blueprint_repo.list_all():
                self.pipeline_manager.sync_blueprint(bp)
            self.pipeline_manager.bind_loop(asyncio.get_running_loop())
            try:
                await asyncio.get_running_loop().run_in_executor(None, self.core.start_worker)
            except Exception as ex:
                logger.warning("OCTOPUS: Temporal worker not started: %s", ex)
            return
        if self.replication_manager and self.replication_manager.temporal_replicator:
            all_names = self._collect_all_activity_names()
            self.replication_manager.temporal_replicator.set_activity_executor(self.activity_executor)
            self.replication_manager.temporal_replicator.register_activity_names(all_names)
            await self.replication_manager.temporal_replicator.connect()

    async def stop_embedded_worker(self) -> None:
        """Stops the embedded Temporal SDK worker."""
        if self.core is not None:
            self.core.stop_worker()
            return
        if self.replication_manager and self.replication_manager.temporal_replicator:
            await self.replication_manager.temporal_replicator.shutdown()

    async def run_worker(self) -> None:
        """Runs a dedicated standalone Temporal worker process asynchronously until cancelled."""
        if self.core is not None:
            for bp in self.blueprint_repo.list_all():
                self.pipeline_manager.sync_blueprint(bp)
            self.pipeline_manager.bind_loop(asyncio.get_running_loop())
            await asyncio.get_running_loop().run_in_executor(None, self.core.start_worker)
            print("👷 Dedicated Temporal worker started on the engine core")
            try:
                await asyncio.Event().wait()
            finally:
                self.core.stop_worker()
            return
        if not self.replication_manager or not self.replication_manager.temporal_replicator:
            raise RuntimeError("Temporal replicator is not configured on this engine.")
        all_names = self._collect_all_activity_names()
        self.replication_manager.temporal_replicator.set_activity_executor(self.activity_executor)
        self.replication_manager.temporal_replicator.register_activity_names(all_names)
        print("\n==================================================================")
        print("👷 Dedicated Standalone Temporal Worker Node Started (Python)")
        print(f"TaskQueue: {self.replication_manager.temporal_replicator.task_queue} -> Polling Temporal Server ({self.replication_manager.temporal_replicator.host})")
        print("==================================================================\n")
        await self.replication_manager.temporal_replicator.run_worker_forever()

    def start_worker(self) -> None:
        """Entrypoint to run a dedicated standalone Temporal worker node synchronously."""
        try:
            asyncio.run(self.run_worker())
        except (KeyboardInterrupt, SystemExit):
            pass

    def start_server(self, port: int = 8000, host: str = "0.0.0.0", **kwargs: Any) -> None:
        """Starts the FastAPI HTTP server with embedded worker."""
        import uvicorn
        app = self.create_api_app(**kwargs)
        uvicorn.run(app, host=host, port=port)

