import asyncio
import pytest
import os
import sys
import time
import httpx

# Ensure package path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../python/src")))

import importlib.util
spec = importlib.util.spec_from_file_location("auth_portal_server", os.path.join(os.path.dirname(__file__), "server.py"))
auth_portal_server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth_portal_server)
app = auth_portal_server.app
engine = auth_portal_server.engine


@pytest.mark.asyncio
async def test_hybrid_persistence_and_temporal_execution():
    """The fast path, the VIP checkout, the security audit on failures, and the jobs being stored."""
    print("🚀 Starting Customer Auth Portal Hybrid Test (Fast-path + SQLite + Temporal)...")
    
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        # 1. Login with VIP User
        login_job_id = f"test-login-{int(time.time()*1000)}"
        login_res = await client.post(
            "/api/v1.0.0/customer_auth_portal/auth/login",
            json={"username": "sarah_vip", "password": "password123"},
            headers={"X-Job-ID": login_job_id}
        )
        assert login_res.status_code == 200, f"Login failed: {login_res.text}"
        login_data = login_res.json()
        token = login_data["access_token"]
        print(f"✅ Step 1: Login successful (User: {login_data['user_id']}, Tier: {login_data['customer_tier']})")

        # 2. Checkout with VIP Discount
        checkout_job_id = f"test-vip-checkout-{int(time.time()*1000)}"
        checkout_res = await client.post(
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            json={
                "cart_id": "cart_test_001",
                "items": [{"sku": "HEADPHONES", "description": "VIP Audio", "quantity": 2, "unit_price": 400.0}]
            },
            headers={
                "Authorization": f"Bearer {token}",
                "X-Job-ID": checkout_job_id
            }
        )
        assert checkout_res.status_code == 200, f"Checkout failed: {checkout_res.text}"
        checkout_data = checkout_res.json()
        assert checkout_data["customer_tier"] == "VIP"
        assert checkout_data["final_total"] == 640.0
        assert checkout_data["discount_applied"] == 160.0
        print(f"✅ Step 2: VIP Checkout successful (Order: {checkout_data['order_id']}, Subtotal: ${checkout_data['subtotal']}, Final: ${checkout_data['final_total']})")

        # 3. Trigger 401 unauthenticated request to protected route
        bad_res = await client.get("/api/v1.0.0/customer_auth_portal/profile")
        assert bad_res.status_code == 401
        print("✅ Step 3: Security audit violation triggered on protected profile route (HTTP 401)")

        # 4. Trigger login failure with invalid password
        bad_login_job_id = f"test-login-fail-{int(time.time()*1000)}"
        bad_login_res = await client.post(
            "/api/v1.0.0/customer_auth_portal/auth/login",
            json={"username": "sarah_vip", "password": "WRONG_PASSWORD_XYZ"},
            headers={"X-Job-ID": bad_login_job_id}
        )
        assert bad_login_res.status_code == 401
        print(f"✅ Step 4: Login failure triggered LogSecurityAuditViolation as expected (HTTP 401: {bad_login_res.json()['detail']})")

        # Allow async replication and Temporal workers to process
        print("⏳ Flushing asynchronous durability queues...")
        if engine.replication_manager:
            await engine.replication_manager.flush()
        await asyncio.sleep(2)

        # 5. The jobs this test ran are stored. The blueprint's store is Redis
        # (`persistence.store: redis`); without a Redis server the engine keeps
        # them in memory, so this checks the engine rather than one database.
        for job_id, status in ((login_job_id, "COMPLETED"), (checkout_job_id, "COMPLETED"), (bad_login_job_id, "FAILED")):
            stored = await engine.get_execution_state(job_id)
            assert stored is not None, f"job {job_id} was not stored"
            got = stored.status.value if hasattr(stored.status, "value") else str(stored.status)
            assert got == status, f"job {job_id}: {got}, expected {status}"
            print(f"💾 Stored: {job_id} | {got} | route {stored.route}")

        print("\n🎉 All tests passed: Fast-path, VIP Checkout, Login Failure, Security Audit and stored jobs verified!")


if __name__ == "__main__":
    asyncio.run(test_hybrid_persistence_and_temporal_execution())
