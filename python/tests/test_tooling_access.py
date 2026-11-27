"""Who may use the tooling routes (the visualizer, history, status, timelines,
streams, diagrams and the API docs), on both engines."""

import itertools

import pytest
from starlette.testclient import TestClient

from octopus_live.adapters.inbound.tooling_access import (
    DENY_NO_API_KEY,
    DENY_NOT_LOCAL,
    client_ip,
    decide_tooling_access,
    is_loopback,
    parse_tooling,
)
from octopus_live.domain.models.models import WorkflowBlueprint, WorkflowStep
from octopus_live.engine import OctopusLiveEngine

WF = "/api/v1.0.0/ping"
TOOLING = [
    "/visualizer",
    "/api/visualizer/domains",
    "/api/visualizer/resolve-job?job_id=nope",
    "/api/visualizer/jobs",
    "/api/jobs",
    f"{WF}/status/nope",
    f"{WF}/timeline/nope",
    f"{WF}/diagram/x6",
    f"{WF}/jobs",
    "/docs",
    "/openapi.json",
    "/redoc",
]


def from_address(app, host):
    """The app, as reached from `host` (Starlette's TestClient says "testclient")."""

    async def wrapped(scope, receive, send):
        if scope["type"] in ("http", "websocket"):
            scope = dict(scope, client=(host, 50000))
        await app(scope, receive, send)

    return wrapped


def make_engine(**options):
    engine = OctopusLiveEngine(**options)
    engine.register_activity("Ping", lambda payload: {"pong": True})
    engine.register_blueprint(WorkflowBlueprint(workflow_name="ping", steps=[WorkflowStep(id="ping", activity="Ping")]))
    return engine


def statuses(client, headers=None):
    return {path: client.get(path, headers=headers or {}).status_code for path in TOOLING}


# What each route answers once allowed: the status and timeline of an unknown
# job are 404, everything else 200.
ALLOWED = {path: 404 if "/status/" in path or "/timeline/" in path else 200 for path in TOOLING}


def refused(code):
    return {path: code for path in TOOLING}


# --- The rule ------------------------------------------------------------------


def test_the_rule_matches_the_cores():
    octopus_core = pytest.importorskip("octopus_core")
    for tooling, authorizer, key, loopback in itertools.product(
        ["local", "api_key", "open", "disabled"], [False, True], [False, True], [False, True]
    ):
        core = octopus_core.Engine("k" if key else "", {"tooling": tooling})
        expected = tuple(core.tooling_decision(authorizer, loopback))
        assert decide_tooling_access(tooling, authorizer, key, loopback) == expected, (tooling, authorizer, key, loopback)


def test_the_client_address_matches_the_cores():
    octopus_core = pytest.importorskip("octopus_core")
    proxies = ["10.0.0.1", "192.168.0.0/16", "2001:db8::/32", "not-an-ip"]
    core = octopus_core.Engine("", {"trusted_proxies": proxies})
    cases = [
        ("", "", ""),
        ("203.0.113.9", "198.51.100.1", ""),  # not a proxy: its headers don't count
        ("10.0.0.1", "198.51.100.1, 10.0.0.1", ""),
        ("10.0.0.1", "", "198.51.100.2"),
        ("10.0.0.1", "", ""),
        ("192.168.4.5", "127.0.0.1", ""),
        ("2001:db8::7", "198.51.100.3", ""),
        ("::ffff:10.0.0.1", "198.51.100.4", ""),
        ("unix-socket", "198.51.100.5", ""),
    ]
    for peer, forwarded, real in cases:
        assert client_ip(peer, forwarded, real, proxies) == core.client_ip(peer, forwarded, real), (peer, forwarded, real)


def test_loopback_and_names():
    for ip in ("127.0.0.1", "127.8.9.10", "::1", "::ffff:127.0.0.1", "localhost"):
        assert is_loopback(ip), ip
    for ip in ("10.0.0.1", "203.0.113.9", "::ffff:10.0.0.1", "testclient", ""):
        assert not is_loopback(ip), ip
    assert parse_tooling(None) == "local"
    assert parse_tooling("APIKEY") == "api_key"
    assert parse_tooling("Open") == "open"
    assert parse_tooling("nonsense") == "local"


# --- The routes, on whichever engine OCTOPUS_LIVE_ENGINE picks ------------------


def test_by_default_only_this_machine_may_use_the_tooling():
    app = make_engine().create_api_app()
    assert statuses(TestClient(app)) == ALLOWED

    remote = TestClient(from_address(app, "203.0.113.9"))
    assert statuses(remote) == refused(403)
    assert remote.get("/visualizer").json()["detail"] == DENY_NOT_LOCAL
    # The live stream too (an allowed one stays open, so it isn't in TOOLING).
    assert remote.get(f"{WF}/stream/nope").status_code == 403
    # The API itself and the health check stay open.
    assert remote.get("/health").status_code == 200
    assert remote.post(f"{WF}/execute", json={}).status_code == 200


def test_a_forwarded_header_counts_only_from_a_trusted_proxy():
    app = make_engine(trusted_proxies=["10.0.0.1"]).create_api_app()
    says_local = {"X-Forwarded-For": "127.0.0.1"}
    assert TestClient(from_address(app, "203.0.113.9")).get("/api/jobs", headers=says_local).status_code == 403
    assert TestClient(from_address(app, "10.0.0.1")).get("/api/jobs", headers=says_local).status_code == 200
    assert TestClient(from_address(app, "10.0.0.1")).get("/api/jobs", headers={"X-Forwarded-For": "203.0.113.9"}).status_code == 403


def test_with_an_api_key_the_key_is_required_from_anywhere():
    app = make_engine().create_api_app(api_key="real-key")
    for client in (TestClient(app), TestClient(from_address(app, "203.0.113.9"))):
        assert statuses(client) == refused(401)
        assert statuses(client, {"X-API-Key": "real-key"}) == ALLOWED
        assert client.get("/api/jobs?api_key=real-key").status_code == 200
        assert client.get("/api/jobs", headers={"X-API-Key": "wrong"}).status_code == 401


def test_api_key_mode_without_a_key_refuses_everyone():
    app = make_engine(tooling="api_key").create_api_app()
    response = TestClient(app).get("/visualizer")
    assert response.status_code == 403
    assert response.json()["detail"] == DENY_NO_API_KEY


def test_open_and_disabled():
    open_app = make_engine(tooling="open").create_api_app()
    assert statuses(TestClient(from_address(open_app, "203.0.113.9"))) == ALLOWED

    disabled = TestClient(make_engine(tooling="disabled").create_api_app())
    assert statuses(disabled) == refused(404)
    assert disabled.post(f"{WF}/execute", json={}).status_code == 200


def test_an_authorizer_decides_instead():
    seen = []

    def authorizer(request):
        seen.append(request.url.path)
        return request.headers.get("X-Team") == "ops"

    app = make_engine(tooling_authorizer=authorizer).create_api_app(api_key="real-key")
    remote = TestClient(from_address(app, "203.0.113.9"))
    assert remote.get("/api/jobs", headers={"X-Team": "ops"}).status_code == 200
    assert remote.get("/api/jobs", headers={"X-API-Key": "real-key"}).status_code == 403
    assert remote.get("/docs", headers={"X-Team": "ops"}).status_code == 200
    assert "/api/jobs" in seen and "/docs" in seen


def test_an_async_authorizer():
    async def authorizer(request):
        return request.headers.get("X-Team") == "ops"

    app = make_engine(tooling_authorizer=authorizer).create_api_app()
    local = TestClient(app)
    assert local.get("/api/jobs").status_code == 403  # it replaces the local rule
    assert local.get("/api/jobs", headers={"X-Team": "ops"}).status_code == 200
