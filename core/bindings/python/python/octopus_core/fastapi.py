"""Mounts an engine's declared `api:` routes and the visualizer's diagram on
FastAPI. The core decides (job IDs, masking, failures, tooling access); this
module only moves requests and answers.

Spike scope: declared routes and `/diagram/x6`. Middleware, the visualizer
page, history and live streams come with the phase 3 SDK.
"""

import ipaddress
import json
from typing import Any, Dict, Optional

from fastapi import FastAPI, Request
from fastapi.concurrency import run_in_threadpool
from fastapi.responses import JSONResponse, Response

from . import Engine

# Starlette's TestClient reports this host; it is in-process, so local.
_IN_PROCESS_HOSTS = {"testclient"}


def _is_loopback(host: Optional[str]) -> bool:
    if not host:
        return False
    if host in _IN_PROCESS_HOSTS:
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return host == "localhost"


async def _body(request: Request) -> Optional[Dict[str, Any]]:
    raw = await request.body()
    if not raw.strip():
        return {}
    try:
        parsed = json.loads(raw)
    except ValueError:
        return None
    if parsed is None:
        return {}
    return parsed if isinstance(parsed, dict) else None


def create_app(engine: Engine, title: str = "OCTOPUS Live Engine (core)", run_in_thread: bool = True) -> FastAPI:
    """A FastAPI app serving `engine`'s declared routes.

    With `run_in_thread`, each run happens on Starlette's thread pool, so the
    event loop stays free while the core works (it releases the GIL).
    """
    app = FastAPI(title=title)

    for r in engine.declared_routes():
        method, route, has_body = r["method"], r["route"], r["has_body"]

        def make_handler(method: str = method, route: str = route, has_body: bool = has_body):
            async def handler(request: Request) -> Response:
                payload: Dict[str, Any] = {}
                if has_body:
                    parsed = await _body(request)
                    if parsed is None:
                        return JSONResponse(
                            {"error": "MalformedJsonException", "detail": "Malformed JSON syntax in request body.", "status_code": 400},
                            status_code=400,
                        )
                    payload = parsed
                payload.update(request.query_params)
                payload.update(request.path_params)
                h = request.headers
                req = {
                    "payload": payload,
                    "job_id_candidates": [
                        h.get("X-Job-ID", ""),
                        h.get("X-Request-ID", ""),
                        h.get("X-Correlation-ID", ""),
                        payload.get("job_id") if isinstance(payload.get("job_id"), str) else "",
                        request.query_params.get("job_id", ""),
                    ],
                    "client_ip": request.client.host if request.client else "",
                    "raw": request.query_params.get("raw") in ("true", "1"),
                }
                if run_in_thread:
                    status, headers, body = await run_in_threadpool(engine.call_route, method, route, req)
                else:
                    status, headers, body = engine.call_route(method, route, req)
                return JSONResponse(body, status_code=status, headers=headers)

            return handler

        # Starlette's path parameters use OpenAPI's {name} form too.
        app.add_api_route(route, make_handler(), methods=[method], summary=r.get("summary") or None)

    def tooling_denied(request: Request) -> Optional[Response]:
        decision, detail = engine.tooling_decision(False, _is_loopback(request.client.host if request.client else None))
        if decision == "allow":
            return None
        if decision == "not_found":
            return Response(status_code=404)
        if decision == "require_api_key":
            if engine.api_key_allows(request.headers.get("X-API-Key", ""), request.query_params.get("api_key", "")):
                return None
            return JSONResponse({"error": "Unauthorized API Key"}, status_code=401)
        return JSONResponse({"error": "ToolingAccessDenied", "detail": detail}, status_code=403)

    @app.get("/api/v{version}/{workflow}/diagram/x6", include_in_schema=False)
    async def diagram(request: Request, version: str, workflow: str, job_id: Optional[str] = None) -> Response:
        denied = tooling_denied(request)
        if denied is not None:
            return denied
        try:
            return JSONResponse(engine.compile_diagram(workflow, job_id))
        except KeyError as e:
            return JSONResponse({"error": str(e)}, status_code=404)

    return app
