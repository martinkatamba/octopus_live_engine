import os
import pytest
from fastapi.testclient import TestClient
from pydantic import BaseModel

from octopus_live.engine import OctopusLiveEngine
from octopus_live.domain.services.middleware import (
    ApiKeyAuthMiddleware,
    BearerAuthAndClaimsMiddleware,
    CorrelationIdMiddleware,
    LiveMiddleware,
    RoleAuthorizationMiddleware,
)


class TaxCalculationActivities:
    @staticmethod
    def NormalizeCartAndAddress(payload: dict) -> dict:
        tier = payload.get("customer_tier", "STANDARD")
        return {
            "normalized": True,
            "tier_detected": tier,
            "items_count": len(payload.get("items", []))
        }

    @staticmethod
    def CalculateVIPTax(payload: dict) -> dict:
        return {
            "tax_rate": 0.05,
            "tax_amount": 5.0,
            "applied_rule": "VIP_DISCOUNTED_TAX",
            "customer_tier": "VIP"
        }

    @staticmethod
    def CalculateStandardTax(payload: dict) -> dict:
        return {
            "tax_rate": 0.10,
            "tax_amount": 10.0,
            "applied_rule": "STANDARD_TAX",
            "customer_tier": "STANDARD"
        }


CHECKOUT_TAX_YAML = """
workflow_name: checkout_tax_calculator
version: "1.0.0"
mode: sync

middlewares:
  - name: BearerAuthAndClaimsMiddleware
    excludes:
      - /api/v1.0.0/checkout_tax_calculator/auth/login
      - /health
      - /visualizer*
      - /docs
      - /openapi.json

api:
  - route: /api/v1.0.0/checkout_tax_calculator/auth/login
    method: POST
    summary: "Public user authentication"
    steps:
      - id: step_public_login
        activity: NormalizeCartAndAddress

  - route: /checkout/tax
    method: POST
    summary: "Calculate sales tax and duty"
    steps:
      - id: step_normalize
        activity: NormalizeCartAndAddress

      - id: step_branch_by_tier
        condition:
          variable: customer_tier
          operator: "=="
          value: "VIP"
          then_steps:
            - id: step_calc_vip
              activity: CalculateVIPTax
          else_steps:
            - id: step_calc_std
              activity: CalculateStandardTax
"""


def test_declarative_middleware_and_token_claims_branching(tmp_path):
    # 1. Write YAML to temporary config
    yaml_file = tmp_path / "checkout_tax_calculator.yaml"
    yaml_file.write_text(CHECKOUT_TAX_YAML)

    # 2. Define custom token validator
    def mock_jwt_validator(token: str) -> dict:
        if token == "vip-token-123":
            return {"sub": "usr_99", "customer_tier": "VIP", "role": "admin"}
        elif token == "standard-token-456":
            return {"sub": "usr_01", "customer_tier": "STANDARD", "role": "buyer"}
        raise ValueError("Invalid JWT signature")

    # 3. Create engine from YAML
    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=TaxCalculationActivities())

    # 4. Register BearerAuthAndClaimsMiddleware instance with custom validator
    engine.register_middleware(
        "BearerAuthAndClaimsMiddleware",
        BearerAuthAndClaimsMiddleware(
            token_validator=mock_jwt_validator,
            claims_mapper=lambda claims: {
                "customer_tier": claims.get("customer_tier"),
                "customer_id": claims.get("sub")
            }
        )
    )

    # 5. Add Correlation ID middleware
    engine.add_middleware(CorrelationIdMiddleware())

    # 6. Create test client
    app = engine.create_api_app()
    client = TestClient(app)

    # TEST A: Excluded public route (/health) requires no auth
    res_health = client.get("/health")
    assert res_health.status_code == 200
    assert res_health.json()["status"] == "UP"

    # TEST B: Excluded login endpoint requires no auth
    res_login = client.post(
        "/api/v1.0.0/checkout_tax_calculator/auth/login",
        json={"username": "alice", "password": "password123"}
    )
    assert res_login.status_code == 200
    assert res_login.json()["normalized"] is True

    # TEST C: Protected route (/checkout/tax) without Bearer token returns 401
    res_unauth = client.post(
        "/checkout/tax",
        json={"items": [{"sku": "SKU-1", "price": 100.0}]}
    )
    assert res_unauth.status_code == 401
    assert "Missing 'Authorization' header" in res_unauth.json()["detail"]

    # TEST D: Protected route with invalid Bearer token returns 401
    res_invalid = client.post(
        "/checkout/tax",
        headers={"Authorization": "Bearer bad-token-xyz"},
        json={"items": [{"sku": "SKU-1", "price": 100.0}]}
    )
    assert res_invalid.status_code == 401
    assert "Invalid JWT signature" in res_invalid.json()["detail"]

    # TEST E: Protected route with VIP Bearer token extracts customer_tier="VIP" and branches to VIP tax!
    res_vip = client.post(
        "/checkout/tax?raw=true",
        headers={"Authorization": "Bearer vip-token-123"},
        json={"items": [{"sku": "SKU-1", "price": 100.0}]}
    )
    assert res_vip.status_code == 200
    vip_data = res_vip.json()
    assert vip_data["data"]["applied_rule"] == "VIP_DISCOUNTED_TAX"
    assert vip_data["data"]["tax_amount"] == 5.0
    assert vip_data["data"]["customer_tier"] == "VIP"
    assert "X-Correlation-ID" in res_vip.headers

    # TEST F: Protected route with Standard Bearer token branches to Standard tax!
    res_std = client.post(
        "/checkout/tax?raw=true",
        headers={"Authorization": "Bearer standard-token-456"},
        json={"items": [{"sku": "SKU-1", "price": 100.0}]}
    )
    assert res_std.status_code == 200
    std_data = res_std.json()
    assert std_data["data"]["applied_rule"] == "STANDARD_TAX"
    assert std_data["data"]["tax_amount"] == 10.0
    assert std_data["data"]["customer_tier"] == "STANDARD"


def test_custom_extensible_user_middleware(tmp_path):
    class CustomHeaderAuditMiddleware(LiveMiddleware):
        def __init__(self, audit_tag: str = "AUDIT_V1", excludes=None):
            super().__init__(excludes=excludes)
            self.audit_tag = audit_tag

        async def process(self, request, call_next):
            response = await call_next(request)
            response.headers["X-Audit-Tag"] = self.audit_tag
            return response

    yaml_file = tmp_path / "simple_wf.yaml"
    yaml_file.write_text("""
workflow_name: simple_wf
version: "1.0.0"
mode: sync
api:
  - route: /api/v1.0.0/simple_wf/ping
    method: GET
    steps:
      - id: step_ping
        activity: NormalizeCartAndAddress
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=TaxCalculationActivities())
    engine.use(CustomHeaderAuditMiddleware(audit_tag="PROD_AUDIT_2026", excludes=["/health"]))

    app = engine.create_api_app()
    client = TestClient(app)

    # Health is excluded from audit header
    res_health = client.get("/health")
    assert res_health.status_code == 200
    assert "X-Audit-Tag" not in res_health.headers

    # Ping endpoint receives audit header
    res_ping = client.get("/api/v1.0.0/simple_wf/ping")
    assert res_ping.status_code == 200
    assert res_ping.headers.get("X-Audit-Tag") == "PROD_AUDIT_2026"


def test_declarative_middleware_exception_handlers_and_activities(tmp_path):
    """Test YAML-declared exception codes, messages, and handler activities for built-in and custom exceptions."""
    audit_logs = []

    class SecurityActivities:
        @staticmethod
        def DummyStep(payload: dict) -> dict:
            return {"ok": True}

        @staticmethod
        def HandleSecurityAlert(payload: dict) -> dict:
            audit_logs.append(payload)
            return {
                "alert_dispatched": True,
                "ticket_id": "SEC-90210"
            }

    yaml_file = tmp_path / "secure_wf.yaml"
    yaml_file.write_text("""
workflow_name: secure_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: BearerAuthAndClaimsMiddleware
    excludes:
      - /health
    exceptions:
      - exception: UserNotAuthenticated
        code: 401
        message: "Custom 401: Authentication credentials missing or expired."
        activity: HandleSecurityAlert

      - exception: UserNotAuthorized
        code: 403
        message: "Custom 403: User lacks clearance."
        activity: HandleSecurityAlert

api:
  - route: /api/v1.0.0/secure_wf/vault
    method: GET
    steps:
      - id: step_vault
        activity: DummyStep
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=SecurityActivities())
    app = engine.create_api_app()
    client = TestClient(app)

    # 1. Missing token triggers UserNotAuthenticated exception handler
    res_missing = client.get("/api/v1.0.0/secure_wf/vault")
    assert res_missing.status_code == 401
    data = res_missing.json()
    assert data["error"] == "UserNotAuthenticated"
    assert data["detail"] == "Custom 401: Authentication credentials missing or expired."
    assert data["details"]["alert_dispatched"] is True
    assert data["details"]["ticket_id"] == "SEC-90210"
    assert len(audit_logs) == 1
    assert audit_logs[0]["exception"] == "UserNotAuthenticated"
    assert audit_logs[0]["path"] == "/api/v1.0.0/secure_wf/vault"


def test_custom_user_middleware_exceptions_and_activity_handlers(tmp_path):
    """Test custom user-defined middleware throwing custom exceptions and invoking handler activities."""
    class AccountSuspendedException(Exception):
        pass

    class AccountStatusMiddleware(LiveMiddleware):
        def __init__(self, excludes=None, exceptions=None, activity_executor=None):
            super().__init__(excludes=excludes, exceptions=exceptions, activity_executor=activity_executor)

        async def process(self, request, call_next):
            tenant = request.headers.get("X-Tenant-ID")
            if tenant == "suspended_corp":
                raise AccountSuspendedException("Account has been suspended due to policy violation")
            return await call_next(request)

    security_events = []

    class TenantActivities:
        @staticmethod
        def PingActivity(payload: dict) -> dict:
            return {"status": "PING_OK"}

        @staticmethod
        def LogSuspensionEvent(payload: dict) -> dict:
            security_events.append(payload)
            return {
                "support_url": "https://help.acme.com/suspended",
                "reference_code": "REF-SUSPENDED-88"
            }

    yaml_file = tmp_path / "tenant_wf.yaml"
    yaml_file.write_text("""
workflow_name: tenant_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: AccountStatusMiddleware
    exceptions:
      - exception: AccountSuspendedException
        code: 403
        message: "Your organization account is suspended. Please contact customer support."
        activity: LogSuspensionEvent

api:
  - route: /api/v1.0.0/tenant_wf/ping
    method: GET
    steps:
      - id: step_ping
        activity: PingActivity
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=TenantActivities())
    engine.register_middleware("AccountStatusMiddleware", AccountStatusMiddleware)

    app = engine.create_api_app()
    client = TestClient(app)

    # 1. Normal active tenant succeeds
    res_active = client.get("/api/v1.0.0/tenant_wf/ping", headers={"X-Tenant-ID": "active_corp"})
    assert res_active.status_code == 200
    assert res_active.json()["status"] == "PING_OK"

    # 2. Suspended tenant triggers AccountSuspendedException handler + LogSuspensionEvent activity
    res_suspended = client.get("/api/v1.0.0/tenant_wf/ping", headers={"X-Tenant-ID": "suspended_corp"})
    assert res_suspended.status_code == 403
    susp_data = res_suspended.json()
    assert susp_data["error"] == "AccountSuspendedException"
    assert susp_data["detail"] == "Your organization account is suspended. Please contact customer support."
    assert susp_data["details"]["reference_code"] == "REF-SUSPENDED-88"
    assert susp_data["details"]["support_url"] == "https://help.acme.com/suspended"
    assert len(security_events) == 1
    assert security_events[0]["exception"] == "AccountSuspendedException"


def test_declarative_rate_limit_middleware_sliding_window_and_headers(tmp_path):
    """Test RateLimitMiddleware enforcing request quotas, sliding window, and RFC 6585 headers."""
    yaml_file = tmp_path / "rate_limited_wf.yaml"
    yaml_file.write_text("""
workflow_name: rate_limited_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: RateLimitMiddleware
    limit: 3
    window_seconds: 60
    key_by: client_ip
    excludes:
      - /health

api:
  - route: /api/v1.0.0/rate_limited_wf/data
    method: GET
    steps:
      - id: step_data
        activity: NormalizeCartAndAddress
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=TaxCalculationActivities())
    app = engine.create_api_app()
    client = TestClient(app)

    # 1. Health check is excluded from rate limiting
    for _ in range(5):
        h_res = client.get("/health")
        assert h_res.status_code == 200
        assert "X-RateLimit-Limit" not in h_res.headers

    # 2. Request 1: OK, remaining=2
    r1 = client.get("/api/v1.0.0/rate_limited_wf/data")
    assert r1.status_code == 200
    assert r1.headers.get("X-RateLimit-Limit") == "3"
    assert r1.headers.get("X-RateLimit-Remaining") == "2"
    assert "X-RateLimit-Reset" in r1.headers

    # 3. Request 2: OK, remaining=1
    r2 = client.get("/api/v1.0.0/rate_limited_wf/data")
    assert r2.status_code == 200
    assert r2.headers.get("X-RateLimit-Remaining") == "1"

    # 4. Request 3: OK, remaining=0
    r3 = client.get("/api/v1.0.0/rate_limited_wf/data")
    assert r3.status_code == 200
    assert r3.headers.get("X-RateLimit-Remaining") == "0"

    # 5. Request 4: Rate limit breach -> 429 RateLimitExceeded
    r4 = client.get("/api/v1.0.0/rate_limited_wf/data")
    assert r4.status_code == 429
    err_data = r4.json()
    assert err_data["error"] == "RateLimitExceeded"
    assert "Rate limit exceeded" in err_data["detail"]
    assert "Retry-After" in r4.headers
    assert r4.headers.get("X-RateLimit-Remaining") == "0"
    assert err_data["details"]["limit"] == 3


def test_declarative_rate_limit_dynamic_tiers_and_handler_activity(tmp_path):
    """Test RateLimitMiddleware with dynamic customer tiers and breach handler activities."""
    rate_limit_breaches = []

    class AuditActivities:
        @staticmethod
        def QueryItem(payload: dict) -> dict:
            return {"item_id": "ITEM-101", "status": "AVAILABLE"}

        @staticmethod
        def LogRateLimitBreach(payload: dict) -> dict:
            rate_limit_breaches.append(payload)
            return {
                "throttled": True,
                "incident_ref": "RATELIMIT-INCIDENT-999"
            }

    yaml_file = tmp_path / "tiered_rate_limit_wf.yaml"
    yaml_file.write_text("""
workflow_name: tiered_rate_limit_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: RateLimitMiddleware
    limit: 2
    window_seconds: 30
    tier_limits:
      VIP: 10
      STANDARD: 2
    exceptions:
      - exception: RateLimitExceeded
        code: 429
        message: "API request quota exceeded. Please upgrade your plan or slow down."
        activity: LogRateLimitBreach

api:
  - route: /api/v1.0.0/tiered_rate_limit_wf/items
    method: GET
    steps:
      - id: step_item
        activity: QueryItem
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=AuditActivities())
    app = engine.create_api_app()
    client = TestClient(app)

    # 1. STANDARD tier: limit = 2
    res1 = client.get("/api/v1.0.0/tiered_rate_limit_wf/items", headers={"X-User-Tier": "STANDARD"})
    assert res1.status_code == 200
    assert res1.headers.get("X-RateLimit-Limit") == "2"

    res2 = client.get("/api/v1.0.0/tiered_rate_limit_wf/items", headers={"X-User-Tier": "STANDARD"})
    assert res2.status_code == 200

    # 3rd standard request breaches standard quota -> triggers custom message & LogRateLimitBreach activity
    res3 = client.get("/api/v1.0.0/tiered_rate_limit_wf/items", headers={"X-User-Tier": "STANDARD"})
    assert res3.status_code == 429
    data3 = res3.json()
    assert data3["error"] == "RateLimitExceeded"
    assert data3["detail"] == "API request quota exceeded. Please upgrade your plan or slow down."
    assert data3["details"]["throttled"] is True
    assert data3["details"]["incident_ref"] == "RATELIMIT-INCIDENT-999"
    assert len(rate_limit_breaches) == 1
    assert rate_limit_breaches[0]["exception"] == "RateLimitExceeded"

    # 2. VIP tier from a different IP / key: limit = 10
    for _ in range(5):
        vip_res = client.get(
            "/api/v1.0.0/tiered_rate_limit_wf/items",
            headers={"X-User-Tier": "VIP", "X-Forwarded-For": "10.0.0.99"}
        )
        assert vip_res.status_code == 200
        assert vip_res.headers.get("X-RateLimit-Limit") == "10"


def test_exception_injection_and_async_handler_activity(tmp_path):
    """Test parameterless exception raising where message is injected from YAML and activity runs async."""
    import time
    async_audit_logs = []

    class MockActivities:
        @staticmethod
        def ProtectedLoginStep(payload: dict) -> dict:
            # Raise UserNotAuthenticated without manual hardcoded message
            from octopus_live.domain.services.middleware import UserNotAuthenticated
            raise UserNotAuthenticated()

        @staticmethod
        def NonBlockingAuditViolation(payload: dict) -> dict:
            # Sleep slightly to prove it is non-blocking when async: true
            time.sleep(0.05)
            async_audit_logs.append(payload)
            return {"audit_recorded": True}

    yaml_file = tmp_path / "async_exc_wf.yaml"
    yaml_file.write_text("""
workflow_name: async_exc_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: BearerAuthAndClaimsMiddleware
    exceptions:
      - exception: UserNotAuthenticated
        code: 401
        message: "Declarative injected message: Token required for access."
        activity: NonBlockingAuditViolation
        async: true

api:
  - route: /api/v1.0.0/async_exc_wf/login
    method: POST
    steps:
      - id: step_login
        activity: ProtectedLoginStep
""")

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=MockActivities())
    app = engine.create_api_app()
    client = TestClient(app)

    # Calling endpoint triggers ProtectedLoginStep -> raises UserNotAuthenticated() -> message injected from YAML
    start = time.time()
    res = client.post("/api/v1.0.0/async_exc_wf/login", json={"user": "alice"})
    elapsed = time.time() - start

    assert res.status_code == 401
    assert res.json()["detail"] == "Declarative injected message: Token required for access."
    # Fast non-blocking response
    assert elapsed < 0.2


def test_declarative_ip_whitelist_middleware_cidr_and_single_ip(tmp_path):
    """Test IpWhitelistMiddleware with CIDR subnets, single IPs, blacklist overrides, and route exemptions."""
    audit_events = []

    class WhitelistActivities:
        @staticmethod
        def SecureDataFetch(payload: dict) -> dict:
            return {"sensitive_data": "SECRET_CORP_DATA_2026", "status": "OK"}

        @staticmethod
        def LogIpBreach(payload: dict) -> dict:
            audit_events.append(payload)
            return {"ip_audit_logged": True}

    yaml_file = tmp_path / "ip_whitelist_wf.yaml"
    yaml_file.write_text("""
workflow_name: ip_whitelist_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: IpWhitelistMiddleware
    allowed_ips:
      - 127.0.0.1
      - 10.0.0.0/24
      - 192.168.1.50
    blocked_ips:
      - 10.0.0.99
    excludes:
      - /health
    exceptions:
      - exception: IpAccessDenied
        code: 403
        message: "Access forbidden: Client IP network is not authorized."
        activity: LogIpBreach

api:
  - route: /api/v1.0.0/ip_whitelist_wf/secure
    method: GET
    steps:
      - id: step_secure
        activity: SecureDataFetch
""")

    # The test client stands in for a proxy, so its X-Forwarded-For names the
    # client; the core believes that header only from trusted proxies (D11).
    engine = OctopusLiveEngine.from_yaml(
        str(yaml_file), activity_instance=WhitelistActivities(), trusted_proxies=["127.0.0.1"]
    )
    app = engine.create_api_app()
    client = TestClient(app)

    # 1. Route exemption: /health is accessible regardless of IP
    res_health = client.get("/health", headers={"X-Forwarded-For": "203.0.113.88"})
    assert res_health.status_code == 200

    # 2. Allowed exact IP 127.0.0.1 succeeds
    res_local = client.get("/api/v1.0.0/ip_whitelist_wf/secure", headers={"X-Forwarded-For": "127.0.0.1"})
    assert res_local.status_code == 200
    assert res_local.json()["sensitive_data"] == "SECRET_CORP_DATA_2026"

    # 3. Allowed CIDR IP 10.0.0.45 in 10.0.0.0/24 succeeds
    res_cidr = client.get("/api/v1.0.0/ip_whitelist_wf/secure", headers={"X-Forwarded-For": "10.0.0.45"})
    assert res_cidr.status_code == 200

    # 4. Blacklisted IP 10.0.0.99 is blocked even though it is within 10.0.0.0/24
    res_blocked = client.get("/api/v1.0.0/ip_whitelist_wf/secure", headers={"X-Forwarded-For": "10.0.0.99"})
    assert res_blocked.status_code == 403
    data_blocked = res_blocked.json()
    assert data_blocked["error"] == "IpAccessDenied"
    assert data_blocked["detail"] == "Access forbidden: Client IP network is not authorized."
    assert len(audit_events) == 1
    assert audit_events[0]["exception"] == "IpAccessDenied"

    # 5. Unauthorized foreign IP is blocked with 403
    res_foreign = client.get("/api/v1.0.0/ip_whitelist_wf/secure", headers={"X-Forwarded-For": "198.51.100.99"})
    assert res_foreign.status_code == 403


def test_custom_database_ip_whitelist_middleware(tmp_path):
    """Test extending IpWhitelistMiddleware by subclassing and querying a dynamic database/cache."""
    db_allowed_ips = {"172.16.0.10", "172.16.0.20", "172.16.0.30"}

    from octopus_live.domain.services.middleware import IpWhitelistMiddleware

    class DatabaseBackedIpFilterMiddleware(IpWhitelistMiddleware):
        async def is_ip_allowed(self, ip_str: str, request) -> bool:
            # Query mock database table or cache dynamically
            return ip_str in db_allowed_ips or ip_str == "127.0.0.1"

    yaml_file = tmp_path / "custom_db_ip_wf.yaml"
    yaml_file.write_text("""
workflow_name: custom_db_ip_wf
version: "1.0.0"
mode: sync

middlewares:
  - name: DatabaseBackedIpFilterMiddleware

api:
  - route: /api/v1.0.0/custom_db_ip_wf/data
    method: GET
    steps:
      - id: step_data
        activity: SecureDataFetch
""")

    class DummyActs:
        @staticmethod
        def SecureDataFetch(payload: dict) -> dict:
            return {"data": "DB_WHITELIST_PROTECTED"}

    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=DummyActs(), trusted_proxies=["127.0.0.1"])
    engine.register_middleware("DatabaseBackedIpFilterMiddleware", DatabaseBackedIpFilterMiddleware)
    app = engine.create_api_app()
    client = TestClient(app)

    # 1. IP found in database succeeds
    res_db_ok = client.get("/api/v1.0.0/custom_db_ip_wf/data", headers={"X-Forwarded-For": "172.16.0.20"})
    assert res_db_ok.status_code == 200
    assert res_db_ok.json()["data"] == "DB_WHITELIST_PROTECTED"

    # 2. IP not in database is rejected
    res_db_rej = client.get("/api/v1.0.0/custom_db_ip_wf/data", headers={"X-Forwarded-For": "172.16.0.99"})
    assert res_db_rej.status_code == 403
    assert res_db_rej.json()["error"] == "IpAccessDenied"






@pytest.mark.parametrize("keys_block, accepted", [
    ("    config:\n      valid_keys: [partner-key-1]\n      header_name: X-Partner-Key\n", "partner-key-1"),
    ("    valid_keys: [partner-key-1]\n    header_name: X-Partner-Key\n", "partner-key-1"),
    ("", None),
])
def test_api_key_middleware_accepts_only_configured_keys(tmp_path, keys_block, accepted):
    """Only the blueprint's keys pass; with none configured, every request is refused."""

    class KeyActivities:
        @staticmethod
        def Ping(payload: dict) -> dict:
            return {"pong": True}

    yaml_file = tmp_path / "api_key_wf.yaml"
    yaml_file.write_text(
        "workflow_name: api_key_wf\nversion: \"1.0.0\"\n\nmiddlewares:\n  - name: ApiKeyAuthMiddleware\n"
        + keys_block
        + "    excludes: [/health]\n\napi:\n  - route: /api/v1.0.0/api_key_wf/ping\n    method: GET\n"
        "    steps:\n      - id: step_ping\n        activity: Ping\n"
    )
    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=KeyActivities())
    client = TestClient(engine.create_api_app())
    url = "/api/v1.0.0/api_key_wf/ping"

    header = "X-Partner-Key" if accepted else "X-API-Key"
    for old_builtin in ("default-api-key", "secret-key", "katz-secret-key-12345"):
        assert client.get(url, headers={header: old_builtin}).status_code == 401
    assert client.get(url).status_code == 401
    if accepted:
        assert client.get(url, headers={"X-Partner-Key": accepted}).status_code == 200
        assert client.get(url, headers={"X-API-Key": accepted}).status_code == 401
    assert client.get("/health").status_code == 200


def _hs256(claims: dict, secret: str, alg: str = "HS256") -> str:
    import base64, hashlib, hmac, json as _json
    enc = lambda d: base64.urlsafe_b64encode(_json.dumps(d).encode()).rstrip(b"=").decode()
    signing_input = f"{enc({'alg': alg, 'typ': 'JWT'})}.{enc(claims)}"
    sig = hmac.new(secret.encode(), signing_input.encode(), hashlib.sha256).digest()
    return signing_input + "." + base64.urlsafe_b64encode(sig).rstrip(b"=").decode()


@pytest.mark.parametrize("secret_block, env_secret", [
    ("    config:\n      jwt_secret: blueprint-secret-1\n", None),
    ("    jwt_secret: blueprint-secret-1\n", None),
    ("", "blueprint-secret-1"),
    ("", None),
])
def test_bearer_middleware_verifies_hs256_or_refuses(tmp_path, monkeypatch, secret_block, env_secret):
    """Tokens are verified with the configured secret; with no validator and no secret every token is refused."""
    import time as _time
    if env_secret:
        monkeypatch.setenv("OCTOPUS_JWT_SECRET", env_secret)
    else:
        monkeypatch.delenv("OCTOPUS_JWT_SECRET", raising=False)

    class ClaimActivities:
        @staticmethod
        def WhoAmI(payload: dict) -> dict:
            return {"sub_seen": payload.get("sub")}

    yaml_file = tmp_path / "jwt_wf.yaml"
    yaml_file.write_text(
        "workflow_name: jwt_wf\nversion: \"1.0.0\"\n\nmiddlewares:\n  - name: BearerAuthAndClaimsMiddleware\n"
        + secret_block
        + "    excludes: [/health]\n\napi:\n  - route: /api/v1.0.0/jwt_wf/me\n    method: GET\n"
        "    steps:\n      - id: step_me\n        activity: WhoAmI\n"
    )
    engine = OctopusLiveEngine.from_yaml(str(yaml_file), activity_instance=ClaimActivities())
    client = TestClient(engine.create_api_app())
    url = "/api/v1.0.0/jwt_wf/me"
    bearer = lambda t: {"Authorization": f"Bearer {t}"}
    later = int(_time.time()) + 600

    good = _hs256({"sub": "alice", "exp": later}, "blueprint-secret-1")
    for refused in (
        "anything-at-all",
        _hs256({"sub": "mallory", "exp": later}, "wrong-secret"),
        _hs256({"sub": "alice", "exp": int(_time.time()) - 5}, "blueprint-secret-1"),
        _hs256({"sub": "alice", "exp": later}, "blueprint-secret-1", alg="none"),
        _hs256({"sub": "alice"}, "octopus-live-engine-secret-key-2026"),
    ):
        assert client.get(url, headers=bearer(refused)).status_code == 401
    configured = bool(secret_block or env_secret)
    res = client.get(url, headers=bearer(good))
    assert res.status_code == (200 if configured else 401)
    if configured:
        assert res.json()["sub_seen"] == "alice"
    assert client.get("/health").status_code == 200
