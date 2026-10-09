import asyncio
import fnmatch
import ipaddress
import json
import logging
import re
import time
import uuid
from datetime import datetime, timezone
from typing import Any, Awaitable, Callable, Dict, List, Optional, Set, Union
from fastapi import Request, Response
from fastapi.responses import JSONResponse
from starlette.middleware.base import BaseHTTPMiddleware, RequestResponseEndpoint
from starlette.types import ASGIApp

from ..models.models import MiddlewareExceptionConfig, ExecutionState, ExecutionStatus, StepExecutionTrace

logger = logging.getLogger("OctopusLive.Middleware")


# Built-in Standard Middleware Exceptions
class OctopusMiddlewareException(Exception):
    """Base exception for all Octopus middleware violations."""
    default_code: int = 400
    default_message: str = "Middleware violation"

    def __init__(
        self,
        message: Optional[str] = None,
        code: Optional[int] = None,
        details: Optional[Dict[str, Any]] = None,
        headers: Optional[Dict[str, str]] = None
    ):
        self.message = message or self.default_message
        self.code = code or self.default_code
        self.details = details or {}
        self.headers = headers or {}
        super().__init__(self.message)


class InvalidCredentials(OctopusMiddlewareException):
    default_code = 401
    default_message = "Invalid username or password. Please try again."

class UserNotAuthenticated(OctopusMiddlewareException):
    default_code = 401
    default_message = "User is not authenticated or token is missing/invalid"

class UserNotAuthorized(OctopusMiddlewareException):
    default_code = 403
    default_message = "User lacks authorization or required permissions"


class InvalidApiKeyException(OctopusMiddlewareException):
    default_code = 401
    default_message = "Invalid or missing API key"


class RateLimitExceeded(OctopusMiddlewareException):
    default_code = 429
    default_message = "Too many requests. Rate limit exceeded"


class InvalidClaimException(OctopusMiddlewareException):
    default_code = 403
    default_message = "Required claim is missing or invalid"


class IpAccessDenied(OctopusMiddlewareException):
    default_code = 403
    default_message = "Access denied: Client IP is not in the allowed whitelist"


# 400 Bad Request & Validation Exceptions
class BadRequestException(OctopusMiddlewareException):
    default_code = 400
    default_message = "Invalid or malformed request payload. Please verify syntax and parameters."


class MalformedJsonException(BadRequestException):
    default_code = 400
    default_message = "Malformed JSON syntax in request body."


class ValidationException(BadRequestException):
    default_code = 400
    default_message = "Request validation rules failed."


class MissingRequiredFieldException(ValidationException):
    default_code = 400
    default_message = "Missing required field in request payload."


class PayloadTooLargeException(OctopusMiddlewareException):
    default_code = 413
    default_message = "Request payload exceeds maximum permitted size limit."


class RouteMatcher:
    """Utility to evaluate route exclusion and inclusion rules using exact matches, wildcards, or regex."""

    SYSTEM_EXCLUDES = [
        "/favicon.ico",
        "/favicon.png",
        "/robots.txt",
        "/health*",
        "/visualizer*",
        "/api/visualizer*",
        "*/diagram*",
        "*/diagram/x6*",
        "/docs*",
        "/openapi.json",
        "/redoc*"
    ]

    @staticmethod
    def matches(path: str, pattern: str) -> bool:
        if not pattern or not path:
            return False
        # Normalize leading slashes
        clean_path = path.rstrip("/") if path != "/" else path
        clean_pat = pattern.rstrip("/") if pattern != "/" else pattern

        # Exact match
        if clean_path == clean_pat or clean_path.lower() == clean_pat.lower():
            return True

        # Wildcard / glob match (e.g. /visualizer*, /api/v1/auth/*, */diagram*)
        if "*" in pattern or "?" in pattern:
            if fnmatch.fnmatch(path, pattern) or fnmatch.fnmatch(clean_path, clean_pat) or fnmatch.fnmatch(path.lower(), pattern.lower()):
                return True

        # Prefix match (e.g. /health matches /health/ready if configured as /health)
        if pattern.endswith("*"):
            prefix = pattern[:-1]
            if path.startswith(prefix) or path.lower().startswith(prefix.lower()):
                return True

        return False

    @staticmethod
    def should_skip(path: str, excludes: List[str], includes: Optional[List[str]] = None) -> bool:
        # 1. Automatic system infrastructure route exclusion (visualizer, swagger, health, favicon)
        if any(RouteMatcher.matches(path, sys_exc) for sys_exc in RouteMatcher.SYSTEM_EXCLUDES):
            return True

        # If includes are defined and path doesn't match any include, skip it (meaning do NOT apply middleware)
        if includes and len(includes) > 0:
            included = any(RouteMatcher.matches(path, inc) for inc in includes)
            if not included:
                return True

        # If path matches any exclude pattern, skip middleware execution
        if excludes and len(excludes) > 0:
            if any(RouteMatcher.matches(path, exc) for exc in excludes):
                return True

        return False


class LiveMiddleware(BaseHTTPMiddleware):
    """Base class for extensible Octopus Live Engine middlewares with route inclusion/exclusion support,
    declarative exception handling, and custom handler activity dispatching."""

    def __init__(
        self,
        app: Optional[ASGIApp] = None,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        temporal_replicator: Optional[Any] = None,
        state_store: Optional[Any] = None,
        durable_store: Optional[Any] = None,
        name: Optional[str] = None,
        **config: Any
    ):
        super().__init__(app) if app is not None else None
        self.excludes = list(excludes or [])
        self.includes = list(includes or [])
        self.exceptions = list(exceptions or [])
        self.activity_executor = activity_executor
        self.temporal_replicator = temporal_replicator
        self.state_store = state_store
        self.durable_store = durable_store
        self.name = name or self.__class__.__name__
        self.config = config

    def should_skip(self, path: str, method: str = "GET") -> bool:
        return RouteMatcher.should_skip(path, self.excludes, self.includes)

    def find_exception_config(self, exc: Exception) -> Optional[MiddlewareExceptionConfig]:
        exc_cls_name = exc.__class__.__name__
        # 1. Exact match on class name
        for ec in self.exceptions:
            if ec.exception == exc_cls_name:
                return ec
        # 2. Case-insensitive match
        for ec in self.exceptions:
            if ec.exception.lower() == exc_cls_name.lower():
                return ec
        # 3. Subclass check
        for ec in self.exceptions:
            for base in exc.__class__.__mro__:
                if base.__name__ == ec.exception:
                    return ec
        return None

    async def handle_exception(self, request: Request, exc: Exception) -> Response:
        exc_name = exc.__class__.__name__
        exc_cfg = self.find_exception_config(exc)

        # Determine default status code, message, headers, details
        if isinstance(exc, OctopusMiddlewareException):
            status_code = exc.code
            message = exc.message
            headers = dict(exc.headers)
            details = dict(exc.details)
        else:
            status_code = 500
            message = str(exc) or "Internal middleware error"
            headers = {}
            details = {}

        # Apply declarative YAML overrides if configured
        if exc_cfg:
            if exc_cfg.code:
                status_code = exc_cfg.code
            if exc_cfg.message:
                message = exc_cfg.message
            if exc_cfg.headers:
                headers.update(exc_cfg.headers)

            # Execute handler activity if declared
            if exc_cfg.activity and self.activity_executor:
                try:
                    activity_payload = {
                        "exception": exc_name,
                        "error": exc_name,
                        "message": message,
                        "original_error": str(exc),
                        "status_code": status_code,
                        "path": request.url.path,
                        "method": request.method,
                        "headers": dict(request.headers),
                        "query_params": dict(request.query_params),
                        "client_host": request.client.host if request.client else None,
                        "details": details,
                    }
                    if hasattr(request.state, "auth_claims"):
                        activity_payload["auth_claims"] = request.state.auth_claims

                    if exc_cfg.is_async:
                        # Non-blocking asynchronous execution in background
                        asyncio.create_task(self._run_handler_activity_safe(exc_cfg.activity, activity_payload))
                    else:
                        act_result = await self._run_handler_activity(exc_cfg.activity, activity_payload)
                        if isinstance(act_result, dict):
                            if "status_code" in act_result:
                                status_code = int(act_result["status_code"])
                            if "code" in act_result:
                                status_code = int(act_result["code"])
                            if "message" in act_result:
                                message = str(act_result["message"])
                            if "detail" in act_result:
                                message = str(act_result["detail"])
                            if "body" in act_result and isinstance(act_result["body"], dict):
                                return JSONResponse(
                                    status_code=status_code,
                                    content=act_result["body"],
                                    headers=headers or None
                                )
                            details.update({k: v for k, v in act_result.items() if k not in ("status_code", "code", "message", "detail")})
                except Exception as act_err:
                    logger.warning("Handler activity '%s' execution failed: %s", exc_cfg.activity, act_err)

        # Track execution failure state in state store
        job_id = getattr(request.state, "job_id", None) or request.headers.get("X-Job-ID") or request.headers.get("x-correlation-id") or f"job-mw-{int(time.time()*1000)}"
        if headers is None:
            headers = {}
        headers["X-Job-ID"] = job_id

        if self.state_store or self.durable_store:
            try:
                path_parts = request.url.path.strip("/").split("/")
                wf_name = "workflow"
                for idx, seg in enumerate(path_parts):
                    if seg.startswith("v") and "." in seg and idx + 1 < len(path_parts):
                        wf_name = path_parts[idx + 1]
                        break
                    elif seg == "api" and idx + 1 < len(path_parts) and not path_parts[idx + 1].startswith("v"):
                        wf_name = path_parts[idx + 1]
                        break

                trace_guard = StepExecutionTrace(
                    step_id="ingress_guards",
                    activity_name=self.name,
                    status="FAILED",
                    input_payload={"error": exc_name, "message": message, "client_host": request.client.host if request.client else None},
                    error_message=message,
                    started_at=datetime.now(timezone.utc).isoformat(),
                    finished_at=datetime.now(timezone.utc).isoformat(),
                    duration_ms=0.1
                )
                mw_traces = [trace_guard]
                if exc_cfg and exc_cfg.activity:
                    trace_act = StepExecutionTrace(
                        step_id=f"exc_{exc_cfg.activity.lower()}",
                        activity_name=exc_cfg.activity,
                        status="COMPLETED",
                        input_payload={"exception": exc_name, "error": exc_name, "status_code": status_code, "message": message, "step_id": "ingress_guards"},
                        output_payload={"audit_logged": True},
                        started_at=datetime.now(timezone.utc).isoformat(),
                        finished_at=datetime.now(timezone.utc).isoformat(),
                        duration_ms=0.1
                    )
                    mw_traces.append(trace_act)

                mw_state = ExecutionState(
                    job_id=job_id,
                    workflow_name=wf_name,
                    status=ExecutionStatus.FAILED,
                    route=request.url.path,
                    method=request.method,
                    current_step_id="ingress_guards",
                    error=message,
                    error_code=status_code,
                    error_type=exc_name,
                    data={"exception": exc_name, "error": exc_name, "status_code": status_code, "path": request.url.path, "method": request.method, "client_host": request.client.host if request.client else None},
                    started_at=datetime.now(timezone.utc).isoformat(),
                    finished_at=datetime.now(timezone.utc).isoformat(),
                    traces=mw_traces
                )
                if self.state_store:
                    await self.state_store.save_state(mw_state)
                if self.durable_store:
                    await self.durable_store.save_state(mw_state)
            except Exception as st_err:
                logger.debug("Failed saving middleware failure execution state: %s", st_err)

        logger.warning(
            "Middleware '%s' violation on %s %s: [%d] %s (%s)",
            self.name,
            request.method,
            request.url.path,
            status_code,
            message,
            exc_name
        )

        response_body = {
            "error": exc_name,
            "detail": message,
            "status_code": status_code
        }
        if details:
            response_body["details"] = details

        return JSONResponse(
            status_code=status_code,
            content=response_body,
            headers=headers or None
        )

    async def _run_handler_activity(self, activity_name: str, payload: Dict[str, Any]) -> Any:
        if not self.activity_executor:
            return None
        import inspect
        if hasattr(self.activity_executor, "execute_activity"):
            return await self.activity_executor.execute_activity(activity_name, payload)
        elif hasattr(self.activity_executor, "execute_async"):
            return await self.activity_executor.execute_async(activity_name, payload)
        elif hasattr(self.activity_executor, "execute"):
            if inspect.iscoroutinefunction(self.activity_executor.execute):
                return await self.activity_executor.execute(activity_name, payload)
            else:
                return self.activity_executor.execute(activity_name, payload)
        elif callable(self.activity_executor):
            if inspect.iscoroutinefunction(self.activity_executor):
                return await self.activity_executor(activity_name, payload)
            else:
                return self.activity_executor(activity_name, payload)
        return None

    async def _run_handler_activity_safe(self, activity_name: str, payload: Dict[str, Any]) -> None:
        try:
            if self.temporal_replicator and hasattr(self.temporal_replicator, "dispatch_activity"):
                await self.temporal_replicator.dispatch_activity(activity_name, payload)
            else:
                await self._run_handler_activity(activity_name, payload)
        except Exception as act_err:
            logger.warning("Background handler activity '%s' execution failed: %s", activity_name, act_err)


    async def dispatch(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        if self.should_skip(request.url.path, request.method):
            return await call_next(request)
        try:
            return await self.process(request, call_next)
        except Exception as exc:
            return await self.handle_exception(request, exc)

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        """Override in subclasses to implement middleware behavior."""
        return await call_next(request)


class BearerAuthAndClaimsMiddleware(LiveMiddleware):
    """Bearer Token / JWT validation middleware with automatic token claims extraction and workflow payload enrichment."""

    def __init__(
        self,
        token_validator: Optional[Callable[[str], Dict[str, Any]]] = None,
        claims_mapper: Optional[Callable[[Dict[str, Any]], Dict[str, Any]]] = None,
        token_header: str = "Authorization",
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "BearerAuthAndClaimsMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        self.token_validator = token_validator or self._default_validator
        self.claims_mapper = claims_mapper or (lambda claims: dict(claims))
        self.token_header = token_header

    def _default_validator(self, token: str) -> Dict[str, Any]:
        if not token or token == "invalid-token":
            raise ValueError("Invalid Bearer token signature or expired")
        return {"sub": "usr_default", "customer_tier": "VIP", "token": token}

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        auth_header = request.headers.get(self.token_header) or request.headers.get(self.token_header.lower())
        if not auth_header:
            raise UserNotAuthenticated(f"Missing '{self.token_header}' header")

        token = auth_header
        if auth_header.lower().startswith("bearer "):
            token = auth_header[7:].strip()

        # 1. Validate & decode token
        try:
            claims = self.token_validator(token)
        except Exception as ex:
            raise UserNotAuthenticated(f"Token validation error: {str(ex)}")

        if not isinstance(claims, dict):
            claims = {"sub": str(claims)}

        # 2. Extract and map workflow injection claims
        try:
            injections = self.claims_mapper(claims)
        except Exception as ex:
            raise UserNotAuthorized(f"Claims mapping error: {str(ex)}")

        # 3. Attach claims to request state so router and engine merge into workflow data
        request.state.auth_claims = claims
        request.state.workflow_injections = injections

        return await call_next(request)


class ApiKeyAuthMiddleware(LiveMiddleware):
    """API Key verification middleware with customizable header and route exemptions."""

    def __init__(
        self,
        api_keys: Optional[Union[List[str], Set[str], str]] = None,
        header_name: str = "X-API-Key",
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "ApiKeyAuthMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        keys_input = api_keys if api_keys is not None else config.get("valid_keys")
        if isinstance(keys_input, str):
            self.valid_keys: Set[str] = {keys_input}
        elif keys_input is not None:
            self.valid_keys = set(keys_input)
        else:
            self.valid_keys = {"default-api-key", "secret-key", "katz-secret-key-12345"}
        self.header_name = config.get("header_name") or header_name

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        req_key = request.headers.get(self.header_name) or request.headers.get(self.header_name.lower())
        if not req_key or req_key not in self.valid_keys:
            raise InvalidApiKeyException(f"Invalid or missing '{self.header_name}' header")
        return await call_next(request)


class RoleAuthorizationMiddleware(LiveMiddleware):
    """Role-Based Access Control (RBAC) middleware verifying required roles against request headers or claims."""

    def __init__(
        self,
        required_roles: Optional[List[str]] = None,
        roles_header: str = "X-User-Roles",
        role_extractor: Optional[Callable[[Request], List[str]]] = None,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "RoleAuthorizationMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        self.required_roles = set(required_roles or [])
        self.roles_header = roles_header
        self.role_extractor = role_extractor

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        user_roles: List[str] = []
        if self.role_extractor:
            try:
                user_roles = self.role_extractor(request)
            except Exception as err:
                raise UserNotAuthorized(f"Role extraction failed: {str(err)}")
        elif hasattr(request.state, "auth_claims") and isinstance(request.state.auth_claims, dict):
            claims = request.state.auth_claims
            roles_val = claims.get("roles") or claims.get("role") or []
            user_roles = roles_val if isinstance(roles_val, list) else [str(roles_val)]
        else:
            raw_hdr = request.headers.get(self.roles_header) or request.headers.get(self.roles_header.lower())
            if raw_hdr:
                user_roles = [r.strip() for r in raw_hdr.split(",") if r.strip()]

        if self.required_roles and not self.required_roles.intersection(set(user_roles)):
            raise UserNotAuthorized(f"User lacks required role(s): {list(self.required_roles)}")

        return await call_next(request)


class CorrelationIdMiddleware(LiveMiddleware):
    """Correlation and Request ID tracking middleware injecting trace headers into requests and responses."""

    def __init__(
        self,
        header_name: str = "X-Correlation-ID",
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "CorrelationIdMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        self.header_name = header_name

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        corr_id = (
            request.headers.get(self.header_name)
            or request.headers.get("X-Request-ID")
            or f"req_{int(time.time()*1000)}_{uuid.uuid4().hex[:6]}"
        )
        request.state.correlation_id = corr_id
        response = await call_next(request)
        response.headers[self.header_name] = corr_id
        return response


class RateLimitMiddleware(LiveMiddleware):
    """Declarative in-memory sliding window rate limiting middleware with dynamic tier limits,
    pluggable key extraction strategies, standard RFC 6585 headers, and breach handler activities."""

    def __init__(
        self,
        limit: int = 60,
        window_seconds: int = 60,
        key_by: str = "client_ip",
        tier_limits: Optional[Dict[str, int]] = None,
        tier_claim: Optional[str] = "customer_tier",
        tier_header: Optional[str] = "X-User-Tier",
        key_extractor: Optional[Callable[[Request], str]] = None,
        include_headers: bool = True,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "RateLimitMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        self.default_limit = int(limit)
        self.window_seconds = int(window_seconds)
        self.key_by = key_by
        self.tier_limits = dict(tier_limits or {})
        self.tier_claim = tier_claim or "customer_tier"
        self.tier_header = tier_header or "X-User-Tier"
        self.key_extractor = key_extractor
        self.include_headers = include_headers
        self._requests: Dict[str, List[float]] = {}

    def _resolve_key(self, request: Request) -> str:
        if self.key_extractor:
            return self.key_extractor(request)

        strat = (self.key_by or "client_ip").lower()
        if strat in ("jwt_user", "user", "user_id", "sub"):
            if hasattr(request.state, "auth_claims") and isinstance(request.state.auth_claims, dict):
                claims = request.state.auth_claims
                user_id = claims.get("user_id") or claims.get("sub") or claims.get("username")
                if user_id:
                    return f"user:{user_id}"
            # Fallback if unauthenticated
            client_ip = request.headers.get("X-Forwarded-For") or (request.client.host if request.client else "unknown")
            return f"ip:{client_ip}"

        elif strat in ("api_key", "key"):
            key = request.headers.get("X-API-Key") or request.headers.get("x-api-key") or "anonymous_key"
            return f"apikey:{key}"

        elif strat in ("tenant_id", "tenant"):
            if hasattr(request.state, "auth_claims") and isinstance(request.state.auth_claims, dict):
                tenant = request.state.auth_claims.get("tenant_id")
                if tenant:
                    return f"tenant:{tenant}"
            hdr_tenant = request.headers.get("X-Tenant-ID") or "default_tenant"
            return f"tenant:{hdr_tenant}"

        elif strat in ("path", "route"):
            return f"path:{request.url.path}"

        else:  # default: client_ip
            client_ip = request.headers.get("X-Forwarded-For")
            if client_ip:
                client_ip = client_ip.split(",")[0].strip()
            elif request.client:
                client_ip = request.client.host
            else:
                client_ip = "127.0.0.1"
            return f"ip:{client_ip}"

    def _resolve_limit(self, request: Request) -> int:
        if self.tier_limits:
            tier = None
            if hasattr(request.state, "auth_claims") and isinstance(request.state.auth_claims, dict):
                claims = request.state.auth_claims
                if self.tier_claim and self.tier_claim in claims:
                    tier = str(claims[self.tier_claim])
                if not tier:
                    tier = (
                        claims.get("customer_tier")
                        or claims.get("tier")
                        or claims.get("subscription_plan")
                        or claims.get("role")
                    )
            if not tier:
                hdr_name = self.tier_header or "X-User-Tier"
                tier = request.headers.get(hdr_name) or "ANONYMOUS"

            if tier in self.tier_limits:
                return self.tier_limits[tier]
            elif tier.upper() in self.tier_limits:
                return self.tier_limits[tier.upper()]

        return self.default_limit

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        now = time.time()
        key = self._resolve_key(request)
        max_limit = self._resolve_limit(request)
        cutoff = now - self.window_seconds

        # Get or create timestamp records for this key
        timestamps = self._requests.setdefault(key, [])
        # Evict timestamps older than current sliding window
        timestamps[:] = [ts for ts in timestamps if ts > cutoff]

        if len(timestamps) >= max_limit:
            oldest = timestamps[0]
            retry_after = max(1, int(self.window_seconds - (now - oldest)))
            reset_ts = int(now + retry_after)

            headers = {
                "Retry-After": str(retry_after),
                "X-RateLimit-Limit": str(max_limit),
                "X-RateLimit-Remaining": "0",
                "X-RateLimit-Reset": str(reset_ts)
            }
            raise RateLimitExceeded(
                message=f"Rate limit exceeded. Quota: {max_limit} req/{self.window_seconds}s. Retry in {retry_after}s.",
                code=429,
                details={
                    "limit": max_limit,
                    "window_seconds": self.window_seconds,
                    "retry_after_seconds": retry_after,
                    "rate_limited_key": key
                },
                headers=headers
            )

        # Record this request timestamp
        timestamps.append(now)
        remaining = max(0, max_limit - len(timestamps))
        reset_ts = int(timestamps[0] + self.window_seconds)

        response = await call_next(request)

        if self.include_headers:
            response.headers["X-RateLimit-Limit"] = str(max_limit)
            response.headers["X-RateLimit-Remaining"] = str(remaining)
            response.headers["X-RateLimit-Reset"] = str(reset_ts)

        return response


class IpWhitelistMiddleware(LiveMiddleware):
    """Declarative and extensible IP Whitelisting Middleware supporting exact IP addresses,
    CIDR network subnets (e.g. 192.168.1.0/24, 10.0.0.0/8), blacklist overrides,
    custom database/cache validators, route exemptions, and breach handler activities."""

    def __init__(
        self,
        allowed_ips: Optional[Union[List[str], Set[str], str]] = None,
        blocked_ips: Optional[Union[List[str], Set[str], str]] = None,
        whitelist: Optional[Union[List[str], Set[str], str]] = None,
        allow_list: Optional[Union[List[str], Set[str], str]] = None,
        blacklist: Optional[Union[List[str], Set[str], str]] = None,
        block_list: Optional[Union[List[str], Set[str], str]] = None,
        custom_validator: Optional[Callable[[str, Request], Union[bool, Awaitable[bool]]]] = None,
        ip_extractor: Optional[Callable[[Request], str]] = None,
        allow_all: bool = False,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        name: str = "IpWhitelistMiddleware",
        **config: Any
    ):
        super().__init__(
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            name=name,
            **config
        )
        raw_allowed = allowed_ips if allowed_ips is not None else (whitelist if whitelist is not None else allow_list)
        raw_blocked = blocked_ips if blocked_ips is not None else (blacklist if blacklist is not None else block_list)

        self.allowed_networks: List[Union[ipaddress.IPv4Network, ipaddress.IPv6Network]] = self._parse_ip_networks(raw_allowed)
        self.blocked_networks: List[Union[ipaddress.IPv4Network, ipaddress.IPv6Network]] = self._parse_ip_networks(raw_blocked)
        self.custom_validator = custom_validator
        self.ip_extractor = ip_extractor
        self.allow_all = allow_all or ("*" in self._normalize_raw_list(raw_allowed))

    @staticmethod
    def _normalize_raw_list(val: Any) -> List[str]:
        if val is None:
            return []
        if isinstance(val, str):
            return [x.strip() for x in val.split(",") if x.strip()]
        if isinstance(val, (list, set, tuple)):
            return [str(x).strip() for x in val if str(x).strip()]
        return []

    def _parse_ip_networks(self, raw: Any) -> List[Union[ipaddress.IPv4Network, ipaddress.IPv6Network]]:
        networks = []
        for item in self._normalize_raw_list(raw):
            if item == "*":
                continue
            try:
                net = ipaddress.ip_network(item, strict=False)
                networks.append(net)
            except ValueError as ex:
                logger.warning("Invalid IP or CIDR format in IpWhitelistMiddleware: '%s' (%s)", item, ex)
        return networks

    def extract_ip(self, request: Request) -> str:
        if self.ip_extractor:
            return self.ip_extractor(request)

        # 1. X-Forwarded-For (first IP in chain)
        xfwd = request.headers.get("X-Forwarded-For") or request.headers.get("x-forwarded-for")
        if xfwd:
            return xfwd.split(",")[0].strip()

        # 2. X-Real-IP
        xreal = request.headers.get("X-Real-IP") or request.headers.get("x-real-ip")
        if xreal:
            return xreal.strip()

        # 3. Request Client Host
        if request.client and request.client.host and request.client.host != "testclient":
            return request.client.host

        return "127.0.0.1"

    async def is_ip_allowed(self, ip_str: str, request: Request) -> bool:
        """Extensible check method. Can be overridden by subclasses to query a database, Redis, or dynamic cache."""
        try:
            ip_obj = ipaddress.ip_address(ip_str)
        except ValueError:
            return False

        # 1. Check blacklist first
        for b_net in self.blocked_networks:
            if ip_obj in b_net:
                return False

        # 2. Check custom validator callback if provided
        if self.custom_validator:
            import inspect
            if inspect.iscoroutinefunction(self.custom_validator):
                res = await self.custom_validator(ip_str, request)
            else:
                res = self.custom_validator(ip_str, request)
            if res is not None:
                return bool(res)

        # 3. Check wildcard allow all
        if self.allow_all:
            return True

        # 4. Check allowed networks list
        if self.allowed_networks:
            for a_net in self.allowed_networks:
                if ip_obj in a_net:
                    return True
            return False

        # If no networks defined and no custom validator, default allow loopback/local
        return ip_obj.is_loopback

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        client_ip = self.extract_ip(request)
        allowed = await self.is_ip_allowed(client_ip, request)

        if not allowed:
            raise IpAccessDenied(
                message=f"Access denied: Client IP '{client_ip}' is not in the allowed IP whitelist",
                code=403,
                details={
                    "client_ip": client_ip,
                    "path": request.url.path,
                    "method": request.method
                }
            )

        return await call_next(request)


class ResponseAuditMiddleware(LiveMiddleware):
    """Post-request middleware that measures request latency and dispatches an audit activity."""

    def __init__(
        self,
        app: Optional[ASGIApp] = None,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        activity: Optional[str] = "RecordResponseAuditLog",
        is_async: bool = True,
        activity_executor: Optional[Any] = None,
        temporal_replicator: Optional[Any] = None,
        name: Optional[str] = "ResponseAuditMiddleware",
        **config: Any
    ):
        super().__init__(
            app=app,
            excludes=excludes,
            includes=includes,
            activity_executor=activity_executor,
            temporal_replicator=temporal_replicator,
            name=name,
            **config
        )
        self.activity_name = activity or config.get("activity", "RecordResponseAuditLog")
        self.is_async = is_async if is_async is not None else config.get("async", True)

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        start_time = time.time()
        response = await call_next(request)
        duration_ms = (time.time() - start_time) * 1000

        job_id = getattr(request.state, "correlation_id", None) or f"req_{int(time.time()*1000)}_{uuid.uuid4().hex[:6]}"
        auth_claims = getattr(request.state, "auth_claims", {}) or {}
        user_id = auth_claims.get("user_id") or auth_claims.get("sub") or "anonymous"
        customer_tier = auth_claims.get("customer_tier") or "STANDARD"

        audit_payload = {
            "job_id": job_id,
            "route": request.url.path,
            "method": request.method,
            "status_code": response.status_code,
            "duration_ms": round(duration_ms, 2),
            "user_id": user_id,
            "customer_tier": customer_tier,
            "client_ip": request.client.host if request.client else "127.0.0.1",
            "timestamp": int(time.time() * 1000)
        }

        if self.activity_name:
            if self.is_async:
                if self.temporal_replicator and hasattr(self.temporal_replicator, "dispatch_activity"):
                    asyncio.create_task(self.temporal_replicator.dispatch_activity(self.activity_name, audit_payload))
                elif self.activity_executor:
                    asyncio.create_task(self._run_handler_activity_safe(self.activity_name, audit_payload))
            else:
                if self.temporal_replicator and hasattr(self.temporal_replicator, "dispatch_activity"):
                    await self.temporal_replicator.dispatch_activity(self.activity_name, audit_payload)
                elif self.activity_executor:
                    await self._run_handler_activity(self.activity_name, audit_payload)

        return response


class RedisCacheMiddleware(LiveMiddleware):
    """Declarative Redis Response Cache Middleware with sub-millisecond short-circuiting,
    composite cache keys, in-memory fallback cache, and RFC bypass headers."""

    def __init__(
        self,
        app: Optional[ASGIApp] = None,
        ttl_seconds: int = 300,
        key_strategy: Optional[List[str]] = None,
        redis_url: str = "redis://localhost:6379/0",
        cache_headers: bool = True,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        temporal_replicator: Optional[Any] = None,
        name: str = "RedisCacheMiddleware",
        **config: Any
    ):
        super().__init__(
            app=app,
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            temporal_replicator=temporal_replicator,
            name=name,
            **config
        )
        self.ttl_seconds = int(ttl_seconds if ttl_seconds is not None else config.get("ttl_seconds", 300))
        self.key_strategy = list(key_strategy or config.get("key_strategy", ["route", "claim:user_id", "query"]))
        self.redis_url = str(redis_url or config.get("redis_url", "redis://localhost:6379/0"))
        self.cache_headers = cache_headers if cache_headers is not None else config.get("cache_headers", True)
        self._redis: Optional[Any] = None
        self._redis_initialized = False
        self._memory_cache: Dict[str, Dict[str, Any]] = {}

    async def _get_redis(self) -> Optional[Any]:
        if not self._redis_initialized:
            self._redis_initialized = True
            try:
                import redis.asyncio as aioredis
                self._redis = aioredis.from_url(self.redis_url, decode_responses=False)
                await asyncio.wait_for(self._redis.ping(), timeout=0.5)
            except Exception as ex:
                logger.warning("RedisCacheMiddleware: Redis unreachable at %s (%s). Using in-memory fallback cache.", self.redis_url, ex)
                self._redis = None
        return self._redis

    def build_cache_key(self, request: Request) -> str:
        method = request.method.upper()
        route = request.url.path
        parts = ["octopus", "cache", method, route]

        claims = getattr(request.state, "auth_claims", {}) or {}
        if not isinstance(claims, dict):
            claims = {}

        for strat in self.key_strategy:
            strat_lower = strat.lower()
            if strat_lower in ("method", "route"):
                continue
            elif strat_lower.startswith("claim:"):
                claim_name = strat[6:].strip()
                val = claims.get(claim_name)
                if val is None and claim_name == "user_id":
                    val = claims.get("sub") or request.headers.get("X-User-ID")
                if val is not None:
                    parts.append(f"{claim_name}={val}")
            elif strat_lower.startswith("header:"):
                hdr_name = strat[7:].strip()
                val = request.headers.get(hdr_name)
                if val:
                    parts.append(f"{hdr_name}={val}")
            elif strat_lower == "query":
                if request.query_params:
                    sorted_params = sorted(request.query_params.items())
                    query_str = "&".join(f"{k}={v}" for k, v in sorted_params if k not in ("no_cache", "force_refresh"))
                    if query_str:
                        parts.append(f"query={query_str}")
            elif strat_lower.startswith("query:"):
                q_name = strat[6:].strip()
                val = request.query_params.get(q_name)
                if val is not None:
                    parts.append(f"{q_name}={val}")

        return ":".join(parts)

    def is_bypass_requested(self, request: Request) -> bool:
        cache_ctrl = (request.headers.get("Cache-Control") or "").lower()
        if "no-cache" in cache_ctrl or "no-store" in cache_ctrl:
            return True
        pragma = (request.headers.get("Pragma") or "").lower()
        if "no-cache" in pragma:
            return True
        qp = request.query_params
        if qp.get("no_cache") in ("1", "true", "yes") or qp.get("force_refresh") in ("1", "true", "yes"):
            return True
        return False

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        # Only cache idempotent GET requests
        if request.method.upper() != "GET":
            return await call_next(request)

        cache_key = self.build_cache_key(request)

        # 1. Check bypass
        if self.is_bypass_requested(request):
            response = await call_next(request)
            if self.cache_headers:
                response.headers["X-Cache"] = "BYPASS"
                response.headers["X-Cache-Key"] = cache_key
            return response

        # 2. Check Cache HIT
        cached_data = None
        r = await self._get_redis()
        if r:
            try:
                cached_data = await asyncio.wait_for(r.get(cache_key), timeout=0.2)
            except Exception as ex:
                logger.debug("Redis GET error: %s", ex)
                cached_data = None

        now = time.time()
        if not cached_data and cache_key in self._memory_cache:
            entry = self._memory_cache[cache_key]
            if entry["expire_at"] > now:
                cached_data = entry["data"]
            else:
                self._memory_cache.pop(cache_key, None)

        if cached_data is not None:
            # CACHE HIT! Short-circuit immediately
            import hashlib
            short_id = hashlib.sha256(cache_key.encode()).hexdigest()[:12]
            headers = {
                "Content-Type": "application/json",
                "X-Job-ID": f"job_cached_{short_id}",
                "X-Execution-Status": "COMPLETED",
            }
            if self.cache_headers:
                headers["X-Cache"] = "HIT"
                headers["X-Cache-Key"] = cache_key
                headers["X-Cache-TTL"] = str(self.ttl_seconds)

            return Response(
                content=cached_data,
                status_code=200,
                headers=headers,
                media_type="application/json"
            )

        # 3. CACHE MISS
        response = await call_next(request)

        # Only cache 200 OK responses
        if response.status_code == 200:
            body = b""
            async for chunk in response.body_iterator:
                if isinstance(chunk, str):
                    body += chunk.encode("utf-8")
                else:
                    body += chunk

            # Write to Redis
            if r:
                try:
                    await asyncio.wait_for(r.setex(cache_key, self.ttl_seconds, body), timeout=0.5)
                except Exception as ex:
                    logger.debug("Redis SETEX error: %s", ex)

            # Write to Memory Cache
            self._memory_cache[cache_key] = {
                "data": body,
                "expire_at": now + self.ttl_seconds
            }

            headers = dict(response.headers)
            headers.pop("content-length", None)
            if self.cache_headers:
                headers["X-Cache"] = "MISS"
                headers["X-Cache-Key"] = cache_key
                headers["X-Cache-TTL"] = str(self.ttl_seconds)

            return Response(
                content=body,
                status_code=response.status_code,
                headers=headers,
                media_type=response.media_type or "application/json"
            )

        return response


class PayloadValidationMiddleware(LiveMiddleware):
    """
    Ingress middleware for intercepting and validating incoming request bodies.
    Validates JSON syntax, prevents memory flooding with max_body_size_bytes,
    and enforces required fields and headers, raising standard 400 Bad Request exceptions.
    """

    def __init__(
        self,
        app: Optional[ASGIApp] = None,
        max_body_size_bytes: int = 10485760,  # 10MB default
        require_json_body: bool = False,
        required_headers: Optional[List[str]] = None,
        required_fields: Optional[Dict[str, List[str]]] = None,
        excludes: Optional[List[str]] = None,
        includes: Optional[List[str]] = None,
        exceptions: Optional[List[MiddlewareExceptionConfig]] = None,
        activity_executor: Optional[Any] = None,
        temporal_replicator: Optional[Any] = None,
        state_store: Optional[Any] = None,
        durable_store: Optional[Any] = None,
        name: Optional[str] = None,
        **config: Any
    ):
        super().__init__(
            app=app,
            excludes=excludes,
            includes=includes,
            exceptions=exceptions,
            activity_executor=activity_executor,
            temporal_replicator=temporal_replicator,
            state_store=state_store,
            durable_store=durable_store,
            name=name or "PayloadValidationMiddleware",
            **config
        )
        self.max_body_size = int(max_body_size_bytes or config.get("max_body_size_bytes", 10485760))
        self.require_json_body = bool(require_json_body if require_json_body is not None else config.get("require_json_body", False))
        self.required_headers = list(required_headers or config.get("required_headers", []))
        if isinstance(required_fields, (dict, list)):
            self.required_fields = required_fields
        elif isinstance(config.get("required_fields"), (dict, list)):
            self.required_fields = config.get("required_fields")
        else:
            self.required_fields = {}

    async def process(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        method = request.method.upper()
        path = request.url.path

        # 1. Check required headers
        for req_hdr in self.required_headers:
            if req_hdr.lower() not in [h.lower() for h in request.headers.keys()]:
                raise BadRequestException(
                    message=f"Missing required request header: '{req_hdr}'",
                    details={"missing_header": req_hdr}
                )

        # 2. Check Content-Length header against max payload size
        content_length = request.headers.get("content-length")
        if content_length:
            try:
                cl_val = int(content_length)
                if cl_val > self.max_body_size:
                    raise PayloadTooLargeException(
                        message=f"Request payload size ({cl_val} bytes) exceeds maximum permitted limit ({self.max_body_size} bytes)",
                        details={"content_length": cl_val, "max_allowed": self.max_body_size}
                    )
            except ValueError:
                raise MalformedJsonException(message="Invalid Content-Length header value")

        # 3. For mutating requests, validate body and JSON formatting
        if method in ("POST", "PUT", "PATCH"):
            raw_body = await request.body()

            if len(raw_body) > self.max_body_size:
                raise PayloadTooLargeException(
                    message=f"Request payload size ({len(raw_body)} bytes) exceeds maximum permitted limit ({self.max_body_size} bytes)",
                    details={"body_size": len(raw_body), "max_allowed": self.max_body_size}
                )

            content_type = (request.headers.get("content-type") or "").lower()
            is_json_declared = "application/json" in content_type or "+json" in content_type

            if self.require_json_body and not is_json_declared and len(raw_body) > 0:
                raise BadRequestException(
                    message="Content-Type must be application/json for this endpoint",
                    details={"content_type": content_type}
                )

            parsed_json = None
            if len(raw_body) > 0 and (is_json_declared or self.require_json_body):
                try:
                    parsed_json = json.loads(raw_body.decode("utf-8"))
                except (json.JSONDecodeError, UnicodeDecodeError) as err:
                    raise MalformedJsonException(
                        message=f"Invalid or malformed JSON payload: {err}",
                        details={"parse_error": str(err)}
                    )

            # Validate required fields if configured
            if isinstance(parsed_json, dict):
                req_fields = []
                if isinstance(self.required_fields, list):
                    req_fields = self.required_fields
                elif isinstance(self.required_fields, dict):
                    req_fields = (
                        self.required_fields.get(path)
                        or self.required_fields.get(path.rstrip("/"))
                        or next((v for k, v in self.required_fields.items() if path.endswith(k.rstrip("/")) or fnmatch.fnmatch(path, k)), [])
                    )
                for f in req_fields:
                    if f not in parsed_json or parsed_json[f] is None or (isinstance(parsed_json[f], str) and parsed_json[f].strip() == ""):
                        raise MissingRequiredFieldException(
                            message=f"Missing required field '{f}' in request body",
                            details={"field": f, "route": path}
                        )

            # Rewind stream for downstream handlers
            async def receive():
                return {"type": "http.request", "body": raw_body, "more_body": False}
            request._receive = receive

        return await call_next(request)


# Standard Aliases
BadRequestMiddleware = PayloadValidationMiddleware
ValidationMiddleware = PayloadValidationMiddleware
InputValidationMiddleware = PayloadValidationMiddleware




