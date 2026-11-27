"""Who may use the tooling routes: the visualizer, job history, status,
timelines, live streams, diagrams and the API docs.

The rule is the core's (`octopus-core` `tooling.rs`), as Go, Rust and .NET
apply it. It is written out here because the SDK learns its API key only in
`create_api_app`, after the core engine exists; a test checks it against the
core's own `tooling_decision`. The client's address follows D11: forwarded
headers count only from `trusted_proxies` (on the core, the core decides).
"""

import hmac
import inspect
import ipaddress
from typing import Any, Awaitable, Callable, List, Optional, Tuple, Union

from fastapi import HTTPException, Request

LOCAL = "local"
API_KEY = "api_key"
OPEN = "open"
DISABLED = "disabled"

DENY_NO_API_KEY = "Tooling requires an API key, but none is configured."
DENY_NOT_LOCAL = (
    "The visualizer and execution history are only available from this machine. "
    "Configure an API key, a ToolingAuthorizer or EngineOptions.Tooling to allow remote access."
)
DENY_AUTHORIZER = "Not allowed to use the engine tooling."

# Decides whether a request may use the tooling routes; replaces the rule.
ToolingAuthorizer = Callable[[Request], Union[bool, Awaitable[bool]]]


def parse_tooling(value: Optional[str]) -> str:
    """The core's names; anything else (and None) is the default, local."""
    v = (value or "").strip().lower()
    if v in ("api_key", "apikey"):
        return API_KEY
    if v in (OPEN, DISABLED):
        return v
    return LOCAL


def decide_tooling_access(
    tooling: str, has_authorizer: bool, api_key_configured: bool, client_is_loopback: bool
) -> Tuple[str, str]:
    """`(decision, detail)`: allow, not_found, ask_authorizer, require_api_key
    or deny (with the detail to answer 403 with)."""
    if tooling == DISABLED:
        return "not_found", ""
    if has_authorizer:
        return "ask_authorizer", ""
    if tooling == OPEN:
        return "allow", ""
    if tooling == API_KEY or api_key_configured:
        return ("require_api_key", "") if api_key_configured else ("deny", DENY_NO_API_KEY)
    return ("allow", "") if client_is_loopback else ("deny", DENY_NOT_LOCAL)


def _ip(text: str) -> Optional[Union[ipaddress.IPv4Address, ipaddress.IPv6Address]]:
    """The address, with an IPv4-mapped IPv6 one (`::ffff:127.0.0.1`) as IPv4."""
    try:
        ip = ipaddress.ip_address(text.strip())
    except ValueError:
        return None
    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped:
        return ip.ipv4_mapped
    return ip


def _trusted(peer: Any, proxies: List[str]) -> bool:
    for proxy in proxies:
        try:
            if peer in ipaddress.ip_network(proxy.strip(), strict=False):
                return True
        except ValueError:
            continue
    return False


def client_ip(peer: str, forwarded_for: str, real_ip: str, trusted_proxies: List[str]) -> str:
    """The TCP peer, or the forwarded headers when the peer is a trusted proxy
    (the core's `net.rs` rule)."""
    peer = (peer or "").strip()
    if not peer:
        return "127.0.0.1"
    try:
        peer_ip = ipaddress.ip_address(peer)
    except ValueError:
        return peer
    if _trusted(peer_ip, trusted_proxies):
        first = (forwarded_for or "").split(",")[0].strip()
        if first:
            return first
        if (real_ip or "").strip():
            return real_ip.strip()
    return peer


def is_loopback(ip: str) -> bool:
    if ip == "localhost":
        return True
    parsed = _ip(ip)
    return parsed is not None and parsed.is_loopback


class ToolingGuard:
    """Applies the rule to a request: returns to serve it, raises HTTPException
    (404, 401 or 403) to refuse it."""

    def __init__(
        self,
        tooling: str = LOCAL,
        authorizer: Optional[ToolingAuthorizer] = None,
        api_key: Optional[str] = None,
        trusted_proxies: Optional[List[str]] = None,
        core: Any = None,
    ):
        self.tooling = parse_tooling(tooling)
        self.authorizer = authorizer
        self.api_key = api_key or None
        self.trusted_proxies = list(trusted_proxies or [])
        self.core = core

    def client_ip(self, request: Request) -> str:
        peer = request.client.host if request.client else ""
        if peer == "testclient":  # Starlette's in-process TestClient
            peer = "127.0.0.1"
        forwarded = request.headers.get("X-Forwarded-For", "")
        real = request.headers.get("X-Real-IP", "")
        if self.core is not None:
            return self.core.client_ip(peer, forwarded, real)
        return client_ip(peer, forwarded, real, self.trusted_proxies)

    async def __call__(self, request: Request) -> None:
        decision, detail = decide_tooling_access(
            self.tooling,
            self.authorizer is not None,
            self.api_key is not None,
            is_loopback(self.client_ip(request)),
        )
        if decision == "allow":
            return
        if decision == "not_found":
            raise HTTPException(status_code=404, detail="Not Found")
        if decision == "ask_authorizer":
            verdict = self.authorizer(request)  # type: ignore[misc]
            if inspect.isawaitable(verdict):
                verdict = await verdict
            if verdict:
                return
            raise HTTPException(status_code=403, detail=DENY_AUTHORIZER)
        if decision == "require_api_key":
            given = request.headers.get("X-API-Key") or request.query_params.get("api_key") or ""
            if hmac.compare_digest(given.encode("utf-8"), self.api_key.encode("utf-8")):  # type: ignore[union-attr]
                return
            raise HTTPException(status_code=401, detail="Unauthorized API Key")
        raise HTTPException(status_code=403, detail=detail)
