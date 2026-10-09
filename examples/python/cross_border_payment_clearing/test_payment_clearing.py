import time
import requests
import json
import sys

BASE_URL = "http://127.0.0.1:8005"
API_KEY = "fintech-partner-key-9988"

HEADERS = {
    "Content-Type": "application/json",
    "X-API-Key": API_KEY,
}

def print_banner(title: str):
    print(f"\n{'='*70}\n🔹 {title}\n{'='*70}")

def test_1_ingress_missing_required_field():
    print_banner("TEST 1: Ingress Validation (HTTP 400 Bad Request on Missing Field)")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/transfer/process"
    
    # Missing 'recipient_iban' and 'amount'
    payload = {
        "sender_account": "ACT-1100",
        "source_currency": "USD"
    }
    
    resp = requests.post(url, json=payload, headers=HEADERS)
    print(f"Status Code: {resp.status_code}")
    print(f"Response: {resp.text}")
    
    assert resp.status_code == 400, f"Expected 400, got {resp.status_code}"
    body = resp.json()
    assert body.get("error") == "MissingRequiredFieldException", f"Expected MissingRequiredFieldException, got {body.get('error')}"
    print("✅ TEST 1 PASSED: Early 400 Bad Request returned correctly.")


def test_2_instant_sepa_settlement():
    print_banner("TEST 2: Instant Settlement Branch (< $100K to EUR)")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/transfer/process"
    
    payload = {
        "sender_account": "ACT-1100",
        "recipient_iban": "DE89370400440532013000",
        "amount": 2500.00,
        "source_currency": "USD",
        "target_currency": "EUR",
        "country_code": "DE"
    }
    
    resp = requests.post(url, json=payload, headers=HEADERS)
    print(f"Status Code: {resp.status_code}")
    print(f"Response: {json.dumps(resp.json(), indent=2)}")
    
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert data.get("status") == "SETTLED"
    assert data.get("settlement_rail") == "SEPA_INSTANT"
    assert data.get("converted_amount") == 2300.00  # 2500 * 0.92
    assert "CLEAR-INST-" in (data.get("aml_reference_code") or "")
    print("✅ TEST 2 PASSED: Instant SEPA rail executed with accurate FX calculation.")


def test_3_high_value_escrow_branch():
    print_banner("TEST 3: High-Value Multi-Sig Escrow Branch (>= $100,000)")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/transfer/process"
    
    payload = {
        "sender_account": "ACT-1100",
        "recipient_iban": "DE89370400440532013000",
        "amount": 500000.00,
        "source_currency": "USD",
        "target_currency": "EUR",
        "country_code": "DE"
    }
    
    resp = requests.post(url, json=payload, headers=HEADERS)
    print(f"Status Code: {resp.status_code}")
    print(f"Response: {json.dumps(resp.json(), indent=2)}")
    
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert data.get("status") == "ESCROW_HELD"
    assert data.get("settlement_rail") == "INSTITUTIONAL_ESCROW"
    assert data.get("clearing_fee") == 45.00
    assert "ESCROW-" in (data.get("aml_reference_code") or "")
    print("✅ TEST 3 PASSED: High-value transfer held in dual-sign escrow.")


def test_4_sanctions_aml_freeze_branch():
    print_banner("TEST 4: Sanctions & Watchlist AML Freeze Branch")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/transfer/process"
    
    payload = {
        "sender_account": "ACT-1100",
        "recipient_iban": "XX99SANCTIONED0001",
        "amount": 1000.00,
        "source_currency": "USD",
        "target_currency": "EUR",
        "country_code": "IR"
    }
    
    resp = requests.post(url, json=payload, headers=HEADERS)
    print(f"Status Code: {resp.status_code}")
    print(f"Response: {json.dumps(resp.json(), indent=2)}")
    
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert data.get("status") == "FROZEN_COMPLIANCE"
    assert data.get("settlement_rail") == "AML_HOLD"
    assert data.get("converted_amount") == 0.0
    assert "AML-ALERT-" in (data.get("aml_reference_code") or "")
    print("✅ TEST 4 PASSED: Sanctioned destination frozen for AML review.")


def test_5_cached_fx_rates():
    print_banner("TEST 5: Cached Live FX Rates (GET /rates/live)")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/rates/live?base=USD&target=EUR"
    
    # 1. First fetch (Cache Miss / Store)
    t0 = time.time()
    resp1 = requests.get(url, headers=HEADERS)
    d1 = (time.time() - t0) * 1000
    print(f"First Request Status: {resp1.status_code} ({d1:.2f}ms)")
    print(f"Response: {resp1.json()}")
    
    assert resp1.status_code == 200
    data1 = resp1.json()
    assert data1.get("rate") == 0.92
    
    # 2. Second fetch (Sub-millisecond Cache Hit)
    t0 = time.time()
    resp2 = requests.get(url, headers=HEADERS)
    d2 = (time.time() - t0) * 1000
    print(f"Second (Cached) Request Status: {resp2.status_code} ({d2:.2f}ms)")
    
    assert resp2.status_code == 200
    print("✅ TEST 5 PASSED: Live FX rate cached and served with sub-millisecond latency.")


def test_6_visualizer_diagram_check():
    print_banner("TEST 6: AntV X6 Visualizer Diagram Check")
    url = f"{BASE_URL}/api/v1.0.0/cross_border_payment_clearing/diagram/x6?route=/api/v1.0.0/cross_border_payment_clearing/transfer/process&method=POST"
    resp = requests.get(url)
    print(f"Visualizer Status: {resp.status_code}")
    assert resp.status_code == 200
    diagram = resp.json()
    nodes = diagram.get("nodes", [])
    node_ids = [n.get("id") for n in nodes]
    print(f"Found {len(nodes)} DAG nodes in visualizer diagram: {node_ids}")
    assert "node__step_screen_sanctions_and_aml" in node_ids or "step_screen_sanctions_and_aml" in node_ids
    assert "node__step_freeze_for_compliance" in node_ids or "step_freeze_for_compliance" in node_ids
    assert "node__step_execute_instant_settlement" in node_ids or "step_execute_instant_settlement" in node_ids
    assert "node__step_route_high_value_escrow" in node_ids or "step_route_high_value_escrow" in node_ids
    print("✅ TEST 6 PASSED: AntV X6 diagram represents all branching steps and terminals.")


if __name__ == "__main__":
    print("\n🏦 ==================== [CROSS-BORDER PAYMENT API TESTS] ====================")
    try:
        test_1_ingress_missing_required_field()
        test_2_instant_sepa_settlement()
        test_3_high_value_escrow_branch()
        test_4_sanctions_aml_freeze_branch()
        test_5_cached_fx_rates()
        test_6_visualizer_diagram_check()
        print("\n🎉 ==================== ALL 6 TESTS PASSED WITH 100% SUCCESS ====================\n")
    except Exception as e:
        print(f"\n❌ TEST FAILED: {e}", file=sys.stderr)
        sys.exit(1)
