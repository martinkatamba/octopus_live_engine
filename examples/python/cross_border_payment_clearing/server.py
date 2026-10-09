import os
import sys

# Ensure octopus_live package is discoverable
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))

from octopus_live.engine import OctopusLiveEngine
from models import FxRateResponse, PaymentTransferRequest, PaymentTransferResponse
from activities import PaymentClearingActivities

# 1. Load Engine from YAML Workflow
yaml_path = os.path.join(os.path.dirname(__file__), "payment_workflow.yaml")
activities_handler = PaymentClearingActivities()
engine = OctopusLiveEngine.from_yaml(
    yaml_path,
    activity_instance=activities_handler
)

# 2. Register Pydantic Models for OpenAPI Schema & Dynamic Validation
engine.register_model("FxRateResponse", FxRateResponse)
engine.register_model("PaymentTransferRequest", PaymentTransferRequest)
engine.register_model("PaymentTransferResponse", PaymentTransferResponse)

# 3. Create FastAPI Application
app = engine.create_api_app(title="Cross-Border Payment & Instant FX Clearing API")

if __name__ == "__main__":
    import argparse
    import uvicorn

    parser = argparse.ArgumentParser(description="Cross-Border Payment & FX Clearing API Server")
    parser.add_argument("--port", type=int, default=8005, help="Port to listen on (default: 8005)")
    args = parser.parse_args()

    port = int(os.getenv("PORT", args.port))

    print("\n==================================================================")
    print(f"🚀 Cross-Border Payment Clearing API listening on http://0.0.0.0:{port}")
    print(f"📖 Interactive Swagger Docs: http://localhost:{port}/docs")
    print(f"🎨 Sleek Live Visualizer:   http://localhost:{port}/visualizer?domain=cross_border_payment_clearing")
    print("==================================================================\n")
    uvicorn.run(app, host="0.0.0.0", port=port)
