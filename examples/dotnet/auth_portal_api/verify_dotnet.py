import urllib.request
import urllib.error
import json

base_url = "http://127.0.0.1:8090"

def get(path, headers=None):
    req = urllib.request.Request(f"{base_url}{path}", headers=headers or {})
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, resp.headers, resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read().decode("utf-8")

def post(path, body=None, headers=None):
    hdrs = {"Content-Type": "application/json"}
    if headers:
        hdrs.update(headers)
    data = json.dumps(body).encode("utf-8") if body else None
    req = urllib.request.Request(f"{base_url}{path}", data=data, headers=hdrs, method="POST")
    try:
        with urllib.request.urlopen(req) as resp:
            return resp.status, resp.headers, resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read().decode("utf-8")

print("1. Testing Health...")
s, _, b = get("/health")
assert s == 200, f"Expected 200, got {s}"
print("   Health OK:", b)

print("\n2. Testing Visualizer UI...")
s, _, b = get("/visualizer?domain=customer_auth_portal")
assert s == 200 and "AntV" in b, f"Visualizer failed: {s}"
print("   Visualizer UI rendered OK")

print("\n3. Testing X6 Diagram Compilation...")
s, _, b = get("/api/v1.0.0/customer_auth_portal/diagram/x6")
assert s == 200, f"Diagram failed: {s}"
diagram = json.loads(b)
nodes_count = len(diagram.get("nodes", []))
edges_count = len(diagram.get("edges", []))
print(f"   Diagram OK: {nodes_count} nodes, {edges_count} edges")

print("\n4. Testing Public Login (Sarah VIP)...")
s, _, b = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "sarah_vip", "password": "password123"})
assert s == 200, f"Login failed: {s}, {b}"
login_data = json.loads(b)
sarah_token = login_data["access_token"]
user_id = login_data["user_id"]
customer_tier = login_data["customer_tier"]
print(f"   Login OK: UserID={user_id}, Tier={customer_tier}, Token={sarah_token[:25]}...")

print("\n5. Testing Public Login with Bad Password (Declarative 401)...")
s, _, b = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "sarah_vip", "password": "bad"})
assert s == 401, f"Expected 401, got {s}"
err_doc = json.loads(b)
print("   Bad password blocked OK:", err_doc.get("error"), err_doc.get("detail"))

print("\n6. Testing Protected Profile without Token (Declarative 401)...")
s, _, b = get("/api/v1.0.0/customer_auth_portal/profile")
assert s == 401, f"Expected 401, got {s}"
err_doc = json.loads(b)
print("   Missing token blocked OK:", err_doc.get("detail"))

print("\n7. Testing Protected Profile with Bearer Token...")
s, _, b = get("/api/v1.0.0/customer_auth_portal/profile", headers={"Authorization": f"Bearer {sarah_token}"})
assert s == 200, f"Profile lookup failed: {s}, {b}"
profile = json.loads(b)
print(f"   Profile OK: {profile.get('username')} ({profile.get('email')}), Tier={profile.get('customer_tier')}, Status={profile.get('account_status')}")

print("\n8. Testing VIP Shopping Cart Checkout (20% discount + Priority Express Air)...")
cart_payload = {
    "cart_id": "cart_vip_001",
    "items": [{"sku": "SONY-WH1000XM5", "description": "Wireless Headphones", "quantity": 2, "unit_price": 400.00}]
}
s, _, b = post("/api/v1.0.0/customer_auth_portal/cart/checkout", cart_payload, headers={"Authorization": f"Bearer {sarah_token}"})
assert s == 200, f"VIP Checkout failed: {s}, {b}"
vip_order = json.loads(b)
print(f"   VIP Checkout OK: Subtotal=${vip_order.get('subtotal')}, Discount=-${vip_order.get('discount_applied')}, FinalTotal=${vip_order.get('final_total')}, Pricing={vip_order.get('pricing_tier')}, Shipping={vip_order.get('shipping_method')}")
assert vip_order["discount_applied"] == 160.00
assert vip_order["final_total"] == 640.00
assert vip_order["shipping_method"] == "FREE_PRIORITY_EXPRESS_AIR"

print("\n9. Testing Standard Shopping Cart Checkout (John Doe - Standard Pricing)...")
s_std, _, b_std = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "john_doe", "password": "password123"})
john_token = json.loads(b_std)["access_token"]

cart_payload_std = {
    "cart_id": "cart_std_002",
    "items": [{"sku": "USB-C-CABLE", "description": "Braided Cable", "quantity": 2, "unit_price": 20.00}]
}
s, _, b = post("/api/v1.0.0/customer_auth_portal/cart/checkout", cart_payload_std, headers={"Authorization": f"Bearer {john_token}"})
assert s == 200, f"Standard Checkout failed: {s}, {b}"
std_order = json.loads(b)
print(f"   Standard Checkout OK: Subtotal=${std_order.get('subtotal')}, Discount=-${std_order.get('discount_applied')}, FinalTotal=${std_order.get('final_total')}, Pricing={std_order.get('pricing_tier')}, Shipping={std_order.get('shipping_method')}")
assert std_order["discount_applied"] == 0.00
assert std_order["final_total"] == 40.00
assert std_order["shipping_method"] == "STANDARD_GROUND_SHIPPING"

print("\n10. Testing IP Whitelist Blacklist Block (10.0.0.99 -> 403)...")
s, _, b = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "sarah_vip", "password": "password123"}, headers={"X-Forwarded-For": "10.0.0.99"})
assert s == 403, f"Expected 403 for blacklisted IP, got {s}"
err_doc = json.loads(b)
print("   Blacklisted IP blocked OK:", err_doc.get("error"), err_doc.get("detail"))

print("\n11. Testing Rate Limit Exhaustion (20 req/min for ANONYMOUS -> 429 on 21st)...")
unique_ip = "192.168.1.189"
for i in range(20):
    s, _, _ = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "sarah_vip", "password": "password123"}, headers={"X-Forwarded-For": unique_ip})
    assert s == 200, f"Request {i+1} failed with {s}"
s_rate, hdrs, b_rate = post("/api/v1.0.0/customer_auth_portal/auth/login", {"username": "sarah_vip", "password": "password123"}, headers={"X-Forwarded-For": unique_ip})
assert s_rate == 429, f"Expected 429 on 21st request, got {s_rate}"
err_doc = json.loads(b_rate)
print("   Rate limit exceeded OK: 429 Too Many Requests, Retry-After =", hdrs.get("Retry-After"), "X-RateLimit-Limit =", hdrs.get("X-RateLimit-Limit"))

print("\n🎉 ALL 11 TEST VERIFICATIONS PASSED WITH 100% SUCCESS IN .NET 8!")
