package main

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

func init() {
	gin.SetMode(gin.TestMode)
}

func setupTestRouter(t *testing.T) *gin.Engine {
	_, router, err := BuildApp("auth_portal_workflow.yaml")
	if err != nil {
		t.Fatalf("BuildApp failed: %v", err)
	}
	return router
}

func TestPublicLoginSuccessAndTokenGeneration(t *testing.T) {
	router := setupTestRouter(t)

	body, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	req, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK, got %d. Body: %s", w.Code, w.Body.String())
	}

	var data map[string]interface{}
	if err := json.Unmarshal(w.Body.Bytes(), &data); err != nil {
		t.Fatalf("Failed to decode response JSON: %v", err)
	}

	if data["access_token"] == nil || data["access_token"] == "" {
		t.Errorf("Expected non-empty access_token in response")
	}
	if data["token_type"] != "Bearer" {
		t.Errorf("Expected token_type 'Bearer', got '%v'", data["token_type"])
	}
	if data["user_id"] != "usr_9988_vip" {
		t.Errorf("Expected user_id 'usr_9988_vip', got '%v'", data["user_id"])
	}
	if data["customer_tier"] != "VIP" {
		t.Errorf("Expected customer_tier 'VIP', got '%v'", data["customer_tier"])
	}
}

func TestPublicLoginInvalidCredentialsRejected(t *testing.T) {
	router := setupTestRouter(t)

	body, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "wrongPassword",
	})
	req, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusUnauthorized {
		t.Fatalf("Expected 401 Unauthorized for invalid credentials, got %d. Body: %s", w.Code, w.Body.String())
	}

	var data map[string]interface{}
	if err := json.Unmarshal(w.Body.Bytes(), &data); err != nil {
		t.Fatalf("Failed to decode error JSON: %v", err)
	}

	detail, _ := data["detail"].(string)
	if !strings.Contains(detail, "Invalid username or password. Please try again.") {
		t.Errorf("Expected InvalidCredentials error message in detail, got: '%s'", detail)
	}
}

func TestProtectedRoutesUnauthorizedRejections(t *testing.T) {
	router := setupTestRouter(t)

	// A. Missing Authorization Header
	reqNoAuth, _ := http.NewRequest("GET", "/api/v1.0.0/customer_auth_portal/profile", nil)
	wNoAuth := httptest.NewRecorder()
	router.ServeHTTP(wNoAuth, reqNoAuth)

	if wNoAuth.Code != http.StatusUnauthorized {
		t.Fatalf("Expected 401 for missing token, got %d. Body: %s", wNoAuth.Code, wNoAuth.Body.String())
	}

	var dataNoAuth map[string]interface{}
	_ = json.Unmarshal(wNoAuth.Body.Bytes(), &dataNoAuth)
	if dataNoAuth["error"] != "UserNotAuthenticated" {
		t.Errorf("Expected error 'UserNotAuthenticated', got '%v'", dataNoAuth["error"])
	}
	detailNoAuth, _ := dataNoAuth["detail"].(string)
	if !strings.Contains(detailNoAuth, "Authentication required. Please provide a valid Bearer token") {
		t.Errorf("Expected declarative injected error message, got: '%s'", detailNoAuth)
	}

	// B. Tampered Token Signature
	reqTampered, _ := http.NewRequest("GET", "/api/v1.0.0/customer_auth_portal/profile", nil)
	reqTampered.Header.Set("Authorization", "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjMifQ.badSignature")
	wTampered := httptest.NewRecorder()
	router.ServeHTTP(wTampered, reqTampered)

	if wTampered.Code != http.StatusUnauthorized {
		t.Fatalf("Expected 401 for tampered token, got %d", wTampered.Code)
	}
}

func TestAuthenticatedUserProfileLookup(t *testing.T) {
	router := setupTestRouter(t)

	// 1. Login as VIP
	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	loginReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	loginReq.Header.Set("Content-Type", "application/json")
	loginW := httptest.NewRecorder()
	router.ServeHTTP(loginW, loginReq)

	var loginData map[string]interface{}
	_ = json.Unmarshal(loginW.Body.Bytes(), &loginData)
	token, _ := loginData["access_token"].(string)
	if token == "" {
		t.Fatalf("Failed to retrieve token from login: %s", loginW.Body.String())
	}

	// 2. Fetch Profile with Bearer token
	profileReq, _ := http.NewRequest("GET", "/api/v1.0.0/customer_auth_portal/profile", nil)
	profileReq.Header.Set("Authorization", "Bearer "+token)
	profileW := httptest.NewRecorder()
	router.ServeHTTP(profileW, profileReq)

	if profileW.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK, got %d. Body: %s", profileW.Code, profileW.Body.String())
	}

	var profileData map[string]interface{}
	_ = json.Unmarshal(profileW.Body.Bytes(), &profileData)

	if profileData["user_id"] != "usr_9988_vip" {
		t.Errorf("Expected user_id 'usr_9988_vip', got '%v'", profileData["user_id"])
	}
	if profileData["username"] != "sarah_vip" {
		t.Errorf("Expected username 'sarah_vip', got '%v'", profileData["username"])
	}
	if profileData["email"] != "sarah@acme-corp.com" {
		t.Errorf("Expected email 'sarah@acme-corp.com', got '%v'", profileData["email"])
	}
	if profileData["customer_tier"] != "VIP" {
		t.Errorf("Expected customer_tier 'VIP', got '%v'", profileData["customer_tier"])
	}
	if profileData["account_status"] != "ACTIVE_VIP" {
		t.Errorf("Expected account_status 'ACTIVE_VIP', got '%v'", profileData["account_status"])
	}
	if profileW.Header().Get("X-Correlation-ID") == "" {
		t.Errorf("Expected X-Correlation-ID header in response")
	}
}

func TestVipCartCheckoutPreferentialDiscount(t *testing.T) {
	router := setupTestRouter(t)

	// 1. Login as VIP
	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	loginReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	loginReq.Header.Set("Content-Type", "application/json")
	loginW := httptest.NewRecorder()
	router.ServeHTTP(loginW, loginReq)

	var loginData map[string]interface{}
	_ = json.Unmarshal(loginW.Body.Bytes(), &loginData)
	token, _ := loginData["access_token"].(string)

	// 2. Checkout
	checkoutBody, _ := json.Marshal(map[string]interface{}{
		"cart_id": "cart_vip_001",
		"items": []map[string]interface{}{
			{
				"sku":         "SONY-WH1000XM5",
				"description": "Wireless Headphones",
				"quantity":    2,
				"unit_price":  400.00,
			},
		},
	})
	checkoutReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/cart/checkout", bytes.NewReader(checkoutBody))
	checkoutReq.Header.Set("Authorization", "Bearer "+token)
	checkoutReq.Header.Set("Content-Type", "application/json")
	checkoutW := httptest.NewRecorder()
	router.ServeHTTP(checkoutW, checkoutReq)

	if checkoutW.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK, got %d. Body: %s", checkoutW.Code, checkoutW.Body.String())
	}

	var data map[string]interface{}
	_ = json.Unmarshal(checkoutW.Body.Bytes(), &data)

	if data["customer_tier"] != "VIP" {
		t.Errorf("Expected customer_tier 'VIP', got '%v'", data["customer_tier"])
	}
	if toFloat(data["subtotal"], 0) != 800.00 {
		t.Errorf("Expected subtotal 800.00, got %v", data["subtotal"])
	}
	if toFloat(data["discount_applied"], 0) != 160.00 {
		t.Errorf("Expected discount_applied 160.00 (20%% off 800), got %v", data["discount_applied"])
	}
	if toFloat(data["final_total"], 0) != 640.00 {
		t.Errorf("Expected final_total 640.00, got %v", data["final_total"])
	}
	if data["pricing_tier"] != "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT" {
		t.Errorf("Expected pricing_tier 'VIP_PREFERENTIAL_20_PERCENT_DISCOUNT', got '%v'", data["pricing_tier"])
	}
	if data["shipping_method"] != "FREE_PRIORITY_EXPRESS_AIR" {
		t.Errorf("Expected shipping_method 'FREE_PRIORITY_EXPRESS_AIR', got '%v'", data["shipping_method"])
	}
}

func TestStandardCartCheckoutStandardPricing(t *testing.T) {
	router := setupTestRouter(t)

	// 1. Login as Standard
	loginBody, _ := json.Marshal(map[string]string{
		"username": "john_doe",
		"password": "password123",
	})
	loginReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	loginReq.Header.Set("Content-Type", "application/json")
	loginW := httptest.NewRecorder()
	router.ServeHTTP(loginW, loginReq)

	var loginData map[string]interface{}
	_ = json.Unmarshal(loginW.Body.Bytes(), &loginData)
	token, _ := loginData["access_token"].(string)

	// 2. Checkout
	checkoutBody, _ := json.Marshal(map[string]interface{}{
		"cart_id": "cart_std_002",
		"items": []map[string]interface{}{
			{
				"sku":         "USB-C-CABLE",
				"description": "Braided Cable",
				"quantity":    2,
				"unit_price":  20.00,
			},
		},
	})
	checkoutReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/cart/checkout", bytes.NewReader(checkoutBody))
	checkoutReq.Header.Set("Authorization", "Bearer "+token)
	checkoutReq.Header.Set("Content-Type", "application/json")
	checkoutW := httptest.NewRecorder()
	router.ServeHTTP(checkoutW, checkoutReq)

	if checkoutW.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK, got %d. Body: %s", checkoutW.Code, checkoutW.Body.String())
	}

	var data map[string]interface{}
	_ = json.Unmarshal(checkoutW.Body.Bytes(), &data)

	if data["customer_tier"] != "STANDARD" {
		t.Errorf("Expected customer_tier 'STANDARD', got '%v'", data["customer_tier"])
	}
	if toFloat(data["subtotal"], 0) != 40.00 {
		t.Errorf("Expected subtotal 40.00, got %v", data["subtotal"])
	}
	if toFloat(data["discount_applied"], 0) != 0.00 {
		t.Errorf("Expected discount_applied 0.00, got %v", data["discount_applied"])
	}
	if toFloat(data["final_total"], 0) != 40.00 {
		t.Errorf("Expected final_total 40.00, got %v", data["final_total"])
	}
	if data["pricing_tier"] != "STANDARD_RETAIL_PRICING" {
		t.Errorf("Expected pricing_tier 'STANDARD_RETAIL_PRICING', got '%v'", data["pricing_tier"])
	}
	if data["shipping_method"] != "STANDARD_GROUND_SHIPPING" {
		t.Errorf("Expected shipping_method 'STANDARD_GROUND_SHIPPING', got '%v'", data["shipping_method"])
	}
}

func TestVisualizerDiagramGeneration(t *testing.T) {
	engine, _, err := BuildApp("auth_portal_workflow.yaml")
	if err != nil {
		t.Fatalf("BuildApp failed: %v", err)
	}

	diagram, err := engine.VisualizerService.CompileX6Diagram(context.Background(), "customer_auth_portal", nil, nil, nil)
	if err != nil {
		t.Fatalf("Failed to compile X6 diagram: %v", err)
	}

	nodes, ok := diagram["nodes"].([]map[string]interface{})
	if !ok || len(nodes) == 0 {
		t.Fatalf("Expected non-empty nodes list in X6 diagram")
	}
}

func TestIpWhitelistBlacklistBlocked(t *testing.T) {
	router := setupTestRouter(t)

	// Blocked IP 10.0.0.99 is explicitly in blocked_ips
	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	req, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("X-Forwarded-For", "10.0.0.99")
	req.RemoteAddr = "10.0.0.99:54321"

	w := httptest.NewRecorder()
	router.ServeHTTP(w, req)

	if w.Code != http.StatusForbidden {
		t.Fatalf("Expected 403 Forbidden for blacklisted IP 10.0.0.99, got %d. Body: %s", w.Code, w.Body.String())
	}

	var data map[string]interface{}
	_ = json.Unmarshal(w.Body.Bytes(), &data)
	if data["error"] != "IpAccessDenied" {
		t.Errorf("Expected error 'IpAccessDenied', got '%v'", data["error"])
	}
	detail, _ := data["detail"].(string)
	if !strings.Contains(detail, "Access forbidden: Client IP network is not authorized") {
		t.Errorf("Expected declarative injected IP error message, got: '%s'", detail)
	}
}

func TestRateLimitQuotaHeadersAndRejection(t *testing.T) {
	// Create engine with a strict rate limit of 3 requests per 60s
	engine := octopus_live.NewLiveEngine()
	RegisterAuthPortalActivities(engine)
	_ = engine.LoadBlueprintFile("auth_portal_workflow.yaml")

	strictLimiter := services.NewRateLimitMiddleware(
		3,
		60,
		"client_ip",
		nil,
		[]string{"/health"},
		[]models.MiddlewareExceptionConfig{
			{
				Exception: "RateLimitExceeded",
				Code:      429,
				Message:   "API request quota exceeded. Please slow down your requests or upgrade to VIP tier.",
				Activity:  "LogSecurityAuditViolation",
				IsAsync:   true,
			},
		},
		engine.ActivityExecutor,
	)

	router := gin.Default()
	router.Use(strictLimiter.Handler())
	engine.MountRoutes(router)

	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})

	// Make 3 requests (within quota)
	for i := 0; i < 3; i++ {
		req, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
		req.Header.Set("Content-Type", "application/json")
		req.RemoteAddr = "192.168.1.50:1234"
		w := httptest.NewRecorder()
		router.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("Expected request %d to succeed with 200, got %d", i+1, w.Code)
		}
		if w.Header().Get("X-RateLimit-Limit") == "" {
			t.Errorf("Expected X-RateLimit-Limit header")
		}
	}

	// 4th request must be rate limited with 429 Too Many Requests
	reqExceeded, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	reqExceeded.Header.Set("Content-Type", "application/json")
	reqExceeded.RemoteAddr = "192.168.1.50:1234"
	wExceeded := httptest.NewRecorder()
	router.ServeHTTP(wExceeded, reqExceeded)

	if wExceeded.Code != http.StatusTooManyRequests {
		t.Fatalf("Expected 429 Too Many Requests on 4th call, got %d. Body: %s", wExceeded.Code, wExceeded.Body.String())
	}

	var data map[string]interface{}
	_ = json.Unmarshal(wExceeded.Body.Bytes(), &data)
	if data["error"] != "RateLimitExceeded" {
		t.Errorf("Expected error 'RateLimitExceeded', got '%v'", data["error"])
	}
	detail, _ := data["detail"].(string)
	if !strings.Contains(detail, "API request quota exceeded") {
		t.Errorf("Expected declarative rate limit message in detail, got: '%s'", detail)
	}
	if wExceeded.Header().Get("Retry-After") == "" {
		t.Errorf("Expected Retry-After header on 429 response")
	}
}

func TestRateLimitTierResolutionDynamicUpgrades(t *testing.T) {
	router := setupTestRouter(t)

	// 1. Anonymous request (no token) -> Limit 20
	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	anonReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	anonReq.Header.Set("Content-Type", "application/json")
	anonReq.RemoteAddr = "192.168.1.80:1234"
	anonW := httptest.NewRecorder()
	router.ServeHTTP(anonW, anonReq)

	if anonW.Header().Get("X-RateLimit-Limit") != "20" {
		t.Errorf("Expected ANONYMOUS limit 20, got '%s'", anonW.Header().Get("X-RateLimit-Limit"))
	}

	var loginData map[string]interface{}
	_ = json.Unmarshal(anonW.Body.Bytes(), &loginData)
	vipToken, _ := loginData["access_token"].(string)

	// 2. Standard User Token -> Limit 100
	stdLoginBody, _ := json.Marshal(map[string]string{
		"username": "john_doe",
		"password": "password123",
	})
	stdLoginReq, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(stdLoginBody))
	stdLoginReq.Header.Set("Content-Type", "application/json")
	stdLoginReq.RemoteAddr = "192.168.1.81:1234"
	stdLoginW := httptest.NewRecorder()
	router.ServeHTTP(stdLoginW, stdLoginReq)

	var stdLoginData map[string]interface{}
	_ = json.Unmarshal(stdLoginW.Body.Bytes(), &stdLoginData)
	stdToken, _ := stdLoginData["access_token"].(string)

	stdReq, _ := http.NewRequest("GET", "/api/v1.0.0/customer_auth_portal/profile", nil)
	stdReq.Header.Set("Authorization", "Bearer "+stdToken)
	stdReq.RemoteAddr = "192.168.1.81:1234"
	stdW := httptest.NewRecorder()
	router.ServeHTTP(stdW, stdReq)

	if stdW.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK for standard profile, got %d", stdW.Code)
	}
	if stdW.Header().Get("X-RateLimit-Limit") != "100" {
		t.Errorf("Expected STANDARD limit 100, got '%s'", stdW.Header().Get("X-RateLimit-Limit"))
	}

	// 3. VIP User Token -> Limit 1000
	vipReq, _ := http.NewRequest("GET", "/api/v1.0.0/customer_auth_portal/profile", nil)
	vipReq.Header.Set("Authorization", "Bearer "+vipToken)
	vipReq.RemoteAddr = "192.168.1.80:1234"
	vipW := httptest.NewRecorder()
	router.ServeHTTP(vipW, vipReq)

	if vipW.Code != http.StatusOK {
		t.Fatalf("Expected 200 OK for VIP profile, got %d", vipW.Code)
	}
	if vipW.Header().Get("X-RateLimit-Limit") != "1000" {
		t.Errorf("Expected VIP limit 1000, got '%s'", vipW.Header().Get("X-RateLimit-Limit"))
	}
}

func TestMultilangTaggedExecution(t *testing.T) {
	engine, router, err := BuildApp("auth_portal_workflow.yaml")
	if err != nil {
		t.Fatalf("BuildApp failed: %v", err)
	}

	t.Log("🐹 ==================== [GO ENGINE TESTS] ====================")

	// 1. Tagged Login Success
	jobLoginOk := "go-login-success-1789774500000"
	loginBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "password123",
	})
	reqLogin, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(loginBody))
	reqLogin.Header.Set("Content-Type", "application/json")
	reqLogin.Header.Set("X-Job-ID", jobLoginOk)
	wLogin := httptest.NewRecorder()
	router.ServeHTTP(wLogin, reqLogin)

	if wLogin.Code != http.StatusOK {
		t.Fatalf("[Go] Login failed: %d %s", wLogin.Code, wLogin.Body.String())
	}
	var loginData map[string]interface{}
	_ = json.Unmarshal(wLogin.Body.Bytes(), &loginData)
	token, _ := loginData["access_token"].(string)
	t.Logf("[Go] 1. Login Successful (Job ID: %s, User: %v, Tier: %v)", jobLoginOk, loginData["user_id"], loginData["customer_tier"])

	// 2. Tagged VIP Checkout
	jobCheckout := "go-checkout-vip-1789774500001"
	checkoutBody, _ := json.Marshal(map[string]interface{}{
		"cart_id": "cart_go_001",
		"items": []map[string]interface{}{
			{
				"sku":         "SONY-WH1000XM5",
				"description": "VIP Audio",
				"quantity":    2,
				"unit_price":  400.00,
			},
		},
	})
	reqCheckout, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/cart/checkout", bytes.NewReader(checkoutBody))
	reqCheckout.Header.Set("Content-Type", "application/json")
	reqCheckout.Header.Set("Authorization", "Bearer "+token)
	reqCheckout.Header.Set("X-Job-ID", jobCheckout)
	wCheckout := httptest.NewRecorder()
	router.ServeHTTP(wCheckout, reqCheckout)

	if wCheckout.Code != http.StatusOK {
		t.Fatalf("[Go] VIP Checkout failed: %d %s", wCheckout.Code, wCheckout.Body.String())
	}
	var checkoutData map[string]interface{}
	_ = json.Unmarshal(wCheckout.Body.Bytes(), &checkoutData)
	t.Logf("[Go] 2. VIP Checkout (Job ID: %s, Subtotal: $%v, Final: $%v, Tier: %v)", jobCheckout, checkoutData["subtotal"], checkoutData["final_total"], checkoutData["pricing_tier"])

	// 3. Tagged Login Failure
	jobLoginFail := "go-login-fail-1789774500002"
	failBody, _ := json.Marshal(map[string]string{
		"username": "sarah_vip",
		"password": "WRONG_PASSWORD_XYZ",
	})
	reqFail, _ := http.NewRequest("POST", "/api/v1.0.0/customer_auth_portal/auth/login", bytes.NewReader(failBody))
	reqFail.Header.Set("Content-Type", "application/json")
	reqFail.Header.Set("X-Job-ID", jobLoginFail)
	wFail := httptest.NewRecorder()
	router.ServeHTTP(wFail, reqFail)

	if wFail.Code != http.StatusUnauthorized {
		t.Fatalf("[Go] Expected 401 for login failure, got %d: %s", wFail.Code, wFail.Body.String())
	}
	t.Logf("[Go] 3. Login Failure Expected 401 (Job ID: %s, Body: %s)", jobLoginFail, wFail.Body.String())

	// Inspect execution traces in Go StateStore
	st, err := engine.PipelineManager.GetExecutionState(context.Background(), jobLoginFail)
	if err != nil || st == nil {
		t.Fatalf("[Go] Failed to retrieve state for %s: %v", jobLoginFail, err)
	}

	t.Logf("[Go] Execution Traces for %s (%d traces):", jobLoginFail, len(st.Traces))
	for _, tr := range st.Traces {
		t.Logf("   • Step: %s | Activity: %s | Status: %s | Output: %v", tr.StepID, tr.ActivityName, tr.Status, tr.OutputPayload)
	}

	if len(st.Traces) < 2 {
		t.Fatalf("[Go] Expected at least 2 traces (VerifyUserCredentials + LogSecurityAuditViolation), got %d", len(st.Traces))
	}
	t.Log("✅ [GO ENGINE] All tests PASSED successfully!")
}



