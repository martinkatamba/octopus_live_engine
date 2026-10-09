import os
import sys
import time
from typing import Any, Dict, List, Optional
from pydantic import BaseModel, Field

# Ensure octopus_live package is discoverable
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))

from octopus_live.engine import OctopusLiveEngine
from octopus_live.domain.services.middleware import BearerAuthAndClaimsMiddleware, CorrelationIdMiddleware


# 1. Pydantic Models for OpenAPI Schema & Strict Type Safety
class LoginRequest(BaseModel):
    username: str = Field(..., example="sarah_vip")
    password: str = Field(..., example="secret123")
    requested_tier: str = Field(default="VIP", example="VIP")


class LoginResponse(BaseModel):
    access_token: str
    token_type: str = "Bearer"
    user_id: str
    customer_tier: str
    expires_in_seconds: int = 3600


class CartItem(BaseModel):
    sku: str = Field(..., example="LAPTOP-PRO-16")
    description: str = Field(default="High-performance laptop", example="High-performance laptop")
    quantity: int = Field(default=1, example=1)
    unit_price: float = Field(..., example=1999.00)


class CheckoutTaxRequest(BaseModel):
    cart_id: str = Field(default="cart_88219", example="cart_88219")
    destination_country: str = Field(default="US", example="US")
    destination_state: str = Field(default="CA", example="CA")
    items: List[CartItem] = Field(default_factory=list)


class TaxLineItem(BaseModel):
    sku: str
    taxable_amount: float
    tax_rate: float
    tax_amount: float


class CheckoutTaxResponse(BaseModel):
    calculation_id: str
    customer_id: str
    customer_tier: str
    subtotal: float
    total_tax: float
    total_with_tax: float
    applied_rule: str
    line_items: List[TaxLineItem]
    fast_path_latency_ms: float = 0.42


# 2. In-Memory Pure Domain Activities
class TaxCalculationActivities:
    @staticmethod
    def GenerateAuthToken(payload: dict) -> dict:
        username = payload.get("username", "guest")
        tier = payload.get("requested_tier", "VIP").upper()
        user_id = f"usr_{hash(username) % 10000}"
        
        # Simulates issuing a signed Bearer token containing embedded claims
        token = f"mock-jwt-{tier}-{user_id}-{int(time.time())}"
        return {
            "access_token": token,
            "token_type": "Bearer",
            "user_id": user_id,
            "customer_tier": tier,
            "expires_in_seconds": 3600
        }

    @staticmethod
    def NormalizeCartAndAddress(payload: dict) -> dict:
        items = payload.get("items", [])
        subtotal = sum(item.get("unit_price", 0.0) * item.get("quantity", 1) for item in items)
        if subtotal == 0.0:
            subtotal = 1999.00  # Default demo value
            items = [{"sku": "LAPTOP-PRO-16", "quantity": 1, "unit_price": 1999.00}]

        return {
            "subtotal": round(subtotal, 2),
            "normalized_items": items,
            "cart_id": payload.get("cart_id", "cart_default")
        }

    @staticmethod
    def CalculateVIPTax(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        items = payload.get("normalized_items", [])
        rate = 0.045  # 4.5% VIP tax privilege
        
        line_items = [
            {
                "sku": it.get("sku", "SKU"),
                "taxable_amount": round(it.get("unit_price", 0.0) * it.get("quantity", 1), 2),
                "tax_rate": rate,
                "tax_amount": round(it.get("unit_price", 0.0) * it.get("quantity", 1) * rate, 2)
            }
            for it in items
        ]
        total_tax = round(sum(li["tax_amount"] for li in line_items), 2)

        return {
            "applied_rule": "VIP_PREFERENTIAL_TAX_RATE (4.5%)",
            "effective_tax_rate": rate,
            "total_tax": total_tax,
            "line_items": line_items
        }

    @staticmethod
    def CalculateStandardTax(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        items = payload.get("normalized_items", [])
        rate = 0.0925  # 9.25% Standard state & local tax
        
        line_items = [
            {
                "sku": it.get("sku", "SKU"),
                "taxable_amount": round(it.get("unit_price", 0.0) * it.get("quantity", 1), 2),
                "tax_rate": rate,
                "tax_amount": round(it.get("unit_price", 0.0) * it.get("quantity", 1) * rate, 2)
            }
            for it in items
        ]
        total_tax = round(sum(li["tax_amount"] for li in line_items), 2)

        return {
            "applied_rule": "STANDARD_JURISDICTION_TAX (9.25%)",
            "effective_tax_rate": rate,
            "total_tax": total_tax,
            "line_items": line_items
        }

    @staticmethod
    def FinalizeTaxSummary(payload: dict) -> dict:
        subtotal = payload.get("subtotal", 0.0)
        total_tax = payload.get("total_tax", 0.0)
        tier = payload.get("customer_tier", "STANDARD")
        customer_id = payload.get("customer_id") or payload.get("user_id", "usr_anon")

        return {
            "calculation_id": f"tax_calc_{int(time.time() * 1000)}",
            "customer_id": customer_id,
            "customer_tier": tier,
            "subtotal": subtotal,
            "total_tax": total_tax,
            "total_with_tax": round(subtotal + total_tax, 2),
            "applied_rule": payload.get("applied_rule", "DEFAULT_TAX"),
            "line_items": payload.get("line_items", []),
            "fast_path_latency_ms": 0.38
        }


# 3. Mock JWT / Bearer Token Validator
def validate_bearer_token(token: str) -> dict:
    # In production: jwt.decode(token, SECRET_KEY, algorithms=["HS256"])
    if not token or "bad" in token or "invalid" in token:
        raise ValueError("Invalid signature or expired token")

    # Extract claims from demo token format: mock-jwt-{TIER}-{USER_ID}-{TIMESTAMP}
    tier = "VIP" if "VIP" in token else "STANDARD"
    user_id = "usr_vip_88" if "VIP" in token else "usr_std_12"

    return {
        "sub": user_id,
        "customer_tier": tier,
        "roles": ["customer", "shopper"]
    }


# 4. Engine Initialization & Middleware Registration
yaml_path = os.path.join(os.path.dirname(__file__), "checkout_tax_workflow.yaml")
engine = OctopusLiveEngine.from_yaml(
    yaml_path,
    activity_instance=TaxCalculationActivities()
)

# Register Pydantic models for OpenAPI schema generation
engine.register_model("LoginRequest", LoginRequest)
engine.register_model("LoginResponse", LoginResponse)
engine.register_model("CheckoutTaxRequest", CheckoutTaxRequest)
engine.register_model("CheckoutTaxResponse", CheckoutTaxResponse)

# Register BearerAuthAndClaimsMiddleware implementation
engine.register_middleware(
    "BearerAuthAndClaimsMiddleware",
    BearerAuthAndClaimsMiddleware(
        token_validator=validate_bearer_token,
        claims_mapper=lambda claims: {
            "customer_tier": claims.get("customer_tier", "STANDARD"),
            "customer_id": claims.get("sub"),
            "user_id": claims.get("sub")
        }
    )
)

# Create FastAPI application
app = engine.create_api_app(title="Checkout Tax Calculator API")

if __name__ == "__main__":
    import argparse
    import uvicorn
    parser = argparse.ArgumentParser(description="Checkout Tax Calculator Server")
    parser.add_argument("--port", type=int, default=8099, help="Port to listen on")
    args = parser.parse_args()
    uvicorn.run(app, host="0.0.0.0", port=args.port)
