package main

import (
	"context"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/services"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/octopus_live"
)

const JwtSecretKey = "octopus-live-engine-secret-key-2026"

// ============================================================================
// 1. Domain Models
// ============================================================================

type LoginRequest struct {
	Username string `json:"username"`
	Password string `json:"password"`
}

type LoginResponse struct {
	AccessToken      string `json:"access_token"`
	TokenType        string `json:"token_type"`
	UserID           string `json:"user_id"`
	CustomerTier     string `json:"customer_tier"`
	ExpiresInSeconds int    `json:"expires_in_seconds"`
}

type UserProfileResponse struct {
	UserID        string   `json:"user_id"`
	Username      string   `json:"username"`
	Email         string   `json:"email"`
	CustomerTier  string   `json:"customer_tier"`
	AccountStatus string   `json:"account_status"`
	MemberSince   string   `json:"member_since"`
	Roles         []string `json:"roles"`
}

type CartItem struct {
	SKU         string  `json:"sku"`
	Description string  `json:"description"`
	Quantity    int     `json:"quantity"`
	UnitPrice   float64 `json:"unit_price"`
}

type CheckoutRequest struct {
	CartID string     `json:"cart_id"`
	Items  []CartItem `json:"items"`
}

type CheckoutResponse struct {
	OrderID           string  `json:"order_id"`
	CustomerID        string  `json:"customer_id"`
	CustomerTier      string  `json:"customer_tier"`
	Subtotal          float64 `json:"subtotal"`
	DiscountApplied   float64 `json:"discount_applied"`
	FinalTotal        float64 `json:"final_total"`
	PricingTier       string  `json:"pricing_tier"`
	ShippingMethod    string  `json:"shipping_method"`
	FastPathLatencyMs float64 `json:"fast_path_latency_ms"`
}

// ============================================================================
// 2. Pure Standard Library JWT Cryptographic Helpers
// ============================================================================

func CreateJwtToken(claims map[string]interface{}, secret string, expiresInSeconds int) (string, error) {
	header := map[string]string{"alg": "HS256", "typ": "JWT"}
	headerJSON, err := json.Marshal(header)
	if err != nil {
		return "", err
	}

	claimsCopy := make(map[string]interface{})
	for k, v := range claims {
		claimsCopy[k] = v
	}
	now := time.Now().Unix()
	claimsCopy["exp"] = now + int64(expiresInSeconds)
	claimsCopy["iat"] = now

	claimsJSON, err := json.Marshal(claimsCopy)
	if err != nil {
		return "", err
	}

	hB64 := base64.RawURLEncoding.EncodeToString(headerJSON)
	pB64 := base64.RawURLEncoding.EncodeToString(claimsJSON)
	signingInput := fmt.Sprintf("%s.%s", hB64, pB64)

	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(signingInput))
	sig := mac.Sum(nil)
	sB64 := base64.RawURLEncoding.EncodeToString(sig)

	return fmt.Sprintf("%s.%s", signingInput, sB64), nil
}

func DecodeAndVerifyJwt(token string, secret string) (map[string]interface{}, error) {
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		return nil, fmt.Errorf("malformed JWT structure (must contain 3 dot-separated parts)")
	}

	hB64, pB64, sB64 := parts[0], parts[1], parts[2]
	signingInput := fmt.Sprintf("%s.%s", hB64, pB64)

	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(signingInput))
	expectedSig := mac.Sum(nil)

	actualSig, err := base64.RawURLEncoding.DecodeString(sB64)
	if err != nil {
		return nil, fmt.Errorf("invalid base64 signature encoding: %w", err)
	}

	if !hmac.Equal(expectedSig, actualSig) {
		return nil, fmt.Errorf("invalid JWT cryptographic signature")
	}

	claimsBytes, err := base64.RawURLEncoding.DecodeString(pB64)
	if err != nil {
		return nil, fmt.Errorf("invalid base64 payload encoding: %w", err)
	}

	var claims map[string]interface{}
	if err := json.Unmarshal(claimsBytes, &claims); err != nil {
		return nil, fmt.Errorf("invalid JSON payload: %w", err)
	}

	if expVal, ok := claims["exp"]; ok {
		var exp int64
		switch v := expVal.(type) {
		case float64:
			exp = int64(v)
		case int64:
			exp = v
		case int:
			exp = int64(v)
		}
		if exp > 0 && exp < time.Now().Unix() {
			return nil, fmt.Errorf("JWT token has expired")
		}
	}

	return claims, nil
}

// ============================================================================
// 3. In-Memory User Database
// ============================================================================

type UserRecord struct {
	UserID        string
	Username      string
	Email         string
	Password      string
	CustomerTier  string
	Roles         []string
	AccountStatus string
	MemberSince   string
}

var UserDatabase = map[string]UserRecord{
	"sarah_vip": {
		UserID:        "usr_9988_vip",
		Username:      "sarah_vip",
		Email:         "sarah@acme-corp.com",
		Password:      "password123",
		CustomerTier:  "VIP",
		Roles:         []string{"customer", "vip_member"},
		AccountStatus: "ACTIVE_VIP",
		MemberSince:   "2024-01-15",
	},
	"john_doe": {
		UserID:        "usr_1024_std",
		Username:      "john_doe",
		Email:         "john.doe@example.com",
		Password:      "password123",
		CustomerTier:  "STANDARD",
		Roles:         []string{"customer"},
		AccountStatus: "ACTIVE_STANDARD",
		MemberSince:   "2025-06-20",
	},
}

// ============================================================================
// 4. Domain Activities
// ============================================================================

func RegisterAuthPortalActivities(engine *octopus_live.OctopusLiveEngine) {
	// 1. VerifyUserCredentials
	engine.RegisterActivity("VerifyUserCredentials", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		username, _ := input["username"].(string)
		password, _ := input["password"].(string)

		if username == "crash_user" {
			return nil, fmt.Errorf("division by zero during unexpected calculation crash")
		}

		user, exists := UserDatabase[username]
		if !exists || user.Password != password {
			return nil, services.NewInvalidCredentials("Invalid username or password. Please try again.")
		}

		return map[string]interface{}{
			"authenticated":     true,
			"user_id":           user.UserID,
			"verified_username": user.Username,
			"email":             user.Email,
			"customer_tier":     user.CustomerTier,
			"roles":             user.Roles,
		}, nil
	})

	// 2. IssueBearerToken
	engine.RegisterActivity("IssueBearerToken", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		userID, _ := input["user_id"].(string)
		tier, _ := input["customer_tier"].(string)
		if tier == "" {
			tier = "STANDARD"
		}
		username, _ := input["verified_username"].(string)
		email, _ := input["email"].(string)
		roles := []string{"customer"}
		if r, ok := input["roles"].([]string); ok {
			roles = r
		} else if r, ok := input["roles"].([]interface{}); ok {
			var strRoles []string
			for _, item := range r {
				if s, ok := item.(string); ok {
					strRoles = append(strRoles, s)
				}
			}
			roles = strRoles
		}

		claims := map[string]interface{}{
			"sub":           userID,
			"user_id":       userID,
			"username":      username,
			"email":         email,
			"customer_tier": tier,
			"roles":         roles,
		}

		token, err := CreateJwtToken(claims, JwtSecretKey, 3600)
		if err != nil {
			return nil, fmt.Errorf("failed to create JWT token: %w", err)
		}

		return map[string]interface{}{
			"access_token":       token,
			"token_type":         "Bearer",
			"user_id":            userID,
			"customer_tier":      tier,
			"expires_in_seconds": 3600,
		}, nil
	})

	// 3. FetchUserProfile
	engine.RegisterActivity("FetchUserProfile", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		userID, _ := input["user_id"].(string)
		var user *UserRecord
		for _, u := range UserDatabase {
			if u.UserID == userID {
				userCopy := u
				user = &userCopy
				break
			}
		}

		if user == nil {
			roles := []string{"customer"}
			if r, ok := input["roles"].([]string); ok {
				roles = r
			}
			username, _ := input["username"].(string)
			if username == "" {
				username = "guest"
			}
			email, _ := input["email"].(string)
			if email == "" {
				email = "guest@example.com"
			}
			tier, _ := input["customer_tier"].(string)
			if tier == "" {
				tier = "STANDARD"
			}

			return map[string]interface{}{
				"user_id":        userID,
				"username":       username,
				"email":          email,
				"customer_tier":  tier,
				"account_status": "ACTIVE",
				"member_since":   "2026-01-01",
				"roles":          roles,
			}, nil
		}

		return map[string]interface{}{
			"user_id":        user.UserID,
			"username":       user.Username,
			"email":          user.Email,
			"customer_tier":  user.CustomerTier,
			"account_status": user.AccountStatus,
			"member_since":   user.MemberSince,
			"roles":          user.Roles,
		}, nil
	})

	// 4. ValidateCartItems
	engine.RegisterActivity("ValidateCartItems", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		subtotal := 0.0
		var items []interface{}
		if rawItems, ok := input["items"].([]interface{}); ok {
			items = rawItems
			for _, raw := range rawItems {
				if itemMap, ok := raw.(map[string]interface{}); ok {
					qty := toFloat(itemMap["quantity"], 1.0)
					price := toFloat(itemMap["unit_price"], 0.0)
					subtotal += qty * price
				}
			}
		}

		if subtotal == 0.0 {
			subtotal = 300.00
			items = []interface{}{
				map[string]interface{}{
					"sku":         "HEADPHONES-PRO",
					"description": "Noise Cancelling",
					"quantity":    1,
					"unit_price":  300.00,
				},
			}
		}

		cartID, _ := input["cart_id"].(string)
		if cartID == "" {
			cartID = "cart_default"
		}

		return map[string]interface{}{
			"subtotal":        roundFloat(subtotal, 2),
			"validated_items": items,
			"cart_id":         cartID,
		}, nil
	})

	// 5. ApplyVIPDiscount
	engine.RegisterActivity("ApplyVIPDiscount", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		subtotal := toFloat(input["subtotal"], 0.0)
		discount := roundFloat(subtotal*0.20, 2)
		finalTotal := roundFloat(subtotal-discount, 2)

		return map[string]interface{}{
			"discount_applied": discount,
			"final_total":      finalTotal,
			"pricing_tier":     "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
			"shipping_method":  "FREE_PRIORITY_EXPRESS_AIR",
		}, nil
	})

	// 6. ApplyStandardPricing
	engine.RegisterActivity("ApplyStandardPricing", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		subtotal := toFloat(input["subtotal"], 0.0)
		return map[string]interface{}{
			"discount_applied": 0.0,
			"final_total":      subtotal,
			"pricing_tier":     "STANDARD_RETAIL_PRICING",
			"shipping_method":  "STANDARD_GROUND_SHIPPING",
		}, nil
	})

	// 7. FinalizeCheckoutOrder
	engine.RegisterActivity("FinalizeCheckoutOrder", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		subtotal := toFloat(input["subtotal"], 0.0)
		discount := toFloat(input["discount_applied"], 0.0)
		finalTotal := toFloat(input["final_total"], subtotal)
		customerID, _ := input["customer_id"].(string)
		if customerID == "" {
			customerID, _ = input["user_id"].(string)
		}
		if customerID == "" {
			customerID = "usr_anon"
		}
		tier, _ := input["customer_tier"].(string)
		if tier == "" {
			tier = "STANDARD"
		}
		pricingTier, _ := input["pricing_tier"].(string)
		shippingMethod, _ := input["shipping_method"].(string)

		return map[string]interface{}{
			"order_id":             fmt.Sprintf("ord_%d", time.Now().UnixNano()/1e6),
			"customer_id":          customerID,
			"customer_tier":        tier,
			"subtotal":             subtotal,
			"discount_applied":     discount,
			"final_total":          finalTotal,
			"pricing_tier":         pricingTier,
			"shipping_method":      shippingMethod,
			"fast_path_latency_ms": 0.35,
		}, nil
	})

	// 8. LogSecurityAuditViolation
	engine.RegisterActivity("LogSecurityAuditViolation", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		exception, _ := input["exception"].(string)
		path, _ := input["path"].(string)
		method, _ := input["method"].(string)
		clientHost, _ := input["client_host"].(string)
		log.Printf("🚨 [SECURITY AUDIT] Unauthorized access attempt blocked: %s %s from %s (%s)", method, path, clientHost, exception)

		return map[string]interface{}{
			"audit_logged":         true,
			"security_incident_id": fmt.Sprintf("inc_%d", time.Now().UnixNano()/1e6),
			"timestamp":            time.Now().UTC().Format(time.RFC3339),
		}, nil
	})

	// 9. RecordResponseAuditLog
	engine.RegisterActivity("RecordResponseAuditLog", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		route, _ := input["route"].(string)
		method, _ := input["method"].(string)
		statusCode := input["status_code"]
		durationMs := input["duration_ms"]
		log.Printf("📊 [AUDIT LOG] Recorded: %s %s -> Status %v in %vms", method, route, statusCode, durationMs)

		return map[string]interface{}{
			"logged":    true,
			"timestamp": time.Now().UTC().Format(time.RFC3339),
		}, nil
	})

	// 10. HandleGlobalUnhandledException
	engine.RegisterActivity("HandleGlobalUnhandledException", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
		jobID, _ := input["job_id"].(string)
		stepID, _ := input["step_id"].(string)
		failingStep, _ := input["failing_step_id"].(string)
		if failingStep == "" {
			failingStep = stepID
		}
		errType, _ := input["error_type"].(string)
		errMsg, _ := input["error_message"].(string)
		if errMsg == "" {
			errMsg, _ = input["message"].(string)
		}
		log.Printf("🚨 [GLOBAL EXCEPTION HANDLER] Incident logged for Job=%s Step=%s: %s - %s", jobID, failingStep, errType, errMsg)

		return map[string]interface{}{
			"incident_id": fmt.Sprintf("inc_%d", time.Now().UnixNano()/1e6),
			"handled":     true,
			"status":      "INCIDENT_LOGGED",
			"error_type":  errType,
			"message":     "An unexpected internal server error occurred. Incident reference logged.",
			"timestamp":   time.Now().UTC().Format(time.RFC3339),
		}, nil
	})
}

func toFloat(val interface{}, def float64) float64 {
	if val == nil {
		return def
	}
	switch v := val.(type) {
	case float64:
		return v
	case float32:
		return float64(v)
	case int:
		return float64(v)
	case int64:
		return float64(v)
	default:
		return def
	}
}

func roundFloat(val float64, precision int) float64 {
	ratio := 1.0
	for i := 0; i < precision; i++ {
		ratio *= 10.0
	}
	return float64(int(val*ratio+0.5)) / ratio
}

// ============================================================================
// 5. Build Engine Application
// ============================================================================

func BuildApp(yamlPath string) (*octopus_live.OctopusLiveEngine, *gin.Engine, error) {
	engine := octopus_live.NewLiveEngine()

	// Register Models
	engine.RegisterModel("LoginRequest", LoginRequest{})
	engine.RegisterModel("LoginResponse", LoginResponse{})
	engine.RegisterModel("UserProfileResponse", UserProfileResponse{})
	engine.RegisterModel("CheckoutRequest", CheckoutRequest{})
	engine.RegisterModel("CheckoutResponse", CheckoutResponse{})

	// Register Activities
	RegisterAuthPortalActivities(engine)

	// Load blueprint
	if err := engine.LoadBlueprintFile(yamlPath); err != nil {
		return nil, nil, fmt.Errorf("failed to load blueprint: %w", err)
	}

	// Register custom factory for BearerAuthAndClaimsMiddleware with pure JWT decoder
	engine.RegisterCustomMiddleware("BearerAuthAndClaimsMiddleware", func(cfg models.MiddlewareConfig, executor ports.IActivityExecutor, dispatcher ...ports.IAsyncActivityDispatcher) gin.HandlerFunc {
		return services.NewBearerAuthAndClaimsMiddleware(
			func(token string) (map[string]interface{}, error) {
				return DecodeAndVerifyJwt(token, JwtSecretKey)
			},
			func(claims map[string]interface{}) map[string]interface{} {
				userID := ""
				if u, ok := claims["user_id"].(string); ok && u != "" {
					userID = u
				} else if s, ok := claims["sub"].(string); ok {
					userID = s
				}
				tier := "STANDARD"
				if t, ok := claims["customer_tier"].(string); ok && t != "" {
					tier = t
				}
				return map[string]interface{}{
					"user_id":       userID,
					"customer_id":   userID,
					"customer_tier": tier,
					"roles":         claims["roles"],
					"email":         claims["email"],
				}
			},
			cfg.Excludes,
			cfg.Exceptions,
			executor,
			dispatcher...,
		).Handler()
	})

	router := gin.Default()
	engine.MountRoutes(router)

	return engine, router, nil
}

func main() {
	portStr := os.Getenv("PORT")
	port := flag.Int("port", 8080, "Port to listen on")
	yamlFile := flag.String("yaml", "auth_portal_workflow.yaml", "Path to workflow YAML")
	workerOnly := flag.Bool("worker-only", false, "Run standalone Temporal worker without HTTP server")
	workerFlag := flag.Bool("worker", false, "Run standalone Temporal worker without HTTP server")
	flag.Parse()

	if os.Getenv("WORKER_ONLY") == "true" || *workerFlag || *workerOnly {
		engine, _, err := BuildApp(*yamlFile)
		if err != nil {
			log.Fatalf("Failed to initialize engine for worker: %v", err)
		}
		if err := engine.StartWorker(); err != nil {
			log.Fatalf("Worker failure: %v", err)
		}
		return
	}

	if portStr != "" {
		if p, err := strconv.Atoi(portStr); err == nil {
			*port = p
		}
	}

	engine, router, err := BuildApp(*yamlFile)
	if err != nil {
		log.Fatalf("Failed to initialize Customer Auth Portal: %v", err)
	}

	_ = engine.StartEmbeddedWorker()
	defer engine.StopEmbeddedWorker()

	addr := fmt.Sprintf(":%d", *port)
	log.Printf("🚀 Customer Auth Portal API (Go) listening on http://0.0.0.0%s", addr)
	if err := router.Run(addr); err != nil && err != http.ErrServerClosed {
		log.Fatalf("Server exited: %v", err)
	}
}
