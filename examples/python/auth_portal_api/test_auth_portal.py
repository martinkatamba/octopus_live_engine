import os
import sys
import importlib.util
import pytest
from fastapi.testclient import TestClient

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))

spec = importlib.util.spec_from_file_location("auth_portal_server", os.path.join(os.path.dirname(__file__), "server.py"))
auth_portal_server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth_portal_server)
app = auth_portal_server.app


@pytest.fixture
def client():
    return TestClient(app)


def test_public_login_success_and_token_generation(client):
    """Scenario 1: Public login route issues signed Bearer JWT token."""
    res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        json={"username": "sarah_vip", "password": "password123"}
    )
    assert res.status_code == 200
    data = res.json()
    assert "access_token" in data
    assert data["token_type"] == "Bearer"
    assert data["user_id"] == "usr_9988_vip"
    assert data["customer_tier"] == "VIP"
    assert data["expires_in_seconds"] == 3600


def test_public_login_invalid_credentials_rejected(client):
    """Scenario 2: Public login route rejects bad credentials with 401 Unauthorized and declarative injected message."""
    res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        json={"username": "sarah_vip", "password": "wrongPassword"}
    )
    assert res.status_code == 401
    assert "Invalid username or password" in res.json()["detail"]


def test_protected_routes_unauthorized_rejections(client):
    """Scenario 3: Protected routes reject requests without token or with tampered token using declarative exception handlers."""
    # A. Missing Authorization Header -> triggers mapped UserNotAuthenticated exception & background LogSecurityAuditViolation
    res_no_auth = client.get("/api/v1.0.0/customer_auth_portal/profile")
    assert res_no_auth.status_code == 401
    data_no_auth = res_no_auth.json()
    assert data_no_auth["error"] == "UserNotAuthenticated"
    assert "Authentication required. Please provide a valid Bearer token" in data_no_auth["detail"]

    # B. Tampered Token Signature
    res_tampered = client.get(
        "/api/v1.0.0/customer_auth_portal/profile",
        headers={"Authorization": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjMifQ.badSignature"}
    )
    assert res_tampered.status_code == 401
    data_tampered = res_tampered.json()
    assert data_tampered["error"] == "UserNotAuthenticated"



def test_authenticated_user_profile_lookup(client):
    """Scenario 4: Fetch user profile using issued Bearer token."""
    # 1. Login as Sarah VIP
    login_res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        json={"username": "sarah_vip", "password": "password123"}
    )
    token = login_res.json()["access_token"]

    # 2. Fetch Profile with Bearer token
    profile_res = client.get(
        "/api/v1.0.0/customer_auth_portal/profile",
        headers={"Authorization": f"Bearer {token}"}
    )
    assert profile_res.status_code == 200
    data = profile_res.json()
    assert data["user_id"] == "usr_9988_vip"
    assert data["username"] == "sarah_vip"
    assert data["email"] == "sarah@acme-corp.com"
    assert data["customer_tier"] == "VIP"
    assert data["account_status"] == "ACTIVE_VIP"
    assert "vip_member" in data["roles"]
    assert "X-Correlation-ID" in profile_res.headers


def test_vip_cart_checkout_preferential_discount(client):
    """Scenario 5: VIP user checks out and receives 20% VIP discount + Free Priority Express Air."""
    # 1. Login as VIP
    login_res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        json={"username": "sarah_vip", "password": "password123"}
    )
    vip_token = login_res.json()["access_token"]

    # 2. Checkout
    checkout_res = client.post(
        "/api/v1.0.0/customer_auth_portal/cart/checkout",
        headers={"Authorization": f"Bearer {vip_token}"},
        json={
            "cart_id": "cart_vip_001",
            "items": [
                {"sku": "SONY-WH1000XM5", "description": "Wireless Headphones", "quantity": 2, "unit_price": 400.00}
            ]
        }
    )
    assert checkout_res.status_code == 200
    data = checkout_res.json()
    assert data["customer_tier"] == "VIP"
    assert data["subtotal"] == 800.00
    assert data["discount_applied"] == 160.00  # 20% of 800
    assert data["final_total"] == 640.00
    assert data["pricing_tier"] == "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT"
    assert data["shipping_method"] == "FREE_PRIORITY_EXPRESS_AIR"


def test_standard_cart_checkout_standard_pricing(client):
    """Scenario 6: Standard user checks out and receives standard retail pricing."""
    # 1. Login as Standard user
    login_res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        json={"username": "john_doe", "password": "password123"}
    )
    std_token = login_res.json()["access_token"]

    # 2. Checkout
    checkout_res = client.post(
        "/api/v1.0.0/customer_auth_portal/cart/checkout",
        headers={"Authorization": f"Bearer {std_token}"},
        json={
            "cart_id": "cart_std_002",
            "items": [
                {"sku": "USB-C-CABLE", "description": "Braided Cable", "quantity": 2, "unit_price": 20.00}
            ]
        }
    )
    assert checkout_res.status_code == 200
    data = checkout_res.json()
    assert data["customer_tier"] == "STANDARD"
    assert data["subtotal"] == 40.00
    assert data["discount_applied"] == 0.00
    assert data["final_total"] == 40.00
    assert data["pricing_tier"] == "STANDARD_RETAIL_PRICING"
    assert data["shipping_method"] == "STANDARD_GROUND_SHIPPING"


def test_rate_limit_tier_resolution_dynamic_upgrades(client):
    """Scenario 7: Rate limit headers accurately reflect tier quotas (ANONYMOUS=20, STANDARD=100, VIP=1000)."""
    # 1. Anonymous request (no token) -> Limit 20
    anon_res = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        headers={"X-Forwarded-For": "192.168.1.90"},
        json={"username": "sarah_vip", "password": "password123"}
    )
    assert anon_res.status_code == 200
    assert anon_res.headers.get("X-RateLimit-Limit") == "20"
    vip_token = anon_res.json()["access_token"]

    # 2. Standard user token -> Limit 100
    std_login = client.post(
        "/api/v1.0.0/customer_auth_portal/auth/login",
        headers={"X-Forwarded-For": "192.168.1.91"},
        json={"username": "john_doe", "password": "password123"}
    )
    std_token = std_login.json()["access_token"]

    std_res = client.get(
        "/api/v1.0.0/customer_auth_portal/profile",
        headers={"Authorization": f"Bearer {std_token}", "X-Forwarded-For": "192.168.1.91"}
    )
    assert std_res.status_code == 200
    assert std_res.headers.get("X-RateLimit-Limit") == "100"

    # 3. VIP user token -> Limit 1000
    vip_res = client.get(
        "/api/v1.0.0/customer_auth_portal/profile",
        headers={"Authorization": f"Bearer {vip_token}", "X-Forwarded-For": "192.168.1.90"}
    )
    assert vip_res.status_code == 200
    assert vip_res.headers.get("X-RateLimit-Limit") == "1000"


def test_visualizer_diagram_routes_filter_and_auto_selection(client):
    """Scenario 8: Visualizer diagram returns routes list and filters by route query param."""
    # 1. Base diagram
    res = client.get("/api/v1.0.0/customer_auth_portal/diagram/x6")
    assert res.status_code == 200
    data = res.json()
    assert "routes" in data
    assert len(data["routes"]) == 3
    assert data["selected_route"] == "/api/v1.0.0/customer_auth_portal/auth/login"

    # 2. Filter by cart checkout route
    cart_res = client.get("/api/v1.0.0/customer_auth_portal/diagram/x6?route=/api/v1.0.0/customer_auth_portal/cart/checkout")
    assert cart_res.status_code == 200
    cart_data = cart_res.json()
    assert cart_data["selected_route"] == "/api/v1.0.0/customer_auth_portal/cart/checkout"
    node_ids = [n["id"] for n in cart_data["nodes"]]
    assert "node__step_validate_cart_items" in node_ids
    assert "node__step_branch_by_tier" in node_ids
    assert "node__step_finalize_checkout" in node_ids


