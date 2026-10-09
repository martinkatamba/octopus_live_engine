import os
import time
from typing import Optional
from pydantic import BaseModel, Field
from octopus_live import LiveEngine

# --------------------------------------------------------------------------
# ZERO-IMPORT DEVELOPER EXPERIENCE:
# Notice there are NO imports of 'temporalio', 'duckdb', 'redis', or 'sqlite3'!
# All connection management, drivers, and background replication are handled
# automatically by the OCTOPUS Live Engine.
# --------------------------------------------------------------------------


class PaymentRequest(BaseModel):
    account_id: str = Field(..., description="Customer account ID")
    amount: float = Field(..., description="Payment amount")
    currency: str = Field(default="USD", description="Currency code (USD, EUR, GBP)")
    card_token: str = Field(..., description="Tokenized payment instrument")
    job_id: Optional[str] = Field(None, description="Idempotency or tracking job ID")


class PaymentResponse(BaseModel):
    transaction_id: str
    status: str
    authorized_amount: float
    currency: str
    fraud_score: float
    fast_path_latency_ms: float
    durable_tracking: str


class PaymentActivities:
    """Native in-memory activities executing with zero network overhead."""

    @staticmethod
    def ScreenFraud(payload: dict) -> dict:
        amount = payload.get("amount", 0.0)
        score = 0.02 if amount < 1000 else 0.85
        return {
            "fraud_score": score,
            "fraud_decision": "APPROVE" if score < 0.5 else "REVIEW"
        }

    @staticmethod
    def AuthorizeCard(payload: dict) -> dict:
        return {
            "transaction_id": f"txn_{int(time.time() * 1000)}",
            "auth_code": "AUTH_9921",
            "settled": True
        }

    @staticmethod
    def FormatPaymentResponse(payload: dict) -> dict:
        return {
            "transaction_id": payload.get("transaction_id", "txn_unknown"),
            "status": "AUTHORIZED",
            "authorized_amount": payload.get("amount", 0.0),
            "currency": payload.get("currency", "USD"),
            "fraud_score": payload.get("fraud_score", 0.0),
            "fast_path_latency_ms": 0.45,
            "durable_tracking": "REPLICATED_TO_TEMPORAL_AND_SQLITE"
        }


# Initialize LiveEngine directly from YAML with Zero-Import configuration
yaml_path = os.path.join(os.path.dirname(__file__), "payment_workflow.yaml")
engine = LiveEngine.from_yaml(
    yaml_path,
    activity_instance=PaymentActivities()
)

# Register models for strict typing and OpenAPI schema generation
engine.register_model("PaymentRequest", PaymentRequest)
engine.register_model("PaymentResponse", PaymentResponse)

# FastAPI application with interactive Visualizer and sub-millisecond API endpoints
app = engine.create_api_app(title="Hybrid Payment Gateway API")

if __name__ == "__main__":
    import argparse
    import uvicorn
    parser = argparse.ArgumentParser(description="OCTOPUS Hybrid Payment API Server")
    parser.add_argument("--port", type=int, default=8000, help="Port to listen on")
    args = parser.parse_args()
    uvicorn.run(app, host="0.0.0.0", port=args.port)
