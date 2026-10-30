"""The order fulfillment example on the core: its blueprint (with its SQLite
persistence) and its activities, served by the spike's FastAPI host. The
checks are the example's own, then each answer is compared with the
pure-Python engine's."""

import os

import pytest
from fastapi.testclient import TestClient

import octopus_core
from octopus_core.fastapi import create_app

from example_server import EXAMPLE_YAML, load_example_server

VIP = {
    "customer_id": "cust_vip_440",
    "customer_tier": "VIP",
    "order_id": "ord_1001",
    "items": [
        {"sku": "SKU_MACBOOK_PRO", "quantity": 1, "unit_price": 2499.00},
        {"sku": "SKU_MAGIC_MOUSE", "quantity": 1, "unit_price": 99.00},
    ],
    "shipping_address": "1 Infinite Loop, Cupertino, CA",
    "job_id": "job_ord_vip_1001",
}

STANDARD = {
    "customer_id": "cust_reg_99",
    "customer_tier": "STANDARD",
    "order_id": "ord_1002",
    "items": [{"sku": "SKU_CABLE_USB_C", "quantity": 2, "unit_price": 19.99}],
    "shipping_address": "123 Main St, Austin, TX",
    "job_id": "job_ord_std_1002",
}

# Time-based fields the activities generate.
VARYING = {"fulfillment_id"}


@pytest.fixture(scope="module")
def server(tmp_path_factory):
    return load_example_server(str(tmp_path_factory.mktemp("python_engine")))


@pytest.fixture(scope="module")
def core_client(server, tmp_path_factory):
    cwd = os.getcwd()
    os.chdir(tmp_path_factory.mktemp("core_engine"))  # the blueprint's SQLite file goes here
    try:
        engine = octopus_core.from_yaml(EXAMPLE_YAML, activity_instance=server.OrderFulfillmentActivities())
    finally:
        os.chdir(cwd)
    return TestClient(create_app(engine)), engine


@pytest.fixture(scope="module")
def python_client(server):
    return TestClient(server.app)


def post(client, payload):
    return client.post("/api/v1/orders/fulfill", json=payload, headers={"X-Job-ID": payload["job_id"]})


def test_high_value_order_routes_to_express_shipping(core_client):
    resp = post(core_client[0], VIP)
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "FULFILLED"
    assert data["order_id"] == "ord_1001"
    assert data["items_reserved_count"] == 2
    assert data["shipping_method"] == "PRIORITY_EXPRESS_AIR"
    assert data["estimated_delivery_days"] == 1


def test_standard_order_routes_to_standard_shipping(core_client):
    resp = post(core_client[0], STANDARD)
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "FULFILLED"
    assert data["order_id"] == "ord_1002"
    assert data["items_reserved_count"] == 1
    assert data["shipping_method"] == "STANDARD_GROUND"
    assert data["estimated_delivery_days"] == 3


def test_visualizer_graph_endpoint(core_client):
    resp = core_client[0].get("/api/v1.0.0/order_fulfillment_pipeline/diagram/x6")
    assert resp.status_code == 200
    data = resp.json()
    assert "nodes" in data
    assert "edges" in data
    assert len(data["nodes"]) >= 5


@pytest.mark.parametrize("payload", [VIP, STANDARD], ids=["vip", "standard"])
def test_same_answer_as_the_python_engine(core_client, python_client, payload):
    core = post(core_client[0], dict(payload, job_id=payload["job_id"] + "_cmp"))
    py = post(python_client, dict(payload, job_id=payload["job_id"] + "_cmp"))
    assert core.status_code == py.status_code == 200
    strip = lambda d: {k: v for k, v in d.items() if k not in VARYING}
    assert strip(core.json()) == strip(py.json())


def test_the_async_notification_step_runs(core_client):
    client, engine = core_client
    resp = post(client, STANDARD)
    state = engine.get_state(resp.headers["X-Job-ID"])
    assert state is not None
    steps = {t["step_id"]: t["status"] for t in state["traces"]}
    assert "step_send_notification" in steps
    assert steps["step_finalize_order"] == "COMPLETED"


def test_a_large_order_returns_the_same_data_as_the_python_engine(server, tmp_path):
    """The returned state converts each map the core shares once; the values
    must still match the Python engine's, trace by trace."""
    import asyncio
    import re

    from octopus_live import LiveEngine

    text = open(EXAMPLE_YAML).read().replace("max_iterations: 20", "max_iterations: 100")
    text = re.sub(r"\npersistence:\n(?:  .*\n|\n)*", "\n", re.sub(r"(?m)^mode: .*\n", "", text))
    path = tmp_path / "order.yaml"
    path.write_text(text)
    acts = server.OrderFulfillmentActivities()
    order = dict(VIP, items=[{"sku": f"SKU_{i}", "quantity": 1, "unit_price": 9.0} for i in range(30)])
    order.pop("job_id")

    core = octopus_core.from_yaml(str(path), activity_instance=acts).execute("order_fulfillment_pipeline", order)
    py = asyncio.run(LiveEngine.from_yaml(str(path), activity_instance=acts).execute_sync("order_fulfillment_pipeline", dict(order)))

    varying = {"job_id", "validated_at", "tracking_number", "dispatched_at", "fulfillment_id"}

    def strip(v):
        if isinstance(v, dict):
            return {k: strip(x) for k, x in v.items() if k not in varying}
        if isinstance(v, list):
            return [strip(x) for x in v]
        return v

    assert len(core["data"]["reserved_items"]) == 30
    assert strip(core["data"]) == strip(py.data)
    py_traces = {t.step_id: t for t in py.traces}
    for t in core["traces"]:
        # Activity steps only: condition traces differ between the engines
        # already (to-be-done.md), and async ones finish at their own pace.
        if t.get("is_async") or t["activity_name"] == "condition":  # is_async: only present when true
            continue
        assert strip(t.get("input_payload", {})) == strip(py_traces[t["step_id"]].input_payload), t["step_id"]
        out = strip(t.get("output_payload", {}))
        # The Python engine leaves `failures` out of a loop's trace; Go and
        # the core record it (to-be-done.md).
        if out.get("failures") == []:
            del out["failures"]
        assert out == strip(py_traces[t["step_id"]].output_payload), t["step_id"]
