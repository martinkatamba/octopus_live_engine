import asyncio
import os
import sys
import time
import json
import httpx

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "src")))

import importlib.util
spec = importlib.util.spec_from_file_location("auth_portal_server", os.path.join(os.path.dirname(__file__), "../examples/python/auth_portal_api/server.py"))
auth_portal_server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth_portal_server)
app = auth_portal_server.app
engine = auth_portal_server.engine


async def test_python():
    print("🐍 ==================== [PYTHON ENGINE TESTS] ====================")
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(transport=transport, base_url="http://test") as client:
        # 1. Login Success
        job_login_ok = f"python-login-success-{int(time.time()*1000)}"
        print(f"\n[Python] 1. POST /auth/login (Job ID: {job_login_ok})")
        res1 = await client.post(
            "/api/v1.0.0/customer_auth_portal/auth/login",
            json={"username": "sarah_vip", "password": "password123"},
            headers={"X-Job-ID": job_login_ok}
        )
        assert res1.status_code == 200, f"Failed: {res1.text}"
        data1 = res1.json()
        token = data1["access_token"]
        print(f"  Status: {res1.status_code} OK | User: {data1.get('user_id')} | Tier: {data1.get('customer_tier')}")

        # 2. VIP Checkout (Protected + Claims + 20% Discount Branch)
        job_checkout = f"python-checkout-vip-{int(time.time()*1000)}"
        print(f"\n[Python] 2. POST /cart/checkout (Job ID: {job_checkout})")
        res2 = await client.post(
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            json={
                "cart_id": "cart_py_001",
                "items": [{"sku": "SONY-WH1000XM5", "description": "VIP Headphones", "quantity": 2, "unit_price": 400.0}]
            },
            headers={"Authorization": f"Bearer {token}", "X-Job-ID": job_checkout}
        )
        assert res2.status_code == 200, f"Failed: {res2.text}"
        data2 = res2.json()
        print(f"  Status: {res2.status_code} OK | Subtotal: ${data2['subtotal']} | Final: ${data2['final_total']} | Tier: {data2['pricing_tier']}")

        # 3. Login Failure (Wrong password -> LogSecurityAuditViolation)
        job_login_fail = f"python-login-fail-{int(time.time()*1000)}"
        print(f"\n[Python] 3. POST /auth/login (Login Failure, Job ID: {job_login_fail})")
        res3 = await client.post(
            "/api/v1.0.0/customer_auth_portal/auth/login",
            json={"username": "sarah_vip", "password": "WRONG_PASSWORD_XYZ"},
            headers={"X-Job-ID": job_login_fail}
        )
        assert res3.status_code == 401, f"Expected 401, got {res3.status_code}"
        print(f"  Status: {res3.status_code} Unauthorized | Detail: {res3.json().get('detail')}")

        # Flush background replication and allow Temporal worker to process queue
        if engine.replication_manager:
            await engine.replication_manager.flush()
            await asyncio.sleep(2.0)

        # Check recorded traces for the failed login
        st = await engine.get_execution_state(job_login_fail)
        print(f"\n[Python] Execution Traces for {job_login_fail}:")
        for t in (st.traces if st else []):
            print(f"   • Step: {t.step_id} | Activity: {t.activity_name} | Status: {t.status} | Output: {t.output_payload}")

    print("\n✅ [PYTHON ENGINE] All tests PASSED successfully!\n")


if __name__ == "__main__":
    asyncio.run(test_python())
