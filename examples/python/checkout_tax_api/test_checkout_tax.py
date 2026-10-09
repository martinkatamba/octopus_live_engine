import os
import sys
import importlib.util
import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))

spec = importlib.util.spec_from_file_location("checkout_tax_server", os.path.join(os.path.dirname(__file__), "server.py"))
checkout_tax_server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checkout_tax_server)
app = checkout_tax_server.app


@pytest.fixture
def client():
    return TestClient(app)


def test_public_login_endpoint(client):
    """Scenario 1: Public authentication route is exempt from Bearer token requirements."""
    res = client.post(
        "/api/v1.0.0/checkout_tax_calculator/auth/login",
        json={"username": "sarah_connor", "password": "secretPassword", "requested_tier": "VIP"}
    )
    assert res.status_code == 200
    data = res.json()
    assert "access_token" in data
    assert data["customer_tier"] == "VIP"
    assert data["token_type"] == "Bearer"


def test_protected_checkout_tax_unauthorized_rejections(client):
    """Scenario 2: Protected tax route rejects requests without valid Bearer tokens."""
    # A. Missing Authorization Header
    res_no_auth = client.post(
        "/checkout/tax",
        json={"cart_id": "cart_123", "items": [{"sku": "MACBOOK-AIR", "unit_price": 1299.00, "quantity": 1}]}
    )
    assert res_no_auth.status_code == 401
    assert "Missing 'Authorization' header" in res_no_auth.json()["detail"]

    # B. Invalid Token
    res_bad_token = client.post(
        "/checkout/tax",
        headers={"Authorization": "Bearer bad-expired-token"},
        json={"cart_id": "cart_123", "items": [{"sku": "MACBOOK-AIR", "unit_price": 1299.00, "quantity": 1}]}
    )
    assert res_bad_token.status_code == 401
    assert "Invalid signature" in res_bad_token.json()["detail"]


def test_vip_bearer_token_claim_extraction_and_preferential_tax(client):
    """Scenario 3: VIP Bearer token extracts customer_tier="VIP", routes to VIP branch with 4.5% tax."""
    # 1. Obtain VIP Token from public login endpoint
    login_res = client.post(
        "/api/v1.0.0/checkout_tax_calculator/auth/login",
        json={"username": "vip_shopper", "password": "password", "requested_tier": "VIP"}
    )
    vip_token = login_res.json()["access_token"]

    # 2. Call protected checkout tax calculation
    tax_res = client.post(
        "/checkout/tax",
        headers={"Authorization": f"Bearer {vip_token}"},
        json={
            "cart_id": "cart_vip_9988",
            "destination_country": "US",
            "destination_state": "CA",
            "items": [
                {"sku": "LAPTOP-PRO-16", "description": "MacBook Pro 16", "quantity": 1, "unit_price": 2000.00}
            ]
        }
    )
    assert tax_res.status_code == 200
    res_data = tax_res.json()
    assert res_data["customer_tier"] == "VIP"
    assert res_data["subtotal"] == 2000.00
    assert res_data["total_tax"] == 90.00  # 4.5% of 2000
    assert res_data["total_with_tax"] == 2090.00
    assert "VIP_PREFERENTIAL_TAX_RATE" in res_data["applied_rule"]
    assert "X-Correlation-ID" in tax_res.headers


def test_standard_bearer_token_claim_extraction_and_standard_tax(client):
    """Scenario 4: Standard Bearer token extracts customer_tier="STANDARD", routes to Standard branch with 9.25% tax."""
    # 1. Obtain Standard Token
    login_res = client.post(
        "/api/v1.0.0/checkout_tax_calculator/auth/login",
        json={"username": "std_shopper", "password": "password", "requested_tier": "STANDARD"}
    )
    std_token = login_res.json()["access_token"]

    # 2. Call protected checkout tax calculation
    tax_res = client.post(
        "/checkout/tax",
        headers={"Authorization": f"Bearer {std_token}"},
        json={
            "cart_id": "cart_std_1122",
            "destination_country": "US",
            "destination_state": "CA",
            "items": [
                {"sku": "KEYBOARD-RGB", "description": "Mechanical Keyboard", "quantity": 1, "unit_price": 100.00}
            ]
        }
    )
    assert tax_res.status_code == 200
    res_data = tax_res.json()
    assert res_data["customer_tier"] == "STANDARD"
    assert res_data["subtotal"] == 100.00
    assert res_data["total_tax"] == 9.25  # 9.25% of 100
    assert res_data["total_with_tax"] == 109.25
    assert "STANDARD_JURISDICTION_TAX" in res_data["applied_rule"]
