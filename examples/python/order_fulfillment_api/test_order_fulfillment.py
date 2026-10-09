import pytest
from fastapi.testclient import TestClient
from .server import app


@pytest.fixture
def client():
    return TestClient(app)


def test_high_value_order_routes_to_express_shipping(client):
    payload = {
        "customer_id": "cust_vip_440",
        "customer_tier": "VIP",
        "order_id": "ord_1001",
        "items": [
            {"sku": "SKU_MACBOOK_PRO", "quantity": 1, "unit_price": 2499.00},
            {"sku": "SKU_MAGIC_MOUSE", "quantity": 1, "unit_price": 99.00}
        ],
        "shipping_address": "1 Infinite Loop, Cupertino, CA",
        "job_id": "job_ord_vip_1001"
    }
    resp = client.post("/api/v1/orders/fulfill", json=payload, headers={"X-Job-ID": "job_ord_vip_1001"})
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "FULFILLED"
    assert data["order_id"] == "ord_1001"
    assert data["items_reserved_count"] == 2
    assert data["shipping_method"] == "PRIORITY_EXPRESS_AIR"
    assert data["estimated_delivery_days"] == 1


def test_standard_order_routes_to_standard_shipping(client):
    payload = {
        "customer_id": "cust_reg_99",
        "customer_tier": "STANDARD",
        "order_id": "ord_1002",
        "items": [
            {"sku": "SKU_CABLE_USB_C", "quantity": 2, "unit_price": 19.99}
        ],
        "shipping_address": "123 Main St, Austin, TX",
        "job_id": "job_ord_std_1002"
    }
    resp = client.post("/api/v1/orders/fulfill", json=payload, headers={"X-Job-ID": "job_ord_std_1002"})
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "FULFILLED"
    assert data["order_id"] == "ord_1002"
    assert data["items_reserved_count"] == 1
    assert data["shipping_method"] == "STANDARD_GROUND"
    assert data["estimated_delivery_days"] == 3


def test_visualizer_graph_endpoint(client):
    resp = client.get("/api/v1.0.0/order_fulfillment_pipeline/diagram/x6")
    assert resp.status_code == 200
    data = resp.json()
    assert "nodes" in data
    assert "edges" in data
    assert len(data["nodes"]) >= 5
