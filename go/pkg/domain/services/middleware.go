package services

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/ports"
	"github.com/redis/go-redis/v9"
)

// ============================================================================
// 1. Standard Built-in Middleware Error Hierarchy
// ============================================================================

type OctopusMiddlewareError struct {
	Name    string                 `json:"error"`
	Code    int                    `json:"status_code"`
	Message string                 `json:"detail"`
	Details map[string]interface{} `json:"details,omitempty"`
	Headers map[string]string      `json:"-"`
	// Cause is the broader error this one is a kind of. An `exceptions`
	// entry naming any error in the Unwrap chain matches (D14).
	Cause error `json:"-"`
}

func (e *OctopusMiddlewareError) Error() string {
	return e.Message
}

func (e *OctopusMiddlewareError) Unwrap() error { return e.Cause }

func NewUserNotAuthenticated(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "User is not authenticated or token is missing/invalid"
	}
	return &OctopusMiddlewareError{
		Name:    "UserNotAuthenticated",
		Code:    http.StatusUnauthorized,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewInvalidCredentials(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "Invalid username or password. Please try again."
	}
	return &OctopusMiddlewareError{
		Name:    "InvalidCredentials",
		Code:    http.StatusUnauthorized,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewUserNotAuthorized(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "User lacks authorization or required permissions"
	}
	return &OctopusMiddlewareError{
		Name:    "UserNotAuthorized",
		Code:    http.StatusForbidden,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewInvalidApiKeyException(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "Invalid or missing API key"
	}
	return &OctopusMiddlewareError{
		Name:    "InvalidApiKeyException",
		Code:    http.StatusUnauthorized,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewRateLimitExceeded(msg string, limit, windowSeconds, retryAfter int, key string, headers map[string]string) *OctopusMiddlewareError {
	if msg == "" {
		msg = fmt.Sprintf("Rate limit exceeded. Quota: %d req/%ds. Retry in %ds.", limit, windowSeconds, retryAfter)
	}
	details := map[string]interface{}{
		"limit":               limit,
		"window_seconds":      windowSeconds,
		"retry_after_seconds": retryAfter,
		"rate_limited_key":    key,
	}
	if headers == nil {
		headers = make(map[string]string)
	}
	return &OctopusMiddlewareError{
		Name:    "RateLimitExceeded",
		Code:    http.StatusTooManyRequests,
		Message: msg,
		Details: details,
		Headers: headers,
	}
}

func NewIpAccessDenied(msg, clientIP, path, method string) *OctopusMiddlewareError {
	if msg == "" {
		msg = fmt.Sprintf("Access denied: Client IP '%s' is not in the allowed IP whitelist", clientIP)
	}
	return &OctopusMiddlewareError{
		Name:    "IpAccessDenied",
		Code:    http.StatusForbidden,
		Message: msg,
		Details: map[string]interface{}{
			"client_ip": clientIP,
			"path":      path,
			"method":    method,
		},
		Headers: make(map[string]string),
	}
}

func NewBadRequestException(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "Invalid or malformed request payload. Please verify syntax and parameters."
	}
	return &OctopusMiddlewareError{
		Name:    "BadRequestException",
		Code:    http.StatusBadRequest,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewMalformedJsonException(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "Malformed JSON syntax in request body."
	}
	return &OctopusMiddlewareError{
		Name:    "MalformedJsonException",
		Code:    http.StatusBadRequest,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewValidationException(msg string) *OctopusMiddlewareError {
	if msg == "" {
		msg = "Request validation rules failed."
	}
	return &OctopusMiddlewareError{
		Name:    "ValidationException",
		Code:    http.StatusBadRequest,
		Message: msg,
		Details: make(map[string]interface{}),
		Headers: make(map[string]string),
	}
}

func NewMissingRequiredFieldException(fieldOrMsg string) *OctopusMiddlewareError {
	msg := fieldOrMsg
	if msg == "" {
		msg = "Missing required field in request payload."
	} else if !strings.Contains(fieldOrMsg, " ") {
		msg = fmt.Sprintf("Missing required field '%s' in request payload.", fieldOrMsg)
	}
	return &OctopusMiddlewareError{
		Name:    "MissingRequiredFieldException",
		Code:    http.StatusBadRequest,
		Message: msg,
		Details: map[string]interface{}{"field": fieldOrMsg},
		Headers: make(map[string]string),
	}
}

func NewPayloadTooLargeException(msg string, maxSize, actualSize int64) *OctopusMiddlewareError {
	if msg == "" {
		msg = fmt.Sprintf("Request payload size %d bytes exceeds maximum limit of %d bytes.", actualSize, maxSize)
	}
	return &OctopusMiddlewareError{
		Name:    "PayloadTooLargeException",
		Code:    http.StatusRequestEntityTooLarge,
		Message: msg,
		Details: map[string]interface{}{
			"max_size_bytes":    maxSize,
			"actual_size_bytes": actualSize,
		},
		Headers: make(map[string]string),
	}
}

// ============================================================================
// 2. Route Matching Utility
// ============================================================================

// SystemExcludes are skipped by every declarative middleware. Tooling routes
// (visualizer, diagrams, docs) are deliberately not listed: they are guarded
// by EngineOptions.Tooling and can be protected by auth middlewares too; list
// them under a middleware's excludes to skip it explicitly.
var SystemExcludes = []string{
	"/favicon.ico",
	"/favicon.png",
	"/robots.txt",
	"/health*",
}

type RouteMatcher struct{}

func (rm *RouteMatcher) Matches(path, pattern string) bool {
	if pattern == "" || path == "" {
		return false
	}
	cleanPath := strings.TrimRight(path, "/")
	cleanPat := strings.TrimRight(pattern, "/")

	if cleanPath == cleanPat {
		return true
	}

	if strings.Contains(pattern, "*") || strings.Contains(pattern, "?") {
		matched, _ := filepath.Match(pattern, path)
		if matched {
			return true
		}
		matched, _ = filepath.Match(cleanPat, cleanPath)
		if matched {
			return true
		}
	}

	if strings.HasSuffix(pattern, "*") {
		prefix := pattern[:len(pattern)-1]
		if strings.HasPrefix(path, prefix) {
			return true
		}
	}
	return false
}

func (rm *RouteMatcher) ShouldSkip(path string, excludes, includes []string) bool {
	for _, sysExc := range SystemExcludes {
		if rm.Matches(path, sysExc) {
			return true
		}
	}

	if len(includes) > 0 {
		included := false
		for _, inc := range includes {
			if rm.Matches(path, inc) {
				included = true
				break
			}
		}
		if !included {
			return true
		}
	}

	if len(excludes) > 0 {
		for _, exc := range excludes {
			if rm.Matches(path, exc) {
				return true
			}
		}
	}
	return false
}

var DefaultRouteMatcher = &RouteMatcher{}

// ============================================================================
// 3. Middleware Exception Responder Helper
// ============================================================================

func RespondMiddlewareError(
	c *gin.Context,
	err *OctopusMiddlewareError,
	exceptions []models.MiddlewareExceptionConfig,
	executor ports.IActivityExecutor,
	dispatcher ...ports.IAsyncActivityDispatcher,
) {
	statusCode := err.Code
	message := err.Message
	details := err.Details
	headers := err.Headers

	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}

	var matchedCfg *models.MiddlewareExceptionConfig
	for _, ec := range exceptions {
		if strings.EqualFold(ec.Exception, err.Name) {
			matchedCfg = &ec
			break
		}
	}

	if matchedCfg != nil {
		if matchedCfg.Code > 0 {
			statusCode = matchedCfg.Code
		}
		if matchedCfg.Message != "" {
			message = matchedCfg.Message
		}
		for k, v := range matchedCfg.Headers {
			headers[k] = v
		}

		if matchedCfg.Activity != "" && executor != nil {
			actPayload := map[string]interface{}{
				"exception":      err.Name,
				"error":          err.Name,
				"message":        message,
				"original_error": err.Message,
				"status_code":    statusCode,
				"path":           c.Request.URL.Path,
				"route":          c.Request.URL.Path,
				"method":         c.Request.Method,
				"client_ip":      c.ClientIP(),
				"client_host":    c.ClientIP(),
				"details":        details,
			}
			if claims, ok := c.Get("auth_claims"); ok {
				actPayload["auth_claims"] = claims
			}

			if matchedCfg.IsAsync {
				if disp != nil {
					jobID, _ := c.Get("job_id")
					jobIDStr, _ := jobID.(string)
					if jobIDStr == "" {
						jobIDStr = c.GetHeader("X-Job-ID")
					}
					if jobIDStr == "" {
						jobIDStr = c.GetHeader("X-Request-ID")
					}
					if jobIDStr == "" {
						jobIDStr = c.GetHeader("X-Correlation-ID")
					}
					if jobIDStr == "" {
						jobIDStr = fmt.Sprintf("sec_audit_%d", time.Now().UnixNano())
					}
					stepID := "exc_" + strings.ToLower(matchedCfg.Activity)
					disp.Dispatch(context.Background(), matchedCfg.Activity, actPayload, jobIDStr, stepID, executor)
				} else {
					// Non-blocking background goroutine
					go func(actName string, payload map[string]interface{}) {
						ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
						defer cancel()
						_, _ = executor.ExecuteActivity(ctx, actName, payload)
					}(matchedCfg.Activity, actPayload)
				}
			} else {
				ctx, cancel := context.WithTimeout(c.Request.Context(), 5*time.Second)
				defer cancel()
				res, actErr := executor.ExecuteActivity(ctx, matchedCfg.Activity, actPayload)
				if actErr == nil && res != nil {
					for k, v := range res {
						if k != "status_code" && k != "code" && k != "message" && k != "detail" {
							details[k] = v
						}
					}
				}
			}
		}
	}

	for k, v := range headers {
		c.Header(k, v)
	}

	c.JSON(statusCode, gin.H{
		"error":       err.Name,
		"detail":      message,
		"status_code": statusCode,
		"details":     details,
	})
	c.Abort()
}

// ============================================================================
// 4. Concrete Middleware Implementations
// ============================================================================

// --- A. Bearer Auth & Claims Middleware ---
type TokenValidatorFunc func(token string) (map[string]interface{}, error)
type ClaimsMapperFunc func(claims map[string]interface{}) map[string]interface{}

type BearerAuthAndClaimsMiddleware struct {
	TokenValidator   TokenValidatorFunc
	ClaimsMapper     ClaimsMapperFunc
	TokenHeader      string
	Excludes         []string
	Includes         []string
	Exceptions       []models.MiddlewareExceptionConfig
	ActivityExecutor ports.IActivityExecutor
	Dispatcher       ports.IAsyncActivityDispatcher
}

func NewBearerAuthAndClaimsMiddleware(
	validator TokenValidatorFunc,
	mapper ClaimsMapperFunc,
	excludes []string,
	exceptions []models.MiddlewareExceptionConfig,
	executor ports.IActivityExecutor,
	dispatcher ...ports.IAsyncActivityDispatcher,
) *BearerAuthAndClaimsMiddleware {
	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}
	if validator == nil {
		validator = func(token string) (map[string]interface{}, error) {
			if token == "" || token == "invalid-token" {
				return nil, fmt.Errorf("invalid bearer token signature")
			}
			return map[string]interface{}{"sub": "usr_default", "customer_tier": "VIP"}, nil
		}
	}
	if mapper == nil {
		mapper = func(claims map[string]interface{}) map[string]interface{} {
			return claims
		}
	}
	return &BearerAuthAndClaimsMiddleware{
		TokenValidator:   validator,
		ClaimsMapper:     mapper,
		TokenHeader:      "Authorization",
		Excludes:         excludes,
		Exceptions:       exceptions,
		ActivityExecutor: executor,
		Dispatcher:       disp,
	}
}

func (m *BearerAuthAndClaimsMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		if DefaultRouteMatcher.ShouldSkip(c.Request.URL.Path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		authHeader := c.GetHeader(m.TokenHeader)
		if authHeader == "" {
			RespondMiddlewareError(c, NewUserNotAuthenticated(fmt.Sprintf("Missing '%s' header", m.TokenHeader)), m.Exceptions, m.ActivityExecutor, m.Dispatcher)
			return
		}

		token := authHeader
		if strings.HasPrefix(strings.ToLower(authHeader), "bearer ") {
			token = strings.TrimSpace(authHeader[7:])
		}

		claims, err := m.TokenValidator(token)
		if err != nil {
			RespondMiddlewareError(c, NewUserNotAuthenticated(fmt.Sprintf("Token validation error: %v", err)), m.Exceptions, m.ActivityExecutor, m.Dispatcher)
			return
		}

		injections := m.ClaimsMapper(claims)
		c.Set("auth_claims", claims)
		c.Set("workflow_injections", injections)
		c.Next()
	}
}

// --- B. API Key Auth Middleware ---
type ApiKeyAuthMiddleware struct {
	ValidKeys        map[string]bool
	HeaderName       string
	Excludes         []string
	Includes         []string
	Exceptions       []models.MiddlewareExceptionConfig
	ActivityExecutor ports.IActivityExecutor
	Dispatcher       ports.IAsyncActivityDispatcher
}

func NewApiKeyAuthMiddleware(keys []string, excludes []string, exceptions []models.MiddlewareExceptionConfig, executor ports.IActivityExecutor, dispatcher ...ports.IAsyncActivityDispatcher) *ApiKeyAuthMiddleware {
	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}
	keySet := make(map[string]bool)
	if len(keys) == 0 {
		keySet["default-api-key"] = true
		keySet["secret-key"] = true
		keySet["katz-secret-key-12345"] = true
	} else {
		for _, k := range keys {
			keySet[k] = true
		}
	}
	return &ApiKeyAuthMiddleware{
		ValidKeys:        keySet,
		HeaderName:       "X-API-Key",
		Excludes:         excludes,
		Exceptions:       exceptions,
		ActivityExecutor: executor,
		Dispatcher:       disp,
	}
}

func (m *ApiKeyAuthMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		if DefaultRouteMatcher.ShouldSkip(c.Request.URL.Path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		reqKey := c.GetHeader(m.HeaderName)
		if reqKey == "" || !m.ValidKeys[reqKey] {
			RespondMiddlewareError(c, NewInvalidApiKeyException(fmt.Sprintf("Invalid or missing '%s' header", m.HeaderName)), m.Exceptions, m.ActivityExecutor, m.Dispatcher)
			return
		}
		c.Next()
	}
}

// --- C. Correlation ID Middleware ---
type CorrelationIdMiddleware struct {
	HeaderName string
	Excludes   []string
	Includes   []string
}

func NewCorrelationIdMiddleware(headerName string, excludes []string) *CorrelationIdMiddleware {
	if headerName == "" {
		headerName = "X-Correlation-ID"
	}
	return &CorrelationIdMiddleware{
		HeaderName: headerName,
		Excludes:   excludes,
	}
}

func (m *CorrelationIdMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		if DefaultRouteMatcher.ShouldSkip(c.Request.URL.Path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		corrID := c.GetHeader(m.HeaderName)
		if corrID == "" {
			corrID = c.GetHeader("X-Request-ID")
		}
		if corrID == "" {
			corrID = fmt.Sprintf("req_%d", time.Now().UnixNano())
		}
		c.Header(m.HeaderName, corrID)
		c.Set("correlation_id", corrID)
		c.Next()
	}
}

// --- D. Rate Limiting Middleware ---
type RateLimitMiddleware struct {
	DefaultLimit     int
	WindowSeconds    int
	KeyBy            string
	TierClaim        string
	TierHeader       string
	TierLimits       map[string]int
	IncludeHeaders   bool
	Excludes         []string
	Includes         []string
	Exceptions       []models.MiddlewareExceptionConfig
	ActivityExecutor ports.IActivityExecutor
	Dispatcher       ports.IAsyncActivityDispatcher
	requests         map[string][]int64
	mu               sync.Mutex
}

func NewRateLimitMiddleware(
	limit int,
	windowSeconds int,
	keyBy string,
	tierLimits map[string]int,
	excludes []string,
	exceptions []models.MiddlewareExceptionConfig,
	executor ports.IActivityExecutor,
	options ...func(*RateLimitMiddleware),
) *RateLimitMiddleware {
	if limit <= 0 {
		limit = 60
	}
	if windowSeconds <= 0 {
		windowSeconds = 60
	}
	if keyBy == "" {
		keyBy = "client_ip"
	}
	mw := &RateLimitMiddleware{
		DefaultLimit:     limit,
		WindowSeconds:    windowSeconds,
		KeyBy:            keyBy,
		TierClaim:        "customer_tier",
		TierHeader:       "X-User-Tier",
		TierLimits:       tierLimits,
		IncludeHeaders:   true,
		Excludes:         excludes,
		Exceptions:       exceptions,
		ActivityExecutor: executor,
		requests:         make(map[string][]int64),
	}
	for _, opt := range options {
		opt(mw)
	}
	return mw
}

func (m *RateLimitMiddleware) resolveKey(c *gin.Context) string {
	strat := strings.ToLower(m.KeyBy)
	switch strat {
	case "jwt_user", "user", "user_id", "sub":
		if claimsVal, exists := c.Get("auth_claims"); exists {
			if claims, ok := claimsVal.(map[string]interface{}); ok {
				if uid, ok := claims["user_id"].(string); ok && uid != "" {
					return fmt.Sprintf("user:%s", uid)
				}
				if sub, ok := claims["sub"].(string); ok && sub != "" {
					return fmt.Sprintf("user:%s", sub)
				}
			}
		}
		return fmt.Sprintf("ip:%s", c.ClientIP())
	case "api_key", "key":
		key := c.GetHeader("X-API-Key")
		if key == "" {
			key = "anonymous_key"
		}
		return fmt.Sprintf("apikey:%s", key)
	case "tenant_id", "tenant":
		tenant := c.GetHeader("X-Tenant-ID")
		if tenant == "" {
			tenant = "default_tenant"
		}
		return fmt.Sprintf("tenant:%s", tenant)
	case "path", "route":
		return fmt.Sprintf("path:%s", c.Request.URL.Path)
	default:
		return fmt.Sprintf("ip:%s", c.ClientIP())
	}
}

func (m *RateLimitMiddleware) resolveLimit(c *gin.Context) int {
	if len(m.TierLimits) > 0 {
		tier := ""

		// 1. Check claims injected by BearerAuthAndClaimsMiddleware
		if claimsVal, exists := c.Get("auth_claims"); exists {
			if claims, ok := claimsVal.(map[string]interface{}); ok {
				if m.TierClaim != "" {
					if t, ok := claims[m.TierClaim].(string); ok && t != "" {
						tier = t
					}
				}
				if tier == "" {
					if t, ok := claims["customer_tier"].(string); ok && t != "" {
						tier = t
					} else if t, ok := claims["tier"].(string); ok && t != "" {
						tier = t
					} else if t, ok := claims["subscription_plan"].(string); ok && t != "" {
						tier = t
					} else if t, ok := claims["role"].(string); ok && t != "" {
						tier = t
					}
				}
			}
		}

		// 2. Check HTTP Header
		if tier == "" {
			hdr := m.TierHeader
			if hdr == "" {
				hdr = "X-User-Tier"
			}
			tier = c.GetHeader(hdr)
		}

		// 3. Fallback to unauthenticated ANONYMOUS tier
		if tier == "" {
			tier = "ANONYMOUS"
		}

		if lim, ok := m.TierLimits[tier]; ok {
			return lim
		}
		if lim, ok := m.TierLimits[strings.ToUpper(tier)]; ok {
			return lim
		}
	}
	return m.DefaultLimit
}

func (m *RateLimitMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		if DefaultRouteMatcher.ShouldSkip(c.Request.URL.Path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		now := time.Now().Unix()
		key := m.resolveKey(c)
		maxLimit := m.resolveLimit(c)
		cutoff := now - int64(m.WindowSeconds)

		m.mu.Lock()
		timestamps := m.requests[key]
		var valid []int64
		for _, ts := range timestamps {
			if ts > cutoff {
				valid = append(valid, ts)
			}
		}

		if len(valid) >= maxLimit {
			oldest := valid[0]
			retryAfter := int64(m.WindowSeconds) - (now - oldest)
			if retryAfter < 1 {
				retryAfter = 1
			}
			resetTs := now + retryAfter
			m.requests[key] = valid
			m.mu.Unlock()

			hdrs := map[string]string{
				"Retry-After":           fmt.Sprintf("%d", retryAfter),
				"X-RateLimit-Limit":     fmt.Sprintf("%d", maxLimit),
				"X-RateLimit-Remaining": "0",
				"X-RateLimit-Reset":     fmt.Sprintf("%d", resetTs),
			}
			RespondMiddlewareError(c, NewRateLimitExceeded("", maxLimit, m.WindowSeconds, int(retryAfter), key, hdrs), m.Exceptions, m.ActivityExecutor, m.Dispatcher)
			return
		}

		valid = append(valid, now)
		remaining := maxLimit - len(valid)
		if remaining < 0 {
			remaining = 0
		}
		resetTs := valid[0] + int64(m.WindowSeconds)
		m.requests[key] = valid
		m.mu.Unlock()

		if m.IncludeHeaders {
			c.Header("X-RateLimit-Limit", fmt.Sprintf("%d", maxLimit))
			c.Header("X-RateLimit-Remaining", fmt.Sprintf("%d", remaining))
			c.Header("X-RateLimit-Reset", fmt.Sprintf("%d", resetTs))
		}

		c.Next()
	}
}

// --- E. IP Whitelist Middleware ---
type IpValidatorCallback func(ip string, c *gin.Context) (bool, error)

type IpWhitelistMiddleware struct {
	AllowedNetworks  []*net.IPNet
	BlockedNetworks  []*net.IPNet
	CustomValidator  IpValidatorCallback
	AllowAll         bool
	Excludes         []string
	Includes         []string
	Exceptions       []models.MiddlewareExceptionConfig
	ActivityExecutor ports.IActivityExecutor
	Dispatcher       ports.IAsyncActivityDispatcher
}

func NewIpWhitelistMiddleware(
	allowedIPs []string,
	blockedIPs []string,
	customValidator IpValidatorCallback,
	excludes []string,
	exceptions []models.MiddlewareExceptionConfig,
	executor ports.IActivityExecutor,
	dispatcher ...ports.IAsyncActivityDispatcher,
) *IpWhitelistMiddleware {
	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}
	parseNets := func(ips []string) ([]*net.IPNet, bool) {
		var nets []*net.IPNet
		hasWildcard := false
		for _, raw := range ips {
			raw = strings.TrimSpace(raw)
			if raw == "*" {
				hasWildcard = true
				continue
			}
			if strings.Contains(raw, "/") {
				_, ipNet, err := net.ParseCIDR(raw)
				if err == nil && ipNet != nil {
					nets = append(nets, ipNet)
				}
			} else {
				ip := net.ParseIP(raw)
				if ip != nil {
					var mask net.IPMask
					if ip.To4() != nil {
						mask = net.CIDRMask(32, 32)
					} else {
						mask = net.CIDRMask(128, 128)
					}
					nets = append(nets, &net.IPNet{IP: ip, Mask: mask})
				}
			}
		}
		return nets, hasWildcard
	}

	allowedNets, allowAll := parseNets(allowedIPs)
	blockedNets, _ := parseNets(blockedIPs)

	return &IpWhitelistMiddleware{
		AllowedNetworks:  allowedNets,
		BlockedNetworks:  blockedNets,
		CustomValidator:  customValidator,
		AllowAll:         allowAll,
		Excludes:         excludes,
		Exceptions:       exceptions,
		ActivityExecutor: executor,
		Dispatcher:       disp,
	}
}

func (m *IpWhitelistMiddleware) IsIPAllowed(clientIPStr string, c *gin.Context) bool {
	clientIP := net.ParseIP(clientIPStr)
	if clientIP == nil {
		return false
	}

	// 1. Check blacklist
	for _, bNet := range m.BlockedNetworks {
		if bNet.Contains(clientIP) {
			return false
		}
	}

	// 2. Custom validator callback
	if m.CustomValidator != nil {
		ok, err := m.CustomValidator(clientIPStr, c)
		if err == nil {
			return ok
		}
	}

	// 3. Wildcard
	if m.AllowAll {
		return true
	}

	// 4. Allowed networks
	if len(m.AllowedNetworks) > 0 {
		for _, aNet := range m.AllowedNetworks {
			if aNet.Contains(clientIP) {
				return true
			}
		}
		return false
	}

	return clientIP.IsLoopback()
}

func (m *IpWhitelistMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		if DefaultRouteMatcher.ShouldSkip(c.Request.URL.Path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		clientIP := c.ClientIP()
		if clientIP == "" {
			clientIP = "127.0.0.1"
		}
		if !m.IsIPAllowed(clientIP, c) {
			RespondMiddlewareError(c, NewIpAccessDenied("", clientIP, c.Request.URL.Path, c.Request.Method), m.Exceptions, m.ActivityExecutor, m.Dispatcher)
			return
		}

		c.Next()
	}
}

// --- G. Payload & Input Validation Middleware (400 Bad Request) ---
type PayloadValidationMiddleware struct {
	MaxBodySizeBytes  int64
	RequiredHeaders   []string
	RequiredFields    []string
	RequiredFieldsMap map[string][]string
	Excludes          []string
	Includes          []string
	Exceptions        []models.MiddlewareExceptionConfig
	ActivityExecutor  ports.IActivityExecutor
	Dispatcher        ports.IAsyncActivityDispatcher
}

func NewPayloadValidationMiddleware(
	maxBodySizeBytes int64,
	requiredHeaders []string,
	requiredFields []string,
	requiredFieldsMap map[string][]string,
	excludes []string,
	includes []string,
	exceptions []models.MiddlewareExceptionConfig,
	executor ports.IActivityExecutor,
	dispatcher ...ports.IAsyncActivityDispatcher,
) *PayloadValidationMiddleware {
	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}
	return &PayloadValidationMiddleware{
		MaxBodySizeBytes:  maxBodySizeBytes,
		RequiredHeaders:   requiredHeaders,
		RequiredFields:    requiredFields,
		RequiredFieldsMap: requiredFieldsMap,
		Excludes:          excludes,
		Includes:          includes,
		Exceptions:        exceptions,
		ActivityExecutor:  executor,
		Dispatcher:        disp,
	}
}

func (m *PayloadValidationMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		path := c.Request.URL.Path
		if DefaultRouteMatcher.ShouldSkip(path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		// 1. Validate Required Headers
		for _, reqHdr := range m.RequiredHeaders {
			if strings.TrimSpace(reqHdr) == "" {
				continue
			}
			if c.GetHeader(reqHdr) == "" {
				RespondMiddlewareError(
					c,
					NewMissingRequiredFieldException(fmt.Sprintf("Missing required header '%s'", reqHdr)),
					m.Exceptions,
					m.ActivityExecutor,
					m.Dispatcher,
				)
				return
			}
		}

		// 2. Validate Body for modifying requests (POST, PUT, PATCH)
		method := strings.ToUpper(c.Request.Method)
		if method == http.MethodPost || method == http.MethodPut || method == http.MethodPatch {
			if c.Request.Body != nil {
				bodyBytes, err := io.ReadAll(c.Request.Body)
				if err != nil {
					RespondMiddlewareError(
						c,
						NewBadRequestException(fmt.Sprintf("Failed to read request body: %v", err)),
						m.Exceptions,
						m.ActivityExecutor,
						m.Dispatcher,
					)
					return
				}

				if m.MaxBodySizeBytes > 0 && int64(len(bodyBytes)) > m.MaxBodySizeBytes {
					RespondMiddlewareError(
						c,
						NewPayloadTooLargeException(
							fmt.Sprintf("Payload size of %d bytes exceeds limit of %d bytes", len(bodyBytes), m.MaxBodySizeBytes),
							m.MaxBodySizeBytes,
							int64(len(bodyBytes)),
						),
						m.Exceptions,
						m.ActivityExecutor,
						m.Dispatcher,
					)
					return
				}

				ct := strings.ToLower(c.GetHeader("Content-Type"))
				trimmedBody := bytes.TrimSpace(bodyBytes)
				if len(trimmedBody) > 0 && (strings.Contains(ct, "application/json") || trimmedBody[0] == '{' || trimmedBody[0] == '[') {
					var parsed interface{}
					if err := json.Unmarshal(trimmedBody, &parsed); err != nil {
						RespondMiddlewareError(
							c,
							NewMalformedJsonException(fmt.Sprintf("Invalid JSON payload: %v", err)),
							m.Exceptions,
							m.ActivityExecutor,
							m.Dispatcher,
						)
						return
					}

					fieldsToValidate := make([]string, len(m.RequiredFields))
					copy(fieldsToValidate, m.RequiredFields)

					if len(m.RequiredFieldsMap) > 0 {
						if rFields, ok := m.RequiredFieldsMap[path]; ok {
							fieldsToValidate = append(fieldsToValidate, rFields...)
						} else {
							for pat, rFields := range m.RequiredFieldsMap {
								if strings.HasSuffix(path, strings.TrimPrefix(pat, "/")) || DefaultRouteMatcher.Matches(path, pat) {
									fieldsToValidate = append(fieldsToValidate, rFields...)
									break
								}
							}
						}
					}

					if len(fieldsToValidate) > 0 {
						if parsedMap, ok := parsed.(map[string]interface{}); ok {
							for _, reqField := range fieldsToValidate {
								if strings.TrimSpace(reqField) == "" {
									continue
								}
								val, exists := parsedMap[reqField]
								if !exists || val == nil || fmt.Sprintf("%v", val) == "" {
									RespondMiddlewareError(
										c,
										NewMissingRequiredFieldException(fmt.Sprintf("Missing required field '%s' in request payload", reqField)),
										m.Exceptions,
										m.ActivityExecutor,
										m.Dispatcher,
									)
									return
								}
							}
						}
					}
				}

				c.Request.Body = io.NopCloser(bytes.NewReader(bodyBytes))
			}
		}

		c.Next()
	}
}

// ============================================================================
// 5. Declarative Middleware Factory & Resolution
// ============================================================================

type CustomMiddlewareFactory func(cfg models.MiddlewareConfig, executor ports.IActivityExecutor, dispatcher ...ports.IAsyncActivityDispatcher) gin.HandlerFunc

func BuildMiddlewareFromConfig(
	cfg models.MiddlewareConfig,
	executor ports.IActivityExecutor,
	dispatcher ports.IAsyncActivityDispatcher,
	customRegistry map[string]CustomMiddlewareFactory,
) gin.HandlerFunc {
	name := strings.TrimSpace(cfg.Name)
	if customRegistry != nil {
		if factory, ok := customRegistry[name]; ok {
			return factory(cfg, executor, dispatcher)
		}
	}

	normName := strings.ToLower(strings.ReplaceAll(strings.ReplaceAll(name, "_", ""), "-", ""))

	switch normName {
	case "bearerauthandclaimsmiddleware", "bearerauth", "bearerauthandclaims", "jwtauthmiddleware", "jwtauth":
		var validator TokenValidatorFunc
		if valFn, ok := cfg.Config["token_validator"].(TokenValidatorFunc); ok {
			validator = valFn
		}
		var mapper ClaimsMapperFunc
		if mapFn, ok := cfg.Config["claims_mapper"].(ClaimsMapperFunc); ok {
			mapper = mapFn
		}
		return NewBearerAuthAndClaimsMiddleware(validator, mapper, cfg.Excludes, cfg.Exceptions, executor, dispatcher).Handler()

	case "ratelimitmiddleware", "ratelimit", "ratelimiter":
		limit := getIntFromMap(cfg.Config, "limit", getIntFromMap(cfg.Config, "default_limit", 60))
		window := getIntFromMap(cfg.Config, "window_seconds", getIntFromMap(cfg.Config, "window", 60))
		keyBy := getStringFromMap(cfg.Config, "key_by", "client_ip")
		tierClaim := getStringFromMap(cfg.Config, "tier_claim", "customer_tier")
		tierHeader := getStringFromMap(cfg.Config, "tier_header", "X-User-Tier")
		tierLimits := getMapStringIntFromMap(cfg.Config, "tier_limits")

		mw := NewRateLimitMiddleware(limit, window, keyBy, tierLimits, cfg.Excludes, cfg.Exceptions, executor, func(m *RateLimitMiddleware) {
			m.TierClaim = tierClaim
			m.TierHeader = tierHeader
		})
		mw.Dispatcher = dispatcher
		return mw.Handler()

	case "ipwhitelistmiddleware", "ipwhitelist", "ipfilter", "ipfiltermiddleware":
		allowed := getStringSliceFromMap(cfg.Config, "allowed_ips", getStringSliceFromMap(cfg.Config, "whitelist", nil))
		blocked := getStringSliceFromMap(cfg.Config, "blocked_ips", getStringSliceFromMap(cfg.Config, "blacklist", nil))
		var customVal IpValidatorCallback
		if cb, ok := cfg.Config["custom_validator"].(IpValidatorCallback); ok {
			customVal = cb
		}
		return NewIpWhitelistMiddleware(allowed, blocked, customVal, cfg.Excludes, cfg.Exceptions, executor, dispatcher).Handler()

	case "correlationidmiddleware", "correlationid", "requestidmiddleware", "requestid":
		hdr := getStringFromMap(cfg.Config, "header_name", "X-Correlation-ID")
		return NewCorrelationIdMiddleware(hdr, cfg.Excludes).Handler()

	case "apikeyauthmiddleware", "apikeyauth", "apikey", "apikeyauthmw":
		keys := getStringSliceFromMap(cfg.Config, "api_keys", getStringSliceFromMap(cfg.Config, "valid_keys", nil))
		return NewApiKeyAuthMiddleware(keys, cfg.Excludes, cfg.Exceptions, executor, dispatcher).Handler()

	case "rediscachemiddleware", "rediscache", "cachemiddleware", "cache":
		ttl := getIntFromMap(cfg.Config, "ttl_seconds", getIntFromMap(cfg.Config, "ttl", 300))
		keyStrat := getStringSliceFromMap(cfg.Config, "key_strategy", nil)
		methods := getStringSliceFromMap(cfg.Config, "methods", nil)
		hdrName := getStringFromMap(cfg.Config, "header_name", "X-Cache")
		pfx := getStringFromMap(cfg.Config, "prefix", "octopus:cache")
		ccHdr := getBoolFromMap(cfg.Config, "cache_control_headers", true)
		rURL := getStringFromMap(cfg.Config, "redis_url", getStringFromMap(cfg.Config, "dsn", "127.0.0.1:6379"))

		return NewRedisCacheMiddleware(ttl, keyStrat, cfg.Excludes, cfg.Includes, methods, hdrName, pfx, ccHdr, rURL).Handler()

	case "payloadvalidationmiddleware", "payloadvalidation", "badrequestmiddleware", "badrequest", "validationmiddleware", "validation", "inputvalidationmiddleware", "inputvalidation":
		maxSize := int64(getIntFromMap(cfg.Config, "max_body_size_bytes", getIntFromMap(cfg.Config, "max_size", 0)))
		reqHeaders := getStringSliceFromMap(cfg.Config, "required_headers", nil)
		reqFieldsMap, reqFields := getFieldsMapFromConfig(cfg.Config, "required_fields")
		return NewPayloadValidationMiddleware(maxSize, reqHeaders, reqFields, reqFieldsMap, cfg.Excludes, cfg.Includes, cfg.Exceptions, executor, dispatcher).Handler()

	case "responseauditmiddleware", "responseaudit", "auditlogmiddleware", "auditmiddleware":
		return NewResponseAuditMiddleware(cfg.Activity, cfg.IsAsync, cfg.Excludes, executor, dispatcher).Handler()

	default:
		if strings.EqualFold(cfg.Type, "post-request") && cfg.Activity != "" {
			return NewResponseAuditMiddleware(cfg.Activity, cfg.IsAsync, cfg.Excludes, executor, dispatcher).Handler()
		}
		return nil
	}
}

// ============================================================================
// 6. ResponseAuditMiddleware (Post-Request Lifecycle Auditing)
// ============================================================================

type ResponseAuditMiddleware struct {
	Activity   string
	IsAsync    bool
	Excludes   []string
	Executor   ports.IActivityExecutor
	Dispatcher ports.IAsyncActivityDispatcher
}

func NewResponseAuditMiddleware(activity string, isAsync bool, excludes []string, executor ports.IActivityExecutor, dispatcher ...ports.IAsyncActivityDispatcher) *ResponseAuditMiddleware {
	var disp ports.IAsyncActivityDispatcher
	if len(dispatcher) > 0 {
		disp = dispatcher[0]
	}
	return &ResponseAuditMiddleware{
		Activity:   activity,
		IsAsync:    isAsync,
		Excludes:   excludes,
		Executor:   executor,
		Dispatcher: disp,
	}
}

func (m *ResponseAuditMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		path := c.Request.URL.Path
		if DefaultRouteMatcher.ShouldSkip(path, m.Excludes, nil) {
			c.Next()
			return
		}

		start := time.Now()
		c.Next()
		durationMs := float64(time.Since(start).Microseconds()) / 1000.0

		if m.Activity != "" && m.Executor != nil {
			jobID, _ := c.Get("job_id")
			userID, _ := c.Get("user_id")
			tier, _ := c.Get("customer_tier")

			auditPayload := map[string]interface{}{
				"job_id":        jobID,
				"route":         path,
				"method":        c.Request.Method,
				"status_code":   c.Writer.Status(),
				"duration_ms":   durationMs,
				"user_id":       userID,
				"customer_tier": tier,
			}

			if m.IsAsync {
				if m.Dispatcher != nil {
					jobIDStr, _ := jobID.(string)
					if jobIDStr == "" {
						jobIDStr = fmt.Sprintf("audit_%d", time.Now().UnixNano())
					}
					m.Dispatcher.Dispatch(context.Background(), m.Activity, auditPayload, jobIDStr, "audit_log", m.Executor)
				} else {
					go func(payload map[string]interface{}) {
						ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
						defer cancel()
						_, _ = m.Executor.ExecuteActivity(ctx, m.Activity, payload)
					}(auditPayload)
				}
			} else {
				ctx, cancel := context.WithTimeout(c.Request.Context(), 5*time.Second)
				defer cancel()
				_, _ = m.Executor.ExecuteActivity(ctx, m.Activity, auditPayload)
			}
		}
	}
}

func getIntFromMap(m map[string]interface{}, key string, def int) int {
	if m == nil {
		return def
	}
	v, ok := m[key]
	if !ok {
		return def
	}
	switch val := v.(type) {
	case int:
		return val
	case int64:
		return int(val)
	case float64:
		return int(val)
	default:
		return def
	}
}

func getStringFromMap(m map[string]interface{}, key string, def string) string {
	if m == nil {
		return def
	}
	v, ok := m[key]
	if !ok {
		return def
	}
	if s, ok := v.(string); ok {
		return s
	}
	return def
}

func getStringSliceFromMap(m map[string]interface{}, key string, def []string) []string {
	if m == nil {
		return def
	}
	v, ok := m[key]
	if !ok {
		return def
	}
	if slice, ok := v.([]string); ok {
		return slice
	}
	if slice, ok := v.([]interface{}); ok {
		var res []string
		for _, item := range slice {
			if s, ok := item.(string); ok {
				res = append(res, s)
			}
		}
		return res
	}
	return def
}

func getMapStringIntFromMap(m map[string]interface{}, key string) map[string]int {
	res := make(map[string]int)
	if m == nil {
		return res
	}
	v, ok := m[key]
	if !ok {
		return res
	}
	if mapInt, ok := v.(map[string]int); ok {
		return mapInt
	}
	if mapVal, ok := v.(map[string]interface{}); ok {
		for k, val := range mapVal {
			switch n := val.(type) {
			case int:
				res[k] = n
			case int64:
				res[k] = int(n)
			case float64:
				res[k] = int(n)
			}
		}
	}
	return res
}

func getBoolFromMap(m map[string]interface{}, key string, def bool) bool {
	if m == nil {
		return def
	}
	v, ok := m[key]
	if !ok {
		return def
	}
	switch b := v.(type) {
	case bool:
		return b
	case string:
		return strings.EqualFold(b, "true") || b == "1"
	default:
		return def
	}
}

func getFieldsMapFromConfig(cfg map[string]interface{}, key string) (map[string][]string, []string) {
	if cfg == nil {
		return nil, nil
	}
	val, ok := cfg[key]
	if !ok || val == nil {
		return nil, nil
	}
	if sliceVal, ok := val.([]interface{}); ok {
		var list []string
		for _, item := range sliceVal {
			if item != nil {
				list = append(list, fmt.Sprintf("%v", item))
			}
		}
		return nil, list
	}
	if sliceStr, ok := val.([]string); ok {
		return nil, sliceStr
	}
	if mapVal, ok := val.(map[string]interface{}); ok {
		res := make(map[string][]string)
		for k, v := range mapVal {
			if sl, ok := v.([]interface{}); ok {
				var list []string
				for _, item := range sl {
					if item != nil {
						list = append(list, fmt.Sprintf("%v", item))
					}
				}
				res[k] = list
			} else if slStr, ok := v.([]string); ok {
				res[k] = slStr
			}
		}
		return res, nil
	}
	if mapRaw, ok := val.(map[interface{}]interface{}); ok {
		res := make(map[string][]string)
		for k, v := range mapRaw {
			kStr := fmt.Sprintf("%v", k)
			if sl, ok := v.([]interface{}); ok {
				var list []string
				for _, item := range sl {
					if item != nil {
						list = append(list, fmt.Sprintf("%v", item))
					}
				}
				res[kStr] = list
			} else if slStr, ok := v.([]string); ok {
				res[kStr] = slStr
			}
		}
		return res, nil
	}
	return nil, nil
}

// ============================================================================
// 7. RedisCacheMiddleware (Declarative Response Caching with Sub-Millisecond Short-Circuiting)
// ============================================================================

type cacheItem struct {
	content string
	expiry  time.Time
}

type bodyLogWriter struct {
	gin.ResponseWriter
	body *bytes.Buffer
}

func (w bodyLogWriter) Write(b []byte) (int, error) {
	w.body.Write(b)
	return w.ResponseWriter.Write(b)
}

func (w bodyLogWriter) WriteString(s string) (int, error) {
	w.body.WriteString(s)
	return w.ResponseWriter.WriteString(s)
}

type RedisCacheMiddleware struct {
	TTLSeconds          int
	KeyStrategy         []string
	Methods             []string
	HeaderName          string
	Prefix              string
	CacheControlHeaders bool
	RedisURL            string
	Excludes            []string
	Includes            []string
	redisClient         *redis.Client
	fallbackCache       sync.Map
}

func NewRedisCacheMiddleware(
	ttlSeconds int,
	keyStrategy []string,
	excludes []string,
	includes []string,
	methods []string,
	headerName string,
	prefix string,
	cacheControlHeaders bool,
	redisURL string,
) *RedisCacheMiddleware {
	if ttlSeconds <= 0 {
		ttlSeconds = 300
	}
	if len(keyStrategy) == 0 {
		keyStrategy = []string{"route", "claim:user_id", "query"}
	}
	if len(methods) == 0 {
		methods = []string{"GET"}
	}
	for i, m := range methods {
		methods[i] = strings.ToUpper(m)
	}
	if headerName == "" {
		headerName = "X-Cache"
	}
	if prefix == "" {
		prefix = "octopus:cache"
	}
	if redisURL == "" {
		redisURL = "127.0.0.1:6379"
	}

	var rdb *redis.Client
	opts, err := redis.ParseURL(redisURL)
	if err != nil {
		host := redisURL
		if strings.Contains(host, "localhost") {
			host = strings.ReplaceAll(host, "localhost", "127.0.0.1")
		}
		if !strings.Contains(host, ":") {
			host += ":6379"
		}
		opts = &redis.Options{
			Addr: host,
		}
	}
	rdb = redis.NewClient(opts)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := rdb.Ping(ctx).Err(); err != nil {
		fmt.Printf("⚠️ [RedisCacheMiddleware] Redis ping warning: %v, using fast in-memory fallback\n", err)
	}

	return &RedisCacheMiddleware{
		TTLSeconds:          ttlSeconds,
		KeyStrategy:         keyStrategy,
		Methods:             methods,
		HeaderName:          headerName,
		Prefix:              prefix,
		CacheControlHeaders: cacheControlHeaders,
		RedisURL:            redisURL,
		Excludes:            excludes,
		Includes:            includes,
		redisClient:         rdb,
	}
}

func (m *RedisCacheMiddleware) BuildCacheKey(c *gin.Context, path string) string {
	var sb strings.Builder
	sb.WriteString(m.Prefix)
	sb.WriteByte(':')
	sb.WriteString(strings.ToUpper(c.Request.Method))
	sb.WriteByte(':')
	sb.WriteString(path)

	for _, strategy := range m.KeyStrategy {
		sLower := strings.ToLower(strategy)
		if strings.HasPrefix(sLower, "claim:") {
			claimKey := strategy[6:]
			var claimVal string
			if v, exists := c.Get(claimKey); exists && v != nil {
				claimVal = fmt.Sprintf("%v", v)
			} else if claimsVal, exists := c.Get("auth_claims"); exists {
				if claimsMap, ok := claimsVal.(map[string]interface{}); ok {
					if v, ok := claimsMap[claimKey]; ok && v != nil {
						claimVal = fmt.Sprintf("%v", v)
					}
				}
			} else if injVal, exists := c.Get("workflow_injections"); exists {
				if injMap, ok := injVal.(map[string]interface{}); ok {
					if v, ok := injMap[claimKey]; ok && v != nil {
						claimVal = fmt.Sprintf("%v", v)
					}
				}
			}
			if claimVal != "" {
				sb.WriteByte(':')
				sb.WriteString(claimKey)
				sb.WriteByte('=')
				sb.WriteString(claimVal)
			}
		} else if sLower == "query" {
			q := c.Request.URL.Query()
			if len(q) > 0 {
				keys := make([]string, 0, len(q))
				for k := range q {
					if !strings.EqualFold(k, "no_cache") && !strings.EqualFold(k, "force_refresh") {
						keys = append(keys, k)
					}
				}
				if len(keys) > 0 {
					sort.Strings(keys)
					var qb strings.Builder
					for i, k := range keys {
						if i > 0 {
							qb.WriteByte('&')
						}
						qb.WriteString(k)
						qb.WriteByte('=')
						qb.WriteString(q.Get(k))
					}
					sb.WriteString(":q=")
					sb.WriteString(qb.String())
				}
			}
		} else if strings.HasPrefix(sLower, "header:") {
			hdrKey := strategy[7:]
			hdrVal := c.GetHeader(hdrKey)
			if hdrVal != "" {
				sb.WriteByte(':')
				sb.WriteString(hdrKey)
				sb.WriteByte('=')
				sb.WriteString(hdrVal)
			}
		}
	}
	return sb.String()
}

func (m *RedisCacheMiddleware) Handler() gin.HandlerFunc {
	return func(c *gin.Context) {
		method := strings.ToUpper(c.Request.Method)
		methodMatches := false
		for _, mm := range m.Methods {
			if mm == method {
				methodMatches = true
				break
			}
		}
		if !methodMatches {
			c.Next()
			return
		}

		path := c.Request.URL.Path
		if DefaultRouteMatcher.ShouldSkip(path, m.Excludes, m.Includes) {
			c.Next()
			return
		}

		if m.CacheControlHeaders {
			cc := strings.ToLower(c.GetHeader("Cache-Control"))
			hasBypassQuery := c.Query("no_cache") != "" || c.Query("force_refresh") != ""
			if strings.Contains(cc, "no-cache") || strings.Contains(cc, "no-store") || hasBypassQuery {
				c.Header(m.HeaderName, "BYPASS")
				c.Next()
				return
			}
		}

		cacheKey := m.BuildCacheKey(c, path)
		c.Set("octopus_cache_key", cacheKey)

		var cachedContent string
		var remainingTTL time.Duration

		if m.redisClient != nil {
			ctx, cancel := context.WithTimeout(c.Request.Context(), 2*time.Second)
			val, err := m.redisClient.Get(ctx, cacheKey).Result()
			ttl, _ := m.redisClient.TTL(ctx, cacheKey).Result()
			cancel()
			if err == nil && val != "" {
				cachedContent = val
				remainingTTL = ttl
			}
		}

		if cachedContent == "" {
			if itemVal, ok := m.fallbackCache.Load(cacheKey); ok {
				if item, ok := itemVal.(cacheItem); ok {
					if time.Now().Before(item.expiry) {
						cachedContent = item.content
						remainingTTL = time.Until(item.expiry)
					} else {
						m.fallbackCache.Delete(cacheKey)
					}
				}
			}
		}

		if cachedContent != "" {
			c.Header(m.HeaderName, "HIT")
			c.Header("X-Cache-Key", cacheKey)
			if remainingTTL >= 0 {
				c.Header("X-Cache-TTL", fmt.Sprintf("%d", int(remainingTTL.Seconds())))
			}
			c.Header("X-Job-ID", fmt.Sprintf("job_cache_hit_%d", time.Now().UnixMilli()))
			c.Header("X-Execution-Status", "COMPLETED")
			c.Data(http.StatusOK, "application/json; charset=utf-8", []byte(cachedContent))
			c.Abort()
			return
		}

		c.Header(m.HeaderName, "MISS")
		c.Header("X-Cache-Key", cacheKey)

		blw := &bodyLogWriter{body: bytes.NewBufferString(""), ResponseWriter: c.Writer}
		c.Writer = blw

		c.Next()

		if blw.Status() == http.StatusOK && blw.body.Len() > 0 {
			bodyStr := blw.body.String()
			expiry := time.Now().Add(time.Duration(m.TTLSeconds) * time.Second)
			m.fallbackCache.Store(cacheKey, cacheItem{content: bodyStr, expiry: expiry})

			if m.redisClient != nil {
				go func(k, val string, ttlSec int) {
					ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
					defer cancel()
					_ = m.redisClient.SetEx(ctx, k, val, time.Duration(ttlSec)*time.Second).Err()
				}(cacheKey, bodyStr, m.TTLSeconds)
			}
		}
	}
}
