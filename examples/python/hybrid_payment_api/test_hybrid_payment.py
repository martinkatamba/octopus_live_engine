import os
import sys
import time
import pytest
from fastapi.testclient import TestClient

# Add python source and example dir to sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))
sys.path.insert(0, os.path.abspath(os.path.dirname(__file__)))

from octopus_live import LiveEngine, SqliteStateStore, StoreFactory
try:
    from octopus_live_engine.examples.python.hybrid_payment_api.server import app, engine, PaymentRequest, PaymentResponse
except ImportError:
    from server import app, engine, PaymentRequest, PaymentResponse


@pytest.fixture
def client():
    c = TestClient(app)
    # Warm up client routing
    c.get("/health")
    return c


def test_hybrid_payment_fast_path_latency(client):
    """Test that synchronous fast-path returns immediate HTTP response with exact output payload."""
    payload = {
        "job_id": f"pay-live-{int(time.time() * 1000)}",
        "account_id": "acc_cust_8821",
        "amount": 299.50,
        "currency": "USD",
        "card_token": "tok_visa_4242"
    }

    # Warmup request
    client.post("/api/v1/payments/authorize", json=payload)

    # Measured request
    start = time.perf_counter()
    response = client.post("/api/v1/payments/authorize", json=payload)
    elapsed_ms = (time.perf_counter() - start) * 1000.0

    assert response.status_code == 200
    data = response.json()

    # Verify is_output response model schema
    assert data["status"] == "AUTHORIZED"
    assert data["authorized_amount"] == 299.50
    assert data["currency"] == "USD"
    assert data["durable_tracking"] == "REPLICATED_TO_TEMPORAL_AND_SQLITE"
    assert data["fraud_score"] == 0.02
    assert "transaction_id" in data
    assert elapsed_ms < 100.0  # Fast-path execution in test harness


@pytest.mark.asyncio
async def test_durable_sqlite_and_temporal_replication():
    """Test that state and step traces are durably persisted in SQLite and replicated to Temporal."""
    job_id = f"pay-durable-{int(time.time() * 1000)}"
    req = {
        "job_id": job_id,
        "account_id": "acc_cust_9901",
        "amount": 1250.00,
        "currency": "EUR",
        "card_token": "tok_mastercard_5555"
    }

    start_exec = time.perf_counter()
    state = await engine.execute_sync("hybrid_payment_gateway", req)
    exec_ms = (time.perf_counter() - start_exec) * 1000.0

    assert exec_ms < 5.0  # Sub-5ms pure engine execution
    assert state.status.value == "COMPLETED"
    assert state.job_id == job_id
    assert len(state.traces) == 3

    # Wait for background replication manager to flush
    if engine.replication_manager:
        await engine.replication_manager.flush()

    # Query durable state store
    saved_state = await engine.state_store.get_state(job_id)
    assert saved_state is not None
    assert saved_state.job_id == job_id
    assert saved_state.status.value == "COMPLETED"
    assert len(saved_state.traces) == 3

    # Check input and output payloads preserved in durable storage
    fraud_trace = saved_state.traces[0]
    assert fraud_trace.activity_name == "ScreenFraud"
    assert fraud_trace.input_payload["amount"] == 1250.00
    assert fraud_trace.output_payload["fraud_decision"] == "REVIEW"

    auth_trace = saved_state.traces[1]
    assert auth_trace.activity_name == "AuthorizeCard"
    assert auth_trace.output_payload["auth_code"] == "AUTH_9921"

    output_trace = saved_state.traces[2]
    assert output_trace.activity_name == "FormatPaymentResponse"
    assert output_trace.output_payload["status"] == "AUTHORIZED"

    # Verify Temporal replication queue
    if engine.replication_manager and engine.replication_manager.temporal_replicator:
        assert job_id in engine.replication_manager.temporal_replicator._replicated_jobs


def test_zero_import_store_factory():
    """Verify that StoreFactory creates SQLite, DuckDB, Redis, and InMemory stores dynamically."""
    sqlite_store = StoreFactory.create("sqlite", path=":memory:")
    assert isinstance(sqlite_store, SqliteStateStore)

    duck_store = StoreFactory.create("duckdb", path=":memory:")
    assert duck_store is not None

    redis_store = StoreFactory.create("redis", url="redis://localhost:6379")
    assert redis_store is not None

    mem_store = StoreFactory.create("inmemory")
    assert mem_store is not None
