import base64
import hashlib
import hmac
import json
import os
import sys
import time
from typing import Any, Dict, List, Optional
from pydantic import BaseModel, Field

# Ensure octopus_live package is discoverable
pkg_root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", "python", "src"))
sys.path.insert(0, pkg_root)

from octopus_live.engine import OctopusLiveEngine
from octopus_live.domain.services.middleware import (
    BearerAuthAndClaimsMiddleware,
    CorrelationIdMiddleware,
    UserNotAuthenticated,
    InvalidCredentials,
    UserNotAuthorized,
)

JWT_SECRET_KEY = "octopus-live-engine-secret-key-2026"


# -----------------------------------------------------------------------------
# 1. Pydantic Models for OpenAPI Schema & Strict Type Safety
# -----------------------------------------------------------------------------
class LoginRequest(BaseModel):
    username: str = Field(..., description="User login username (e.g. sarah_vip or john_doe)")
    password: str = Field(..., description="User login password")


class LoginResponse(BaseModel):
    access_token: str
    token_type: str = "Bearer"
    user_id: str
    customer_tier: str
    expires_in_seconds: int = 3600


class UserProfileResponse(BaseModel):
    user_id: str
    username: str
    email: str
    customer_tier: str
    account_status: str
    member_since: str
    roles: List[str] = Field(default_factory=list)


class CartItem(BaseModel):
    sku: str
    description: str = "Product Item"
    quantity: int = 1
    unit_price: float


class CheckoutRequest(BaseModel):
    cart_id: str
    items: List[CartItem] = Field(default_factory=list)


class CheckoutResponse(BaseModel):
    order_id: str
    customer_id: str
    customer_tier: str
    subtotal: float
    discount_applied: float
    final_total: float
    pricing_tier: str
    shipping_method: str
    fast_path_latency_ms: float = 0.35


# -----------------------------------------------------------------------------
# 2. JWT Cryptographic Helpers (Pure Python Standard Library HMAC-SHA256)
# -----------------------------------------------------------------------------
def create_jwt_token(payload: dict, secret: str = JWT_SECRET_KEY, expires_in: int = 3600) -> str:
    header = {"alg": "HS256", "typ": "JWT"}
    claims = dict(payload)
    claims["exp"] = int(time.time()) + expires_in
    claims["iat"] = int(time.time())

    def _b64(data: dict) -> str:
        return base64.urlsafe_b64encode(json.dumps(data).encode()).decode().rstrip("=")

    h_b64 = _b64(header)
    p_b64 = _b64(claims)
    signing_input = f"{h_b64}.{p_b64}".encode()
    sig = hmac.new(secret.encode(), signing_input, hashlib.sha256).digest()
    s_b64 = base64.urlsafe_b64encode(sig).decode().rstrip("=")
    return f"{h_b64}.{p_b64}.{s_b64}"


def decode_and_verify_jwt(token: str, secret: str = JWT_SECRET_KEY) -> dict:
    parts = token.split(".")
    if len(parts) != 3:
        raise ValueError("Malformed JWT structure (must contain 3 dot-separated parts)")

    h_b64, p_b64, s_b64 = parts
    signing_input = f"{h_b64}.{p_b64}".encode()
    expected_sig = hmac.new(secret.encode(), signing_input, hashlib.sha256).digest()
    
    # Pad base64 if needed
    rem = len(s_b64) % 4
    padded_s_b64 = s_b64 + ("=" * (4 - rem) if rem else "")
    actual_sig = base64.urlsafe_b64decode(padded_s_b64.encode())

    if not hmac.compare_digest(expected_sig, actual_sig):
        raise ValueError("Invalid JWT cryptographic signature")

    # Decode payload
    p_rem = len(p_b64) % 4
    padded_p_b64 = p_b64 + ("=" * (4 - p_rem) if p_rem else "")
    payload_dict = json.loads(base64.urlsafe_b64decode(padded_p_b64.encode()).decode())

    # Check expiry
    if payload_dict.get("exp", 0) < int(time.time()):
        raise ValueError("JWT token has expired")

    return payload_dict


# -----------------------------------------------------------------------------
# 3. In-Memory Domain Activities
# -----------------------------------------------------------------------------
# Mock database of users
USER_DATABASE = {
    "sarah_vip": {
        "user_id": "usr_9988_vip",
        "username": "sarah_vip",
        "email": "sarah@acme-corp.com",
        "password": "password123",
        "customer_tier": "VIP",
        "roles": ["customer", "vip_member"],
        "account_status": "ACTIVE_VIP",
        "member_since": "2024-01-15"
    },
    "john_doe": {
        "user_id": "usr_1024_std",
        "username": "john_doe",
        "email": "john.doe@example.com",
        "password": "password123",
        "customer_tier": "STANDARD",
        "roles": ["customer"],
        "account_status": "ACTIVE_STANDARD",
        "member_since": "2025-06-20"
    }
}


class AuthPortalActivities:
    @staticmethod
    def VerifyUserCredentials(payload: dict) -> dict:
        username = payload.get("username", "")
        password = payload.get("password", "")

        # Simulate unexpected runtime crash for crash_user
        if username == "crash_user":
            raise ZeroDivisionError("division by zero during unexpected calculation crash")

        user = USER_DATABASE.get(username)
        if not user or user["password"] != password:
            raise InvalidCredentials()

        return {
            "authenticated": True,
            "user_id": user["user_id"],
            "verified_username": user["username"],
            "email": user["email"],
            "customer_tier": user["customer_tier"],
            "roles": user["roles"]
        }

    @staticmethod
    def IssueBearerToken(payload: dict) -> dict:
        user_id = payload.get("user_id")
        tier = payload.get("customer_tier", "STANDARD")
        username = payload.get("verified_username")
        email = payload.get("email")
        roles = payload.get("roles", ["customer"])

        jwt_claims = {
            "sub": user_id,
            "user_id": user_id,
            "username": username,
            "email": email,
            "customer_tier": tier,
            "roles": roles
        }
        token = create_jwt_token(jwt_claims, secret=JWT_SECRET_KEY, expires_in=3600)

        return {
            "access_token": token,
            "token_type": "Bearer",
            "user_id": user_id,
            "customer_tier": tier,
            "expires_in_seconds": 3600
        }

    @staticmethod
    def FetchUserProfile(payload: dict) -> dict:
        user_id = payload.get("user_id")
        user = next((u for u in USER_DATABASE.values() if u["user_id"] == user_id), None)
        if not user:
            # Fallback for dynamic tokens
            return {
                "user_id": user_id or "usr_unknown",
                "username": payload.get("username", "guest"),
                "email": payload.get("email", "guest@example.com"),
                "customer_tier": payload.get("customer_tier", "STANDARD"),
                "account_status": "ACTIVE",
                "member_since": "2026-01-01",
                "roles": payload.get("roles", ["customer"])
            }

        return {
            "user_id": user["user_id"],
            "username": user["username"],
            "email": user["email"],
            "customer_tier": user["customer_tier"],
            "account_status": user["account_status"],
            "member_since": user["member_since"],
            "roles": user["roles"]
        }

    @staticmethod
    def ValidateCartItems(payload: dict) -> dict:
        items = payload.get("items", [])
        subtotal = sum(item.get("unit_price", 0.0) * item.get("quantity", 1) for item in items)
        if subtotal == 0.0:
            subtotal = 300.00
            items = [{"sku": "HEADPHONES-PRO", "description": "Noise Cancelling", "quantity": 1, "unit_price": 300.00}]

        return {
            "subtotal": round(subtotal, 2),
            "validated_items": items,
            "cart_id": payload.get("cart_id", "cart_default")
        }

    @staticmethod
    def ApplyVIPDiscount(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        discount = round(subtotal * 0.20, 2)  # 20% VIP privilege discount
        final_total = round(subtotal - discount, 2)

        return {
            "discount_applied": discount,
            "final_total": final_total,
            "pricing_tier": "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
            "shipping_method": "FREE_PRIORITY_EXPRESS_AIR"
        }

    @staticmethod
    def ApplyStandardPricing(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        return {
            "discount_applied": 0.0,
            "final_total": subtotal,
            "pricing_tier": "STANDARD_RETAIL_PRICING",
            "shipping_method": "STANDARD_GROUND_SHIPPING"
        }

    @staticmethod
    def FinalizeCheckoutOrder(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        discount = payload.get("discount_applied", 0.0)
        final_total = payload.get("final_total", subtotal)
        customer_id = payload.get("customer_id") or payload.get("user_id", "usr_anon")
        tier = payload.get("customer_tier", "STANDARD")

        return {
            "order_id": f"ord_{int(time.time() * 1000)}",
            "customer_id": customer_id,
            "customer_tier": tier,
            "subtotal": subtotal,
            "discount_applied": discount,
            "final_total": final_total,
            "pricing_tier": payload.get("pricing_tier", "STANDARD_RETAIL_PRICING"),
            "shipping_method": payload.get("shipping_method", "STANDARD_GROUND_SHIPPING"),
            "fast_path_latency_ms": 0.35
        }

    @staticmethod
    def LogSecurityAuditViolation(payload: dict) -> dict:
        exception = payload.get("exception", "SecurityViolation")
        path = payload.get("path", "")
        method = payload.get("method", "")
        client_host = payload.get("client_host", "unknown")
        print(f"🔒 [SECURITY AUDIT ACTIVITY] Security violation logged: {method} {path} from {client_host} ({exception})", file=sys.stderr)
        return {
            "audit_logged": True,
            "security_incident_id": f"inc_{int(time.time()*1000)}",
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        }

    @staticmethod
    def RecordResponseAuditLog(payload: dict) -> dict:
        job_id = payload.get("job_id", "job_unknown")
        route = payload.get("route", "")
        status_code = payload.get("status_code", 200)
        duration_ms = payload.get("duration_ms", 0.0)
        user_id = payload.get("user_id", "anonymous")
        tier = payload.get("customer_tier", "STANDARD")
        print(f"📊 [POST-REQUEST AUDIT] Job={job_id} Route={route} Status={status_code} User={user_id} Tier={tier} Latency={duration_ms:.2f}ms")
        return {
            "audited": True,
            "job_id": job_id,
            "timestamp": int(time.time() * 1000)
        }

    @staticmethod
    def HandleGlobalUnhandledException(payload: dict) -> dict:
        err_type = payload.get("error_type") or payload.get("exception", "UnknownError")
        err_msg = payload.get("error_message") or payload.get("message", "Internal Error")
        job_id = payload.get("job_id", "unknown")
        failing_step = payload.get("failing_step_id", "unknown")
        print(f"🚨 [GLOBAL EXCEPTION HANDLER] Incident logged for Job={job_id} Step={failing_step}: {err_type} - {err_msg}", file=sys.stderr)
        return {
            "incident_logged": True,
            "incident_id": f"inc_global_{int(time.time()*1000)}",
            "error_type": err_type,
            "error_message": err_msg,
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        }


# -----------------------------------------------------------------------------
# 4. Engine Initialization & Middleware Registration
# -----------------------------------------------------------------------------
yaml_path = os.path.join(os.path.dirname(__file__), "auth_portal_workflow.yaml")
engine = OctopusLiveEngine.from_yaml(
    yaml_path,
    activity_instance=AuthPortalActivities()
)

# Register models for OpenAPI schema generation & validation
engine.register_model("LoginRequest", LoginRequest)
engine.register_model("LoginResponse", LoginResponse)
engine.register_model("UserProfileResponse", UserProfileResponse)
engine.register_model("CheckoutRequest", CheckoutRequest)
engine.register_model("CheckoutResponse", CheckoutResponse)

# Register BearerAuthAndClaimsMiddleware with JWT verification and claim injection
engine.register_middleware(
    "BearerAuthAndClaimsMiddleware",
    BearerAuthAndClaimsMiddleware(
        token_validator=decode_and_verify_jwt,
        claims_mapper=lambda claims: {
            "user_id": claims.get("user_id") or claims.get("sub"),
            "customer_id": claims.get("user_id") or claims.get("sub"),
            "customer_tier": claims.get("customer_tier", "STANDARD"),
            "roles": claims.get("roles", ["customer"]),
            "email": claims.get("email")
        }
    )
)

# Create FastAPI application
app = engine.create_api_app(title="Customer Auth Portal API")

if __name__ == "__main__":
    import argparse
    import uvicorn
    parser = argparse.ArgumentParser(description="Customer Auth Portal API Server & Temporal Worker")
    parser.add_argument("--port", type=int, default=8000, help="Port to listen on (default: 8000)")
    parser.add_argument("--worker", "--worker-only", action="store_true", dest="worker_only", help="Start dedicated standalone Temporal worker node")
    args = parser.parse_args()

    worker_only = args.worker_only or os.getenv("WORKER_ONLY", "").lower() == "true"
    port = int(os.getenv("PORT", args.port))

    if worker_only:
        engine.start_worker()
    else:
        print("\n==================================================================")
        print(f"🚀 Customer Auth Portal API (Python 3.10+) listening on http://0.0.0.0:{port}")
        print("👷 Embedded Temporal Worker Active on TaskQueue 'auth_portal_api'")
        print(f"📖 Interactive Swagger Docs: http://localhost:{port}/docs")
        print(f"🎨 Sleek Live Visualizer:   http://localhost:{port}/visualizer?domain=customer_auth_portal")
        print("==================================================================\n")
        uvicorn.run(app, host="0.0.0.0", port=port)

