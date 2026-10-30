"""The core backend (`engine="core"`): what it must keep the same as the
Python engine. The rest of the suite runs on either engine (the core by
default, `OCTOPUS_LIVE_ENGINE=python` for Python); these tests pick one
explicitly."""

import asyncio
import logging

import pytest

pytest.importorskip("octopus_core")

from octopus_live.core_backend import blueprint_to_raw
from octopus_live.domain.models.models import StepCondition, WorkflowBlueprint, WorkflowStep
from octopus_live.domain.services.middleware import UserNotAuthorized
from octopus_live.engine import OctopusLiveEngine


class Declined(Exception):
    code = 402


def _fail_plain(payload):
    raise ValueError("bad input")


def _fail_coded(payload):
    raise Declined("card declined")


def _fail_middleware(payload):
    raise UserNotAuthorized("no role")


def _two_steps(engine, failing):
    e = OctopusLiveEngine(engine=engine)
    e.register_activity("ok", lambda p: {"a": 1})
    e.register_activity("bad", failing)
    e.register_blueprint(
        WorkflowBlueprint(workflow_name="w", steps=[WorkflowStep(id="s1", activity="ok"), WorkflowStep(id="s2", activity="bad")])
    )
    return e


@pytest.mark.parametrize("failing", [_fail_plain, _fail_coded, _fail_middleware], ids=["plain", "code", "middleware"])
def test_a_failed_activity_is_reported_as_the_python_engine_does(failing, caplog):
    caplog.set_level(logging.CRITICAL)

    def report(engine):
        e = _two_steps(engine, failing)
        st = asyncio.run(e.execute_sync("w", {}))
        stored = asyncio.run(e.get_execution_state(st.job_id))
        t = st.traces[-1]
        return (st.status, st.error, st.error_type, st.error_code, st.current_step_id, t.status, t.error_message,
                stored.error, stored.error_code)

    assert report("core") == report("python")


def test_async_activities_run_on_the_callers_loop():
    seen = {}

    async def fetch(payload):
        seen["loop"] = asyncio.get_running_loop()
        await asyncio.sleep(0)
        return {"fetched": payload["n"] + 1}

    e = OctopusLiveEngine(engine="core")
    e.register_activity("fetch", fetch)
    e.register_activity("plain", lambda p: 5)  # not a dict: wrapped as {"result": 5}
    e.register_blueprint(WorkflowBlueprint(workflow_name="a", steps=[WorkflowStep(id="f", activity="fetch"), WorkflowStep(id="p", activity="plain")]))

    async def main():
        st = await e.execute_sync("a", {"n": 1})
        return st, asyncio.get_running_loop()

    st, loop = asyncio.run(main())
    assert e.engine_backend == "core"
    assert st.data["fetched"] == 2 and st.data["result"] == 5
    assert seen["loop"] is loop


def test_live_events_stream_until_the_job_finishes():
    e = OctopusLiveEngine(engine="core")
    e.register_activity("slow", lambda p: __import__("time").sleep(0.2) or {"done": True})
    e.register_blueprint(WorkflowBlueprint(workflow_name="ev", steps=[WorkflowStep(id="s", activity="slow")]))

    async def main():
        events = []

        async def listen():
            async for ev in e.stream_events("job-ev-1"):
                events.append(ev["event"])

        listener = asyncio.create_task(listen())
        await asyncio.sleep(0.1)
        await e.execute_sync("ev", {"job_id": "job-ev-1"})
        await asyncio.wait_for(listener, 5)
        return events

    events = asyncio.run(main())
    assert events[-1] == "workflow_completed"
    assert "step_completed" in events


def test_blueprints_reach_the_core_in_its_own_shape():
    bp = WorkflowBlueprint(
        workflow_name="m",
        allowed_initiator_roles=["admin"],
        steps=[
            WorkflowStep(
                id="r",
                condition=StepCondition(
                    match="any",
                    rules=[{"field": "a", "operator": "gt", "value": 1}],
                    then_steps=[WorkflowStep(id="x", activity="X", timeout_seconds=5)],
                ),
            ),
            WorkflowStep(id="y", activity="Y", is_async=True),
        ],
    )
    raw = blueprint_to_raw(bp)
    step = raw["steps"][0]
    assert step["conditions"] == {"match": "any", "conditions": [{"field": "a", "operator": "gt", "value": 1}]}
    assert "rules" not in step["condition"]
    assert step["condition"]["then_steps"][0]["timeout_seconds"] == 5
    assert "timeout_seconds" not in raw["steps"][1]  # the dataclass default, never enforced by Python
    assert raw["steps"][1]["async"] is True
    assert "allowed_initiator_roles" not in raw  # the router enforces it


def test_configurations_the_core_cannot_serve_stay_on_python(caplog):
    from octopus_live.adapters.outbound.in_memory_store import InMemoryStateStore

    e = OctopusLiveEngine(engine="core", state_store=InMemoryStateStore())
    assert e.engine_backend == "python"
    assert "a Python state store object was given" in caplog.text
    for store, dsn in (("postgres", "postgresql://u:p@127.0.0.1:1/db?sslmode=require"),
                       ("mysql", "mysql://u:p@127.0.0.1:1/db?ssl-mode=REQUIRED")):
        tls = OctopusLiveEngine(engine="core", state_store=store, persistence={"dsn": dsn})
        assert tls.engine_backend == "python", store
    assert "without TLS" in caplog.text


def _app_with(middleware, activities, steps="      - id: s\n        activity: Hello\n", method="GET", trusted=None):
    import tempfile, os
    from starlette.testclient import TestClient

    path = os.path.join(tempfile.mkdtemp(), "mw.yaml")
    with open(path, "w") as f:
        f.write(
            "workflow_name: mw\nversion: \"1.0.0\"\napi:\n"
            f"  - route: /api/v1.0.0/mw/run\n    method: {method}\n    steps:\n{steps}"
        )
    e = OctopusLiveEngine.from_yaml(path, activity_instance=activities, engine="core", trusted_proxies=trusted)
    e.add_middleware(middleware)
    return e, TestClient(e.create_api_app())


class _Acts:
    calls = []

    @staticmethod
    def Hello(payload):
        return {"hello": "world"}

    @staticmethod
    def Audit(payload):
        _Acts.calls.append(payload)
        return {}


def test_core_middleware_roles_body_limits_cache_and_audit():
    from octopus_live.domain.services.middleware import (
        PayloadValidationMiddleware, RedisCacheMiddleware, ResponseAuditMiddleware, RoleAuthorizationMiddleware)

    _, client = _app_with(RoleAuthorizationMiddleware(required_roles=["admin"]), _Acts())
    assert client.get("/api/v1.0.0/mw/run", headers={"X-User-Roles": "admin"}).status_code == 200
    denied = client.get("/api/v1.0.0/mw/run", headers={"X-User-Roles": "guest"})
    assert denied.status_code == 403 and denied.json()["error"] == "UserNotAuthorized"

    _, client = _app_with(PayloadValidationMiddleware(max_body_size_bytes=16, require_json_body=True,
                                                      required_fields=["name"]), _Acts(), method="POST")
    assert client.post("/api/v1.0.0/mw/run", json={"name": "x"}).status_code == 200
    assert client.post("/api/v1.0.0/mw/run", json={"name": "far too long"}).json()["error"] == "PayloadTooLargeException"
    assert client.post("/api/v1.0.0/mw/run", json={}).json()["error"] == "MissingRequiredFieldException"
    plain = client.post("/api/v1.0.0/mw/run", content=b"x", headers={"Content-Type": "text/plain"})
    assert plain.status_code == 400 and plain.json()["error"] == "BadRequestException"

    _, client = _app_with(RedisCacheMiddleware(ttl_seconds=60, redis_url="127.0.0.1:1"), _Acts())
    first = client.get("/api/v1.0.0/mw/run?page=1")
    second = client.get("/api/v1.0.0/mw/run?page=1")
    assert first.headers["X-Cache"] == "MISS" and second.headers["X-Cache"] == "HIT"
    assert second.json() == first.json()

    _Acts.calls.clear()
    _, client = _app_with(ResponseAuditMiddleware(activity="Audit", is_async=False), _Acts())
    res = client.get("/api/v1.0.0/mw/run")
    assert _Acts.calls and _Acts.calls[0]["status_code"] == 200
    assert _Acts.calls[0]["route"] == "/api/v1.0.0/mw/run" and _Acts.calls[0]["job_id"] == res.headers["X-Job-ID"]


def test_forwarded_for_counts_only_from_trusted_proxies():
    from octopus_live.domain.services.middleware import IpWhitelistMiddleware

    mw = lambda: IpWhitelistMiddleware(allowed_ips=["10.0.0.0/8"])
    spoof = {"X-Forwarded-For": "10.1.2.3"}
    _, client = _app_with(mw(), _Acts())
    assert client.get("/api/v1.0.0/mw/run", headers=spoof).status_code == 403  # D11: no picking your own IP
    _, client = _app_with(mw(), _Acts(), trusted=["127.0.0.1"])
    assert client.get("/api/v1.0.0/mw/run", headers=spoof).status_code == 200


def _wait_for(e, job_id, done):
    import time

    for _ in range(100):
        st = asyncio.run(e.get_execution_state(job_id))
        if st is not None and done(st):
            return st
        time.sleep(0.05)
    return st


def test_hybrid_mode_runs_async_steps_in_process_when_temporal_is_down(caplog):
    caplog.set_level(logging.WARNING)
    e = OctopusLiveEngine(engine="core", mode="hybrid", temporal_host="127.0.0.1:1")
    assert e.engine_backend == "core" and e.pipeline_manager.temporal["host"] == "127.0.0.1:1"
    e.register_activity("charge", lambda p: {"charged": True})
    ran = []
    e.register_activity("notify", lambda p: ran.append(p) or {"notified": True})
    e.register_blueprint(WorkflowBlueprint(
        workflow_name="h", steps=[WorkflowStep(id="c", activity="charge"), WorkflowStep(id="n", activity="notify", is_async=True)]))

    asyncio.run(e.start_embedded_worker())  # Temporal unreachable: a warning, not an error
    assert "Temporal worker not started" in caplog.text

    st = asyncio.run(e.execute_sync("h", {}))
    final = _wait_for(e, st.job_id, lambda s: s.traces[-1].status == "ASYNC_COMPLETED")
    # As on the Python engine, an async step's result stays out of the job's data.
    assert final.status.value == "COMPLETED" and ran
    assert [(t.step_id, t.status) for t in final.traces] == [("c", "COMPLETED"), ("n", "ASYNC_COMPLETED")]
    asyncio.run(e.stop_embedded_worker())


def test_a_constructor_sqlite_path_keeps_jobs_in_the_cores_store(tmp_path):
    db = str(tmp_path / "jobs.db")

    def engine():
        e = OctopusLiveEngine(engine="core", sqlite_path=db)
        e.register_activity("ok", lambda p: {"a": 1})
        e.register_blueprint(WorkflowBlueprint(workflow_name="s", steps=[WorkflowStep(id="s1", activity="ok")]))
        return e

    first = engine()
    assert first.engine_backend == "core"
    st = asyncio.run(first.execute_sync("s", {}))
    again = asyncio.run(engine().get_execution_state(st.job_id))  # a second engine, the same file
    assert again is not None and again.status.value == "COMPLETED" and again.data["a"] == 1


@pytest.mark.parametrize("engine,label", [("core", "Rust core"), ("python", "Python engine")])
def test_health_and_the_visualizer_name_the_engine(engine, label):
    from starlette.testclient import TestClient

    e = OctopusLiveEngine(engine=engine, mode="hybrid", temporal_host="127.0.0.1:1")
    e.register_activity("ok", lambda p: {})
    e.register_blueprint(WorkflowBlueprint(workflow_name="v", steps=[WorkflowStep(id="s", activity="ok")]))
    client = TestClient(e.create_api_app())
    health = client.get("/health").json()
    assert health["backend"] == engine and health["mode"] == "hybrid"
    page = client.get("/visualizer").text
    assert page.count('id="engineBackendBadge"') == 1 and f">{label}</span>" in page
    # The step inspector names the language the activities are written in.
    assert "Python Activity Implementation" in page and "Rust Activity" not in page


def test_the_core_is_the_default_when_installed(monkeypatch, caplog):
    import octopus_live.engine as engine_mod

    monkeypatch.delenv("OCTOPUS_LIVE_ENGINE", raising=False)
    assert OctopusLiveEngine().engine_backend == "core"
    assert OctopusLiveEngine(engine="python").engine_backend == "python"
    monkeypatch.setenv("OCTOPUS_LIVE_ENGINE", "python")
    assert OctopusLiveEngine().engine_backend == "python"

    # Not installed: Python, quietly unless the core was asked for by name.
    monkeypatch.delenv("OCTOPUS_LIVE_ENGINE")
    monkeypatch.setattr(engine_mod, "core_available", lambda: False)
    caplog.set_level(logging.INFO, logger="octopus_live.engine")
    assert OctopusLiveEngine().engine_backend == "python"
    assert [r.levelname for r in caplog.records] == ["INFO"]
    caplog.clear()
    assert OctopusLiveEngine(engine="core").engine_backend == "python"
    assert [r.levelname for r in caplog.records] == ["WARNING"]
    assert "octopus_core is not installed" in caplog.text


@pytest.mark.parametrize("writer,reader", [("python", "core"), ("core", "python")])
def test_each_engine_reads_the_others_sqlite_jobs(tmp_path, writer, reader):
    db = str(tmp_path / "jobs.db")

    def engine(kind):
        e = OctopusLiveEngine(engine=kind, state_store="sqlite", sqlite_path=db)
        e.register_activity("ok", lambda p: {"a": 1})
        e.register_blueprint(WorkflowBlueprint(workflow_name="x", steps=[WorkflowStep(id="s1", activity="ok")]))
        return e

    async def main():
        st = await engine(writer).execute_sync("x", {})
        for _ in range(50):  # the Python engine writes from a background task
            got = await engine(reader).get_execution_state(st.job_id)
            if got is not None:
                return got
            await asyncio.sleep(0.05)

    got = asyncio.run(main())
    assert got is not None and got.status.value == "COMPLETED" and got.data["a"] == 1
    assert [t.output_payload for t in got.traces] == [{"a": 1}]
