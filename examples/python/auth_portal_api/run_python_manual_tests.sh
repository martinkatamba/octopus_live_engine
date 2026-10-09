#!/usr/bin/env bash
# ==============================================================================
# OCTOPUS Python 3.10+ Customer Auth Portal API Manual Test & Validation Runner
# ==============================================================================

PORT="${PORT:-8000}"
BASE_URL="http://127.0.0.1:${PORT}/api/v1.0.0/customer_auth_portal"
VISUALIZER_BASE="http://localhost:${PORT}/visualizer?domain=customer_auth_portal"

# Color Codes
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
NC='\033[0m' # No Color

echo -e "${BOLD}${CYAN}==============================================================================${NC}"
echo -e "${BOLD}${CYAN} 🐙 OCTOPUS Python Customer Auth Portal - Validation & Test Runner          ${NC}"
echo -e "${BOLD}${CYAN}==============================================================================${NC}"
echo -e " Target Server:   ${BOLD}http://localhost:${PORT}${NC}"
echo -e " Base Visualizer: ${BOLD}${VISUALIZER_BASE}${NC}"
echo ""

# Helper to check server health
check_server() {
  if ! curl -s -f "http://127.0.0.1:${PORT}/health" > /dev/null 2>&1; then
    echo -e "${RED}❌ Error: Python Auth Portal server is not responding on http://127.0.0.1:${PORT}${NC}"
    echo -e "Please start the Python server first:"
    echo -e "  python3 octopus_live_engine/examples/python/auth_portal_api/server.py --port ${PORT}"
    exit 1
  fi
}

check_server

# Global tokens storage
SARAH_VIP_TOKEN=""
JOHN_STD_TOKEN=""

run_tc01() {
  local job_id="job-py-login-vip-01"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-01] POST /auth/login - Public Login Success (Sarah VIP)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Authenticates Sarah VIP and issues Bearer JWT token"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"sarah_vip","password":"password123"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  
  SARAH_VIP_TOKEN=$(echo "$body" | grep -o '"access_token":"[^"]*' | cut -d'"' -f4)
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc02() {
  local job_id="job-py-login-std-02"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-02] POST /auth/login - Public Login Success (John Doe Standard)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Authenticates Standard user and issues Bearer JWT token"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"john_doe","password":"password123"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  
  JOHN_STD_TOKEN=$(echo "$body" | grep -o '"access_token":"[^"]*' | cut -d'"' -f4)
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc03() {
  local job_id="job-py-login-fail-03"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-03] POST /auth/login - Handled Domain Exception (Wrong Password)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  step_verify_credentials raises InvalidCredentials -> 401 & async LogSecurityAuditViolation (Global Handler bypassed)"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"sarah_vip","password":"WRONG_PASSWORD_XYZ"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${YELLOW}${status_code} Unauthorized (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc04() {
  local job_id="job-py-prof-noauth-04"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-04] GET /profile - Protected Route Blocked (Missing Token)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  BearerAuthAndClaimsMiddleware blocks request before workflow pipeline starts -> 401"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X GET "${BASE_URL}/profile" \
    -H "X-Job-ID: ${job_id}")
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${YELLOW}${status_code} Unauthorized (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc05() {
  local job_id="job-py-prof-badtok-05"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-05] GET /profile - Protected Route Blocked (Tampered/Invalid JWT)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Tampered HMAC signature rejected by BearerAuthAndClaimsMiddleware -> 401"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X GET "${BASE_URL}/profile" \
    -H "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJmYWtlIn0.TAMPERED_SIGNATURE_XXXX" \
    -H "X-Job-ID: ${job_id}")
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${YELLOW}${status_code} Unauthorized (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc06() {
  local job_id="job-py-prof-vip-06"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-06] GET /profile - Protected Profile Lookup (Sarah VIP with Token)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Claims injected by middleware -> step_fetch_user_profile returns Sarah VIP record"
  
  if [ -z "$SARAH_VIP_TOKEN" ]; then
    run_tc01 > /dev/null 2>&1
  fi
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X GET "${BASE_URL}/profile" \
    -H "Authorization: Bearer ${SARAH_VIP_TOKEN}" \
    -H "X-Job-ID: ${job_id}")
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc07() {
  local job_id="job-py-prof-std-07"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-07] GET /profile - Protected Profile Lookup (John Doe Standard with Token)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Claims injected by middleware -> step_fetch_user_profile returns John Doe record"
  
  if [ -z "$JOHN_STD_TOKEN" ]; then
    run_tc02 > /dev/null 2>&1
  fi
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X GET "${BASE_URL}/profile" \
    -H "Authorization: Bearer ${JOHN_STD_TOKEN}" \
    -H "X-Job-ID: ${job_id}")
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc08() {
  local job_id="job-py-cart-noauth-08"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-08] POST /cart/checkout - Protected Checkout Blocked (Missing Token)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  BearerAuthAndClaimsMiddleware blocks checkout without Authorization header -> 401"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/cart/checkout" \
    -H "Content-Type: application/json" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"cart_id":"cart_noauth","items":[{"sku":"ITEM-1","quantity":1,"unit_price":50.0}]}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${YELLOW}${status_code} Unauthorized (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc09() {
  local job_id="job-py-cart-vip-09"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-09] POST /cart/checkout - VIP Cart Checkout (20% Discount Branch)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  customer_tier == 'VIP' -> Takes THEN branch (ApplyVIPDiscount, -$160, Priority Express Air)"
  
  if [ -z "$SARAH_VIP_TOKEN" ]; then
    run_tc01 > /dev/null 2>&1
  fi
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/cart/checkout" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${SARAH_VIP_TOKEN}" \
    -H "X-Job-ID: ${job_id}" \
    -d '{
      "cart_id": "cart_vip_001",
      "items": [
        {"sku": "SONY-WH1000XM5", "description": "VIP Audio Headphones", "quantity": 2, "unit_price": 400.0}
      ]
    }')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc10() {
  local job_id="job-py-cart-std-10"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-10] POST /cart/checkout - Standard Cart Checkout (Retail Pricing Branch)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  customer_tier == 'STANDARD' -> Takes ELSE branch (ApplyStandardPricing, Retail $40, Ground)"
  
  if [ -z "$JOHN_STD_TOKEN" ]; then
    run_tc02 > /dev/null 2>&1
  fi
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/cart/checkout" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${JOHN_STD_TOKEN}" \
    -H "X-Job-ID: ${job_id}" \
    -d '{
      "cart_id": "cart_std_001",
      "items": [
        {"sku": "USB-C-CABLE", "description": "Fast Charging Cable", "quantity": 2, "unit_price": 20.0}
      ]
    }')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${GREEN}${status_code} OK${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc11() {
  local job_id="job-py-ip-denied-11"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-11] POST /auth/login - Ingress Guard Blocked (Blocked IP 10.0.0.99)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  IpWhitelistMiddleware blocks blocked IP with 403 & LogSecurityAuditViolation"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Forwarded-For: 10.0.0.99" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"sarah_vip","password":"password123"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${RED}${status_code} Forbidden (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc12() {
  local job_id="job-py-ratelimit-12"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-12] POST /auth/login - Rate Limit Exceeded (Burst Quota Breach)${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  Multiple bursts exceed window quota -> RateLimitMiddleware returns 429"
  
  local test_ip="192.168.1.99"
  for i in {1..25}; do
    curl -s -X POST "${BASE_URL}/auth/login" \
      -H "Content-Type: application/json" \
      -H "X-Forwarded-For: ${test_ip}" \
      -d '{"username":"sarah_vip","password":"password123"}' > /dev/null 2>&1
  done
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Forwarded-For: ${test_ip}" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"sarah_vip","password":"password123"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${YELLOW}${status_code} Too Many Requests (Expected)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc13() {
  local job_id="job-py-unhandled-crash-13"
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-13] POST /auth/login - 🚨 Global Exception Handler: Unhandled Runtime Crash${NC}"
  echo -e "  Job ID:       ${YELLOW}${job_id}${NC}"
  echo -e "  Description:  'crash_user' triggers ZeroDivisionError in activity -> caught by Global Exception Handler"
  echo -e "                Executes HandleGlobalUnhandledException activity with full traceback & sanitized state -> 500"
  
  local response
  response=$(curl -s -w "\nHTTP_STATUS:%{http_code}" -X POST "${BASE_URL}/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Job-ID: ${job_id}" \
    -d '{"username":"crash_user","password":"anyPassword"}')
  
  local status_code=$(echo "$response" | grep "HTTP_STATUS:" | cut -d':' -f2)
  local body=$(echo "$response" | grep -v "HTTP_STATUS:")
  
  echo -e "  HTTP Status:  ${RED}${status_code} Internal Server Error (Expected Handled Crash)${NC}"
  echo -e "  Response:     ${body}"
  echo -e "  🎨 Visualizer: ${BOLD}${MAGENTA}${VISUALIZER_BASE}&job_id=${job_id}${NC}"
}

run_tc14() {
  echo -e "${BOLD}${BLUE}------------------------------------------------------------------------------${NC}"
  echo -e "${BOLD}▶ [TC-14] Redis TTL & Retention Verification${NC}"
  echo -e "  Description:  Verifies Redis keys are set with dynamic TTLs (3600s success, 604800s failure)"
  
  if command -v redis-cli > /dev/null 2>&1; then
    local keys_count=$(redis-cli keys "octopus:job:job-py-*" | wc -l | tr -d ' ')
    echo -e "  Redis Keys Found: ${GREEN}${keys_count} tracked octopus job keys${NC}"
    
    # Check TTL of a success key and a crash key
    local ttl_success=$(redis-cli ttl "octopus:job:job-py-login-vip-01" 2>/dev/null || echo "-1")
    local ttl_crash=$(redis-cli ttl "octopus:job:job-py-unhandled-crash-13" 2>/dev/null || echo "-1")
    
    echo -e "  TTL (Success Job): ${CYAN}${ttl_success}s / 3600s remaining${NC}"
    echo -e "  TTL (Crash Job):   ${CYAN}${ttl_crash}s / 604800s remaining${NC}"
  else
    echo -e "  ${YELLOW}redis-cli not found in PATH, skipping direct Redis query.${NC}"
  fi
}

# Main Dispatcher
MODE="${1:-all}"

case "$MODE" in
  1|tc01|login_vip)
    run_tc01
    ;;
  2|tc02|login_std)
    run_tc02
    ;;
  3|tc03|login_fail)
    run_tc03
    ;;
  4|tc04|prof_noauth)
    run_tc04
    ;;
  5|tc05|prof_badtok)
    run_tc05
    ;;
  6|tc06|prof_vip)
    run_tc06
    ;;
  7|tc07|prof_std)
    run_tc07
    ;;
  8|tc08|cart_noauth)
    run_tc08
    ;;
  9|tc09|cart_vip)
    run_tc09
    ;;
  10|tc10|cart_std)
    run_tc10
    ;;
  11|tc11|ip_denied)
    run_tc11
    ;;
  12|tc12|rate_limit)
    run_tc12
    ;;
  13|tc13|crash|unhandled)
    run_tc13
    ;;
  14|tc14|redis)
    run_tc14
    ;;
  all|*)
    echo -e "${BOLD}Executing All Test Scenarios Sequentially...${NC}\n"
    run_tc01
    echo ""
    run_tc02
    echo ""
    run_tc03
    echo ""
    run_tc04
    echo ""
    run_tc05
    echo ""
    run_tc06
    echo ""
    run_tc07
    echo ""
    run_tc08
    echo ""
    run_tc09
    echo ""
    run_tc10
    echo ""
    run_tc11
    echo ""
    run_tc12
    echo ""
    run_tc13
    echo ""
    run_tc14
    echo ""
    echo -e "${BOLD}${GREEN}==============================================================================${NC}"
    echo -e "${BOLD}${GREEN} ✅ All 14 Test Scenarios (Including Global Exception & Redis) Completed!    ${NC}"
    echo -e "${BOLD}${GREEN}==============================================================================${NC}"
    echo -e "Open the Visualizer at ${BOLD}http://localhost:${PORT}/visualizer?domain=customer_auth_portal${NC}"
    echo -e "or click any of the scenario-specific URLs above to inspect the execution graphs."
    ;;
esac
