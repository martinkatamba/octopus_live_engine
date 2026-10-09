import urllib.request
import urllib.error
import json
import time
import sys

BASE_URL = "http://127.0.0.1:8090"
PREFIX = "/api/v1.0.0/customer_auth_portal"

opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def get_hdr(hdrs, key):
    if not hdrs:
        return None
    return next((v for k, v in hdrs.items() if k.lower() == key.lower()), None)

def make_req(path, method="GET", body=None, headers=None):
    url = f"{BASE_URL}{path}"
    h = {"Content-Type": "application/json"}
    if headers:
        h.update(headers)
    
    data = json.dumps(body).encode('utf-8') if body is not None else None
    req = urllib.request.Request(url, data=data, headers=h, method=method)
    
    try:
        with opener.open(req) as resp:
            resp_headers = dict(resp.getheaders())
            content = resp.read().decode('utf-8')
            try:
                parsed = json.loads(content)
            except Exception:
                parsed = content
            return resp.status, parsed, resp_headers
    except urllib.error.HTTPError as e:
        content = e.read().decode('utf-8')
        try:
            parsed = json.loads(content)
        except Exception:
            parsed = content
        return e.code, parsed, dict(e.headers)

passed = 0
failed = 0

def check(name, condition, extra=""):
    global passed, failed
    if condition:
        print(f"✅ PASS: {name} {extra}")
        passed += 1
    else:
        print(f"❌ FAIL: {name} {extra}")
        failed += 1

print("\n=======================================================")
print("🧪 RUNNING COMPREHENSIVE TESTS ON .NET AUTH PORTAL API (Port 8090)")
print("=======================================================\n")

# 1. Health Check
st, body, hdrs = make_req("/health")
check("Health Check (200 OK)", st == 200, f"Status={st}, Body={body}")

# 2. Public Login Success (VIP Sarah)
st, body, hdrs = make_req(f"{PREFIX}/auth/login", method="POST", body={"username": "sarah_vip", "password": "password123"})
sarah_token = body.get("access_token") if isinstance(body, dict) else None
sarah_job_id = get_hdr(hdrs, "X-Job-ID")
check("Public Login VIP Success (200 OK)", st == 200 and sarah_token and body.get("customer_tier") == "VIP", f"Status={st}, Tier={body.get('customer_tier') if isinstance(body, dict) else ''}")

# 3. Public Login Success (Standard John)
st, body, hdrs = make_req(f"{PREFIX}/auth/login", method="POST", body={"username": "john_doe", "password": "password123"})
john_token = body.get("access_token") if isinstance(body, dict) else None
check("Public Login Standard Success (200 OK)", st == 200 and john_token and body.get("customer_tier") == "STANDARD", f"Status={st}, Tier={body.get('customer_tier') if isinstance(body, dict) else ''}")

# 4. Public Login Handled Domain Exception (401 Wrong Password)
st, body, hdrs = make_req(f"{PREFIX}/auth/login", method="POST", body={"username": "sarah_vip", "password": "wrong_password"})
login_401_job_id = get_hdr(hdrs, "X-Job-ID")
check("Public Login Handled Domain Exception (401 Unauthorized)", st == 401, f"Status={st}, JobID={login_401_job_id}, Body={body}")

# 5. Public Login Unhandled Runtime Crash (500 Division by zero / unexpected crash)
st, body, hdrs = make_req(f"{PREFIX}/auth/login", method="POST", body={"username": "crash_user", "password": "any_password"})
crash_500_job_id = get_hdr(hdrs, "X-Job-ID")
check("Public Login Unhandled Runtime Crash (500 Internal Server Error)", st == 500, f"Status={st}, JobID={crash_500_job_id}, Body={body}")

# 6. Protected Route Profile without Auth Header (401 UserNotAuthenticated)
st, body, hdrs = make_req(f"{PREFIX}/profile")
check("Protected Profile without Token (401 Unauthorized)", st == 401, f"Status={st}, Body={body}")

# 7. Protected Route Profile with Tampered Token (401 UserNotAuthenticated)
st, body, hdrs = make_req(f"{PREFIX}/profile", headers={"Authorization": "Bearer invalid.token.payload"})
check("Protected Profile with Bad Token (401 Unauthorized)", st == 401, f"Status={st}, Body={body}")

# 8. Protected Route Profile with Sarah VIP Token (200 OK)
st, body, hdrs = make_req(f"{PREFIX}/profile", headers={"Authorization": f"Bearer {sarah_token}"})
check("Protected Profile VIP (200 OK)", st == 200 and body.get("username") == "sarah_vip" and body.get("customer_tier") == "VIP", f"Status={st}, User={body.get('username') if isinstance(body, dict) else ''}")

# 9. Protected Route Profile with John Standard Token (200 OK)
st, body, hdrs = make_req(f"{PREFIX}/profile", headers={"Authorization": f"Bearer {john_token}"})
check("Protected Profile Standard (200 OK)", st == 200 and body.get("username") == "john_doe" and body.get("customer_tier") == "STANDARD", f"Status={st}, User={body.get('username') if isinstance(body, dict) else ''}")

# 10. Cart Checkout VIP (20% discount + Express Shipping)
st, body, hdrs = make_req(f"{PREFIX}/cart/checkout", method="POST", headers={"Authorization": f"Bearer {sarah_token}"}, body={
    "cart_id": "cart_vip_dotnet_01",
    "items": [{"sku": "SONY-WH1000XM5", "description": "Wireless Headphones", "quantity": 2, "unit_price": 400.00}]
})
check("Cart Checkout VIP Branch (200 OK)", st == 200 and body.get("discount_applied") == 160.0 and body.get("final_total") == 640.0, f"Status={st}, Discount={body.get('discount_applied') if isinstance(body, dict) else ''}, Total={body.get('final_total') if isinstance(body, dict) else ''}")

# 11. Cart Checkout Standard (No discount + Ground Shipping)
st, body, hdrs = make_req(f"{PREFIX}/cart/checkout", method="POST", headers={"Authorization": f"Bearer {john_token}"}, body={
    "cart_id": "cart_std_dotnet_02",
    "items": [{"sku": "USB-C-CABLE", "description": "Braided Cable", "quantity": 2, "unit_price": 20.00}]
})
check("Cart Checkout Standard Branch (200 OK)", st == 200 and body.get("discount_applied") == 0.0 and body.get("final_total") == 40.0, f"Status={st}, Discount={body.get('discount_applied') if isinstance(body, dict) else ''}, Total={body.get('final_total') if isinstance(body, dict) else ''}")

# 12. Visualizer Diagram Base API
st, body, hdrs = make_req(f"{PREFIX}/diagram/x6")
check("Visualizer Diagram Base API (200 OK)", st == 200 and "nodes" in body and "edges" in body, f"Status={st}, NodesCount={len(body.get('nodes', [])) if isinstance(body, dict) else 0}")

# 13. Visualizer Diagram with 401 Job Trace (Verify ONLY 401 handler is active, 500 & 429 NOT active)
time.sleep(0.5)
if login_401_job_id:
    st, body, hdrs = make_req(f"{PREFIX}/diagram/x6?job_id={login_401_job_id}")
    nodes = {n["id"]: n for n in body.get("nodes", [])} if isinstance(body, dict) else {}
    
    h401 = nodes.get("node__exc_step_verify_credentials_invalidcredentials") or nodes.get("node__exc_step_verify_credentials_InvalidCredentials") or nodes.get("node__exc_invalidcredentials")
    h500 = nodes.get("node__exc_handleglobalunhandledexception") or nodes.get("node__exc_global_HandleGlobalUnhandledException") or nodes.get("node__exc_global")
    h429 = nodes.get("node__exc_ratelimitexceeded") or nodes.get("node__exc_RateLimitMiddleware_RateLimitExceeded")
    
    h401_status = h401.get("data", {}).get("status") if h401 else None
    h500_status = h500.get("data", {}).get("status") if h500 else None
    h429_status = h429.get("data", {}).get("status") if h429 else None
    
    check("Visualizer 401 Job Trace (401 is COMPLETED/ACTIVE, 500 & 429 SKIPPED)", 
          h401_status in ["FAILED", "COMPLETED", "ACTIVE"] and h500_status in ["SKIPPED", "PENDING"] and h429_status in ["SKIPPED", "PENDING"],
          f"401={h401_status}, 500={h500_status}, 429={h429_status}")

# 14. Visualizer Diagram with 500 Job Trace (Verify ONLY 500 global handler is active, 401 & 429 NOT active)
if crash_500_job_id:
    st, body, hdrs = make_req(f"{PREFIX}/diagram/x6?job_id={crash_500_job_id}")
    nodes = {n["id"]: n for n in body.get("nodes", [])} if isinstance(body, dict) else {}
    
    h401 = nodes.get("node__exc_step_verify_credentials_invalidcredentials") or nodes.get("node__exc_step_verify_credentials_InvalidCredentials") or nodes.get("node__exc_invalidcredentials")
    h500 = nodes.get("node__exc_handleglobalunhandledexception") or nodes.get("node__exc_global_HandleGlobalUnhandledException") or nodes.get("node__exc_global")
    h429 = nodes.get("node__exc_ratelimitexceeded") or nodes.get("node__exc_RateLimitMiddleware_RateLimitExceeded")
    
    h500_status = h500.get("data", {}).get("status") if h500 else None
    h401_status = h401.get("data", {}).get("status") if h401 else None
    h429_status = h429.get("data", {}).get("status") if h429 else None
    
    check("Visualizer 500 Job Trace (500 is COMPLETED/ACTIVE, 401 & 429 SKIPPED)", 
          h500_status in ["FAILED", "COMPLETED", "ACTIVE"] and h401_status in ["SKIPPED", "PENDING"] and h429_status in ["SKIPPED", "PENDING"],
          f"500={h500_status}, 401={h401_status}, 429={h429_status}")

print(f"\n=======================================================")
print(f"📊 Summary: {passed} PASSED, {failed} FAILED out of {passed + failed} tests.")
print(f"=======================================================\n")

if failed > 0:
    sys.exit(1)
