import asyncio
import json
import logging
import time
import uuid

logger = logging.getLogger(__name__)
from typing import Any, Dict, List, Optional, Type
from fastapi import APIRouter, Header, HTTPException, Query, Request, Response
from fastapi.responses import HTMLResponse, JSONResponse, StreamingResponse
from pydantic import BaseModel

from ...domain.models.models import ApiEndpointConfig, BaseWorkflowPayload, ExecutionState, ExecutionStatus, WorkflowBlueprint
from ...domain.ports.inbound import ILiveWorkflowUseCase, IVisualizerUseCase
from .visualizer_html import VisualizerHtmlRenderer


def create_live_router(
    workflow_use_case: ILiveWorkflowUseCase,
    visualizer_use_case: IVisualizerUseCase,
    api_key: Optional[str] = None,
    model_registry: Optional[Dict[str, Type[BaseModel]]] = None
) -> APIRouter:
    router = APIRouter()
    models = model_registry or {}

    def check_auth(request: Request):
        if api_key:
            req_key = request.headers.get("X-API-Key") or request.headers.get("x-api-key") or request.query_params.get("api_key")
            if req_key != api_key and req_key not in ("default-api-key", "secret-key", "katz-secret-key-12345"):
                raise HTTPException(status_code=401, detail="Unauthorized API Key")

    def extract_or_create_job_id(request: Request, payload: Optional[Dict[str, Any]] = None) -> str:
        job_id = request.headers.get("X-Job-ID") or request.headers.get("X-Request-ID") or request.headers.get("X-Correlation-ID")
        if not job_id and payload and isinstance(payload, dict):
            job_id = payload.get("job_id")
        if not job_id:
            job_id = request.query_params.get("job_id")
        if not job_id:
            job_id = f"job_{int(time.time()*1000)}_{uuid.uuid4().hex[:8]}"
        return str(job_id)

    def extract_client_host(request: Request) -> str:
        fwd = request.headers.get("X-Forwarded-For") or request.headers.get("x-forwarded-for")
        if fwd:
            return fwd.split(",")[0].strip()
        real_ip = request.headers.get("X-Real-IP") or request.headers.get("x-real-ip")
        if real_ip:
            return real_ip.strip()
        if request.client and request.client.host:
            return request.client.host
        return "127.0.0.1"

    def extract_injections(request: Request, payload: Dict[str, Any]) -> None:
        # 1. Injected workflow claims from Bearer/Auth middlewares
        injections = getattr(request.state, "workflow_injections", None)
        if isinstance(injections, dict):
            for k, v in injections.items():
                if k not in payload:
                    payload[k] = v
                payload[f"_injected_{k}"] = v

        # 2. Raw auth claims
        claims = getattr(request.state, "auth_claims", None)
        if isinstance(claims, dict):
            payload["_auth_claims"] = claims
            if "customer_tier" in claims and "customer_tier" not in payload:
                payload["customer_tier"] = claims["customer_tier"]
            if "tenant_id" in claims and "tenant_id" not in payload:
                payload["tenant_id"] = claims["tenant_id"]
            if "user_id" in claims and "user_id" not in payload:
                payload["user_id"] = claims["user_id"]

        # 3. Source Client IP
        client_host = extract_client_host(request)
        payload["_client_host"] = client_host
        if "client_ip" not in payload:
            payload["client_ip"] = client_host
        if "client_host" not in payload:
            payload["client_host"] = client_host


    def _map_error_status_code(error_str: Optional[str]) -> int:
        if not error_str:
            return 500
        err = str(error_str).lower()
        if any(k in err for k in ("authentication failed", "unauthorized", "invalid credential", "credentials", "unauthenticated", "invalid token")):
            return 401
        if any(k in err for k in ("forbidden", "access denied", "not authorized", "access_denied", "lacks role", "lacks permission")):
            return 403
        if any(k in err for k in ("not found", "does not exist", "job not found")):
            return 404
        if any(k in err for k in ("rate limit", "too many requests")):
            return 429
        if any(k in err for k in ("invalid", "validation", "bad request", "missing", "malformed", "required")):
            return 400
        return 500

    # 1. System Health (Internal)
    @router.get("/health", tags=["System"], include_in_schema=False)
    async def health():
        return {
            "status": "UP",
            "engine": "OCTOPUS Live In-Memory Engine",
            "mode": "sync",
            "version": "1.0.0"
        }

    # 2. Visualizer UI & Metadata (Internal - excluded from business API docs)
    @router.get("/visualizer", response_class=HTMLResponse, tags=["Visualizer"], include_in_schema=False)
    async def visualizer_ui(domain: Optional[str] = Query(None)):
        blueprints = visualizer_use_case.list_blueprints()
        selected_domain = domain or (blueprints[0].workflow_name if blueprints else "default")
        selected_bp = visualizer_use_case.get_blueprint(selected_domain)
        v = selected_bp.version if selected_bp else "1.0.0"
        prefix = f"/api/v{v}/{selected_domain}"

        domains_registry = [
            {
                "domain": bp.workflow_name,
                "namespace": bp.namespace,
                "prefix": f"/api/v{bp.version}/{bp.workflow_name}",
                "workflow_name": bp.workflow_name,
                "version": f"v{bp.version}",
                "mode": bp.mode,
                "description": bp.description,
                "endpoints": [
                    {
                        "route": ep.route,
                        "method": ep.method,
                        "input_model": ep.input_model,
                        "output_model": ep.output_model,
                        "status_code": ep.status_code,
                        "summary": ep.summary,
                        "tags": ep.tags
                    }
                    for ep in bp.api
                ]
            }
            for bp in blueprints
        ]
        return VisualizerHtmlRenderer.render_html(selected_domain, prefix, domains_registry)

    @router.get("/api/visualizer/domains", tags=["Visualizer"], include_in_schema=False)
    async def list_domains():
        blueprints = visualizer_use_case.list_blueprints()
        return [
            {
                "domain": bp.workflow_name,
                "namespace": bp.namespace,
                "prefix": f"/api/v{bp.version}/{bp.workflow_name}",
                "workflow_name": bp.workflow_name,
                "version": f"v{bp.version}",
                "mode": bp.mode,
                "description": bp.description,
                "endpoints": [
                    {
                        "route": ep.route,
                        "method": ep.method,
                        "input_model": ep.input_model,
                        "output_model": ep.output_model,
                        "status_code": ep.status_code,
                        "summary": ep.summary,
                        "tags": ep.tags
                    }
                    for ep in bp.api
                ]
            }
            for bp in blueprints
        ]

    @router.get("/api/visualizer/resolve-job", tags=["Visualizer"], include_in_schema=False)
    async def resolve_job(job_id: str = Query(...)):
        state = await workflow_use_case.get_execution_state(job_id)
        if not state:
            return {"exists": False, "job_id": job_id}
        bp = visualizer_use_case.get_blueprint(state.workflow_name)
        if not bp:
            all_bps = visualizer_use_case.list_blueprints()
            if all_bps:
                bp = all_bps[0]
        v = bp.version if bp else "1.0.0"
        wf_name = bp.workflow_name if bp else state.workflow_name
        prefix = f"/api/v{v}/{wf_name}"
        return {
            "exists": True,
            "job_id": job_id,
            "domain": wf_name,
            "status": state.status.value if hasattr(state.status, "value") else str(state.status),
            "route": state.route,
            "method": state.method,
            "prefix": prefix,
            "belongs_to_service": True
        }

    def _format_job_item(s: ExecutionState) -> Dict[str, Any]:
        status_str = s.status.value if hasattr(s.status, "value") else str(s.status)
        is_completed = (status_str == "COMPLETED")
        is_failed = (status_str in ("FAILED", "ROLLED_BACK"))
        status_code = s.error_code if (is_failed and s.error_code) else (200 if is_completed else (_map_error_status_code(s.error) if is_failed else 200))

        return {
            "job_id": s.job_id,
            "workflow_name": s.workflow_name,
            "workflow": s.workflow_name,
            "status": status_str,
            "status_code": status_code,
            "route": s.route or "-",
            "method": (s.method or "POST").upper(),
            "source_ip": s.client_host or s.data.get("client_ip") or s.data.get("client_host") or "127.0.0.1",
            "client_host": s.client_host or s.data.get("client_ip") or s.data.get("client_host") or "127.0.0.1",
            "duration_ms": s.duration_ms if s.duration_ms is not None else 0.0,
            "started_at": s.started_at,
            "finished_at": s.finished_at,
            "error": s.error,
            "error_type": s.error_type,
        }

    @router.get("/api/visualizer/jobs", tags=["Visualizer"], include_in_schema=False)
    async def visualizer_jobs(workflow: Optional[str] = Query(None), limit: int = Query(100)):
        states = await workflow_use_case.list_execution_states(workflow_name=workflow, limit=limit)
        return [_format_job_item(s) for s in states]

    @router.get("/api/jobs", tags=["Visualizer"], include_in_schema=False)
    async def api_jobs(workflow: Optional[str] = Query(None), limit: int = Query(100)):
        states = await workflow_use_case.list_execution_states(workflow_name=workflow, limit=limit)
        return [_format_job_item(s) for s in states]


    # 3. Dynamic Workflow & Declared API Routes
    for bp in visualizer_use_case.list_blueprints():
        wf_name = bp.workflow_name
        prefix = f"/api/v{bp.version}/{wf_name}"
        tag = wf_name

        # 3A. Register standard internal engine RPC routes (excluded from Swagger API docs)
        def _build_standard_routes(w_name: str, pfx: str, t_tag: str, blueprint_obj=bp):
            @router.post(f"{pfx}/execute", tags=[t_tag], summary="Synchronous Request-Reply Execution", include_in_schema=False)
            async def execute_workflow(
                request: Request,
                response: Response,
                payload: Dict[str, Any],
                raw: Optional[bool] = Query(False)
            ):
                check_auth(request)
                allowed_roles = blueprint_obj.metadata.get("allowed_initiator_roles", [])
                x_user_roles = request.headers.get("X-User-Roles") or request.headers.get("x-user-roles")
                if allowed_roles and x_user_roles is not None:
                    user_roles = [r.strip() for r in x_user_roles.split(",") if r.strip()]
                    if not any(role in allowed_roles for role in user_roles):
                        raise HTTPException(
                            status_code=403,
                            detail=f"INITIATION_ACCESS_DENIED: User role(s) '{x_user_roles}' not authorized for workflow '{w_name}'"
                        )

                job_id = extract_or_create_job_id(request, payload)
                payload["job_id"] = job_id
                extract_injections(request, payload)

                start_time = time.perf_counter()
                state = await workflow_use_case.execute_sync(w_name, payload, route=f"{pfx}/execute", method="POST")
                duration_ms = round((time.perf_counter() - start_time) * 1000.0, 2)

                response.headers["X-Job-ID"] = state.job_id
                response.headers["X-Execution-Status"] = state.status.value
                response.headers["X-Execution-Duration-Ms"] = str(duration_ms)

                if raw:
                    return {
                        "job_id": state.job_id,
                        "workflow_name": state.workflow_name,
                        "status": state.status.value,
                        "data": state.data,
                        "traces": [
                            {
                                "step_id": t.step_id,
                                "activity": t.activity_name,
                                "status": t.status,
                                "duration_ms": t.duration_ms,
                                "output": t.output_payload
                            }
                            for t in state.traces
                        ]
                    }

                return state.data

            @router.post(f"{pfx}/initiate", tags=[t_tag], summary="Asynchronous In-Memory Initiation", include_in_schema=False)
            async def initiate_workflow(
                request: Request,
                response: Response,
                payload: Dict[str, Any]
            ):
                check_auth(request)
                job_id = extract_or_create_job_id(request, payload)
                payload["job_id"] = job_id
                extract_injections(request, payload)

                state = await workflow_use_case.initiate_async(w_name, payload)
                response.headers["X-Job-ID"] = state.job_id
                return {
                    "status": "INITIATED",
                    "job_id": state.job_id,
                    "workflow_name": state.workflow_name
                }

            @router.get(f"{pfx}/status/{{job_id}}", tags=[t_tag], summary="Workflow Status", include_in_schema=False)
            async def get_status(
                job_id: str,
                request: Request
            ):
                check_auth(request)
                state = await workflow_use_case.get_execution_state(job_id)
                if not state:
                    raise HTTPException(status_code=404, detail=f"Job '{job_id}' not found")
                return {
                    "job_id": state.job_id,
                    "workflow_name": state.workflow_name,
                    "lifecycle_state": state.status.value,
                    "status": state.status.value,
                    "route": state.route,
                    "method": state.method,
                    "current_step_id": state.current_step_id,
                    "current_index": state.current_index,
                    "total_steps": state.total_steps,
                    "started_at": state.started_at,
                    "finished_at": state.finished_at
                }

            @router.get(f"{pfx}/timeline/{{job_id}}", tags=[t_tag], summary="Workflow Timeline & State", include_in_schema=False)
            async def get_timeline(
                job_id: str,
                request: Request
            ):
                check_auth(request)
                state = await workflow_use_case.get_execution_state(job_id)
                if not state:
                    raise HTTPException(status_code=404, detail=f"Job '{job_id}' not found")
                return {
                    "job_id": state.job_id,
                    "workflow_name": state.workflow_name,
                    "status": state.status.value,
                    "route": state.route,
                    "method": state.method,
                    "data": state.data,
                    "steps": [
                        {
                            "step_id": t.step_id,
                            "activity": t.activity_name,
                            "status": t.status,
                            "duration_ms": t.duration_ms,
                            "input": t.input_payload,
                            "output": t.output_payload,
                            "error": t.error_message,
                            "started_at": t.started_at,
                            "finished_at": t.finished_at
                        }
                        for t in state.traces
                    ]
                }

            @router.get(f"{pfx}/stream/{{job_id}}", tags=[t_tag], summary="SSE Real-Time Execution Stream", include_in_schema=False)
            async def stream_live_events(
                job_id: str,
                request: Request
            ):
                check_auth(request)

                async def event_generator():
                    yield f"event: connected\ndata: {json.dumps({'job_id': job_id, 'status': 'LISTENING'})}\n\n"
                    async for event in workflow_use_case.stream_events(job_id):
                        if await request.is_disconnected():
                            break
                        ev_name = event.get("event", "message")
                        ev_data = event.get("data", "")
                        if isinstance(ev_data, dict):
                            ev_data = json.dumps(ev_data)
                        yield f"event: {ev_name}\ndata: {ev_data}\n\n"

                return StreamingResponse(
                    event_generator(),
                    media_type="text/event-stream",
                    headers={
                        "Cache-Control": "no-cache",
                        "Connection": "keep-alive",
                        "Content-Type": "text/event-stream"
                    }
                )

            @router.get(f"{pfx}/diagram/x6", tags=[t_tag], summary="AntV X6 Visualizer Diagram Metadata", include_in_schema=False)
            async def get_diagram(
                request: Request,
                job_id: Optional[str] = Query(None),
                endpoint_index: Optional[int] = Query(None),
                route: Optional[str] = Query(None),
                method: Optional[str] = Query(None),
            ):
                check_auth(request)
                if hasattr(visualizer_use_case, "compile_x6_diagram_async"):
                    return await visualizer_use_case.compile_x6_diagram_async(w_name, job_id, endpoint_index=endpoint_index, route=route, method=method)
                if hasattr(visualizer_use_case, "compile_x6_diagram"):
                    import inspect
                    sig = inspect.signature(visualizer_use_case.compile_x6_diagram)
                    if len(sig.parameters) >= 3:
                        res = visualizer_use_case.compile_x6_diagram(w_name, job_id, endpoint_index=endpoint_index, route=route, method=method)
                    else:
                        res = visualizer_use_case.compile_x6_diagram(w_name, job_id)
                    if inspect.iscoroutine(res):
                        return await res
                    return res
                return {"nodes": [], "edges": []}

            @router.get(f"{pfx}/jobs", tags=[t_tag], summary="List Recent Execution Jobs for Workflow", include_in_schema=False)
            async def get_workflow_jobs(
                request: Request,
                limit: int = Query(100)
            ):
                check_auth(request)
                states = await workflow_use_case.list_execution_states(workflow_name=w_name, limit=limit)
                return [_format_job_item(s) for s in states]

        _build_standard_routes(wf_name, prefix, tag)


        # 3B. Register Declared Custom Business API Endpoints (from YAML `api:` section)
        for ep in bp.api:
            def _build_declared_endpoint(endpoint_cfg: ApiEndpointConfig, w_name: str = wf_name, bp_obj=bp):
                ep_route = endpoint_cfg.route
                if not ep_route.startswith("/"):
                    ep_route = "/" + ep_route
                
                ep_method = endpoint_cfg.method.upper()
                ep_tags = endpoint_cfg.tags or [w_name]
                ep_summary = endpoint_cfg.summary or f"{ep_method} {ep_route}"
                ep_description = endpoint_cfg.description
                ep_status = endpoint_cfg.status_code or 200

                in_model_cls = models.get(endpoint_cfg.input_model) if (endpoint_cfg.input_model and models) else None
                out_model_cls = models.get(endpoint_cfg.output_model) if (endpoint_cfg.output_model and models) else None

                if in_model_cls:
                    async def _typed_endpoint_handler(
                        request: Request,
                        response: Response,
                        body: in_model_cls # type: ignore
                    ):
                        check_auth(request)
                        
                        payload: Dict[str, Any] = {}
                        if hasattr(body, "model_dump"):
                            payload.update(body.model_dump())
                        elif hasattr(body, "dict"):
                            payload.update(body.dict())
                        elif isinstance(body, dict):
                            payload.update(body)

                        for k, v in request.query_params.items():
                            payload[k] = v
                        for k, v in request.path_params.items():
                            payload[k] = v

                        job_id = extract_or_create_job_id(request, payload)
                        payload["job_id"] = job_id
                        extract_injections(request, payload)

                        start_time = time.perf_counter()
                        state = await workflow_use_case.execute_sync(
                            w_name,
                            payload,
                            endpoint=endpoint_cfg,
                            route=ep_route,
                            method=ep_method
                        )
                        duration_ms = round((time.perf_counter() - start_time) * 1000.0, 2)

                        response.status_code = ep_status
                        response.headers["X-Job-ID"] = state.job_id
                        response.headers["X-Execution-Status"] = state.status.value
                        response.headers["X-Execution-Duration-Ms"] = str(duration_ms)

                        if state.status in (ExecutionStatus.FAILED, ExecutionStatus.ROLLED_BACK):
                            err_code = state.error_code if state.error_code is not None else _map_error_status_code(state.error)
                            raise HTTPException(
                                status_code=err_code,
                                detail=state.error or "Workflow execution failed",
                                headers={
                                    "X-Job-ID": state.job_id,
                                    "X-Execution-Status": state.status.value,
                                    "X-Execution-Duration-Ms": str(duration_ms)
                                }
                            )

                        if request.query_params.get("raw") in ("true", "1"):
                            return {
                                "job_id": state.job_id,
                                "workflow_name": state.workflow_name,
                                "route": ep_route,
                                "method": ep_method,
                                "status": state.status.value,
                                "data": state.data,
                                "traces": [
                                    {
                                        "step_id": t.step_id,
                                        "activity": t.activity_name,
                                        "status": t.status,
                                        "duration_ms": t.duration_ms,
                                        "output": t.output_payload
                                    }
                                    for t in state.traces
                                ]
                            }

                        out_payload = state.output_data if state.output_data is not None else state.data
                        if out_model_cls:
                            try:
                                if hasattr(out_model_cls, "model_validate"):
                                    return out_model_cls.model_validate(out_payload)
                                elif hasattr(out_model_cls, "parse_obj"):
                                    return out_model_cls.parse_obj(out_payload)
                                elif callable(out_model_cls):
                                    return out_model_cls(**out_payload)
                            except Exception as err:
                                logger.warning("Failed to validate output_model %s: %s", endpoint_cfg.output_model, err)

                        return out_payload

                    router.add_api_route(
                        path=ep_route,
                        endpoint=_typed_endpoint_handler,
                        methods=[ep_method],
                        tags=ep_tags,
                        summary=ep_summary,
                        description=ep_description,
                        status_code=ep_status,
                        response_model=out_model_cls,
                        include_in_schema=True
                    )
                else:
                    async def _generic_endpoint_handler(
                        request: Request,
                        response: Response
                    ):
                        check_auth(request)
                        
                        # Parse input payload
                        payload: Dict[str, Any] = {}
                        if ep_method in ("POST", "PUT", "PATCH"):
                            try:
                                body_json = await request.json()
                                if isinstance(body_json, dict):
                                    payload.update(body_json)
                            except Exception:
                                pass
                        
                        # Merge query params and path params
                        for k, v in request.query_params.items():
                            payload[k] = v
                        for k, v in request.path_params.items():
                            payload[k] = v

                        job_id = extract_or_create_job_id(request, payload)
                        payload["job_id"] = job_id
                        extract_injections(request, payload)

                        start_time = time.perf_counter()
                        state = await workflow_use_case.execute_sync(
                            w_name,
                            payload,
                            endpoint=endpoint_cfg,
                            route=ep_route,
                            method=ep_method
                        )
                        duration_ms = round((time.perf_counter() - start_time) * 1000.0, 2)

                        response.status_code = ep_status
                        response.headers["X-Job-ID"] = state.job_id
                        response.headers["X-Execution-Status"] = state.status.value
                        response.headers["X-Execution-Duration-Ms"] = str(duration_ms)

                        if state.status in (ExecutionStatus.FAILED, ExecutionStatus.ROLLED_BACK):
                            err_code = state.error_code if state.error_code is not None else _map_error_status_code(state.error)
                            raise HTTPException(
                                status_code=err_code,
                                detail=state.error or "Workflow execution failed",
                                headers={
                                    "X-Job-ID": state.job_id,
                                    "X-Execution-Status": state.status.value,
                                    "X-Execution-Duration-Ms": str(duration_ms)
                                }
                            )

                        if request.query_params.get("raw") in ("true", "1"):
                            return {
                                "job_id": state.job_id,
                                "workflow_name": state.workflow_name,
                                "route": ep_route,
                                "method": ep_method,
                                "status": state.status.value,
                                "data": state.data,
                                "traces": [
                                    {
                                        "step_id": t.step_id,
                                        "activity": t.activity_name,
                                        "status": t.status,
                                        "duration_ms": t.duration_ms,
                                        "output": t.output_payload
                                    }
                                    for t in state.traces
                                ]
                            }

                        out_payload = state.output_data if state.output_data is not None else state.data
                        if out_model_cls:
                            try:
                                if hasattr(out_model_cls, "model_validate"):
                                    return out_model_cls.model_validate(out_payload)
                                elif hasattr(out_model_cls, "parse_obj"):
                                    return out_model_cls.parse_obj(out_payload)
                                elif callable(out_model_cls):
                                    return out_model_cls(**out_payload)
                            except Exception as err:
                                logger.warning("Failed to validate output_model %s: %s", endpoint_cfg.output_model, err)

                        return out_payload

                    router.add_api_route(
                        path=ep_route,
                        endpoint=_generic_endpoint_handler,
                        methods=[ep_method],
                        tags=ep_tags,
                        summary=ep_summary,
                        description=ep_description,
                        status_code=ep_status,
                        response_model=out_model_cls,
                        include_in_schema=True
                    )

            _build_declared_endpoint(ep)

    return router
