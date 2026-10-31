//! The built-in middleware: authentication, API keys, correlation IDs, rate
//! limiting, IP filtering, payload validation, response auditing and the
//! response cache.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::request::{
    unix_nanos, Decision, ErrorResponder, Middleware, RequestMeta, Response, ResponseMeta,
    JSON_CONTENT_TYPE,
};
use super::*;
use crate::adapters::outbound::redis_kv::RedisKv;
use crate::crypto::{base64url_decode, constant_time_eq, hmac_sha256};
use crate::domain::models::MiddlewareExceptionConfig;
use crate::domain::ports::{IActivityExecutor, IAsyncActivityDispatcher};
use crate::json::{self, Value, ValueMap};
use crate::net::IpNet;
use crate::vmap;

fn skipped(req: &RequestMeta, excludes: &[String], includes: &[String]) -> bool {
    RouteMatcher::new().should_skip(req.path, excludes, includes)
}

// --- A. Bearer (JWT) Auth Middleware ---

pub type TokenValidatorFunc = Arc<dyn Fn(&str) -> Result<ValueMap, String> + Send + Sync>;
/// An `Err` refuses the request (`UserNotAuthorized`).
pub type ClaimsMapperFunc = Arc<dyn Fn(&ValueMap) -> Result<ValueMap, String> + Send + Sync>;
/// Host code standing in for part of a built-in's check (a function given in
/// code, an overridden method); see `MiddlewareHooks`.
pub type RequestHook<T> = Arc<dyn Fn(&RequestMeta) -> Result<T, String> + Send + Sync>;

#[derive(Clone)]
pub struct BearerAuthAndClaimsMiddleware {
    pub token_validator: TokenValidatorFunc,
    pub claims_mapper: ClaimsMapperFunc,
    pub token_header: String,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
}

impl BearerAuthAndClaimsMiddleware {
    pub fn new(
        validator: Option<TokenValidatorFunc>,
        mapper: Option<ClaimsMapperFunc>,
        excludes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        let token_validator = validator.unwrap_or_else(|| {
            Arc::new(|_token: &str| {
                Err("no token validator or jwt_secret is configured".to_string())
            })
        });
        let claims_mapper =
            mapper.unwrap_or_else(|| Arc::new(|claims: &ValueMap| Ok(claims.clone())));
        Self {
            token_validator,
            claims_mapper,
            token_header: "Authorization".to_string(),
            excludes,
            includes: Vec::new(),
            errors: ErrorResponder::new(exceptions, executor, dispatcher),
        }
    }
}

impl Middleware for BearerAuthAndClaimsMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }

        let auth_header = req.header(&self.token_header);
        if auth_header.is_empty() {
            return self.errors.reject(
                req,
                new_user_not_authenticated(&format!("Missing '{}' header", self.token_header)),
            );
        }

        let token = if auth_header.len() >= 7 && auth_header[..7].eq_ignore_ascii_case("bearer ") {
            auth_header[7..].trim().to_string()
        } else {
            auth_header
        };

        match (self.token_validator)(&token) {
            Ok(claims) => match (self.claims_mapper)(&claims) {
                Ok(injections) => Decision::next()
                    .set("auth_claims", Value::Object(claims))
                    .set("workflow_injections", Value::Object(injections)),
                Err(err) => self.errors.reject(
                    req,
                    new_user_not_authorized(&format!("Claims mapping error: {}", err)),
                ),
            },
            Err(err) => self.errors.reject(
                req,
                new_user_not_authenticated(&format!("Token validation error: {}", err)),
            ),
        }
    }
}

pub fn verify_hs256_jwt(token: &str, secret: &str) -> Result<ValueMap, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("malformed JWT".to_string());
    }
    let header_bytes = base64url_decode(parts[0]).map_err(|_| "malformed JWT".to_string())?;
    let claims_bytes = base64url_decode(parts[1]).map_err(|_| "malformed JWT".to_string())?;
    let sig_bytes = base64url_decode(parts[2]).map_err(|_| "malformed JWT".to_string())?;

    let header_str = std::str::from_utf8(&header_bytes).map_err(|_| "malformed JWT".to_string())?;
    let claims_str = std::str::from_utf8(&claims_bytes).map_err(|_| "malformed JWT".to_string())?;

    let header_val = json::parse(header_str).map_err(|_| "malformed JWT".to_string())?;
    let claims_val = json::parse(claims_str).map_err(|_| "malformed JWT".to_string())?;

    let header = header_val
        .as_object()
        .ok_or_else(|| "malformed JWT".to_string())?;
    let claims = claims_val
        .as_object()
        .ok_or_else(|| "malformed JWT".to_string())?
        .clone();

    let alg = header.get("alg").and_then(|v| v.as_str()).unwrap_or("");
    if alg != "HS256" {
        return Err("JWT must be HS256".to_string());
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let expected_sig = hmac_sha256(secret.as_bytes(), signing_input.as_bytes());
    if !constant_time_eq(&expected_sig, &sig_bytes) {
        return Err("invalid JWT signature".to_string());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    if let Some(exp) = claims.get("exp").and_then(|v| v.as_f64()) {
        if now >= exp {
            return Err("JWT has expired".to_string());
        }
    }
    if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_f64()) {
        if now < nbf {
            return Err("JWT is not valid yet".to_string());
        }
    }

    Ok(claims)
}

// --- B. API Key Auth Middleware ---
#[derive(Clone)]
pub struct ApiKeyAuthMiddleware {
    pub valid_keys: Vec<String>,
    pub header_name: String,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
}

impl ApiKeyAuthMiddleware {
    pub fn new(
        keys: Vec<String>,
        excludes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        let valid_keys: Vec<String> = keys.into_iter().filter(|k| !k.is_empty()).collect();
        Self {
            valid_keys,
            header_name: "X-API-Key".to_string(),
            excludes,
            includes: Vec::new(),
            errors: ErrorResponder::new(exceptions, executor, dispatcher),
        }
    }
}

impl Middleware for ApiKeyAuthMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }

        let req_key = req.header(&self.header_name);
        let matched = !req_key.is_empty()
            && self
                .valid_keys
                .iter()
                .any(|k| constant_time_eq(k.as_bytes(), req_key.as_bytes()));
        if !matched {
            return self.errors.reject(
                req,
                new_invalid_api_key_exception(&format!(
                    "Invalid or missing '{}' header",
                    self.header_name
                )),
            );
        }
        Decision::next()
    }
}

// --- C. Correlation ID Middleware ---
#[derive(Clone)]
pub struct CorrelationIdMiddleware {
    pub header_name: String,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
}

impl CorrelationIdMiddleware {
    pub fn new(header_name: &str, excludes: Vec<String>) -> Self {
        let hdr = if header_name.is_empty() {
            "X-Correlation-ID".to_string()
        } else {
            header_name.to_string()
        };
        Self {
            header_name: hdr,
            excludes,
            includes: Vec::new(),
        }
    }
}

impl Middleware for CorrelationIdMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }

        let mut corr_id = req.header(&self.header_name);
        if corr_id.is_empty() {
            corr_id = req.header("X-Request-ID");
        }
        if corr_id.is_empty() {
            corr_id = format!("req_{}", unix_nanos());
        }
        Decision::next()
            .header(&self.header_name, &corr_id)
            .set("correlation_id", Value::String(corr_id))
    }
}

// --- D. Rate Limiting Middleware ---
pub struct RateLimitMiddleware {
    pub default_limit: i32,
    pub window_seconds: i32,
    pub key_by: String,
    pub tier_claim: String,
    pub tier_header: String,
    pub tier_limits: HashMap<String, i32>,
    pub include_headers: bool,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
    /// The key and the limit from host code; on an `Err`, the built-in's own.
    pub key_hook: Option<RequestHook<String>>,
    pub limit_hook: Option<RequestHook<i32>>,
    requests: Mutex<HashMap<String, Vec<i64>>>,
}

impl RateLimitMiddleware {
    pub fn new(
        limit: i32,
        window_seconds: i32,
        key_by: &str,
        tier_limits: HashMap<String, i32>,
        excludes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
    ) -> Self {
        let default_limit = if limit <= 0 { 60 } else { limit };
        let win = if window_seconds <= 0 {
            60
        } else {
            window_seconds
        };
        let kb = if key_by.is_empty() {
            "client_ip".to_string()
        } else {
            key_by.to_string()
        };
        Self {
            default_limit,
            window_seconds: win,
            key_by: kb,
            tier_claim: "customer_tier".to_string(),
            tier_header: "X-User-Tier".to_string(),
            tier_limits,
            include_headers: true,
            excludes,
            includes: Vec::new(),
            errors: ErrorResponder::new(exceptions, executor, None),
            key_hook: None,
            limit_hook: None,
            requests: Mutex::new(HashMap::new()),
        }
    }

    fn resolve_key(&self, req: &RequestMeta) -> String {
        match self.key_by.to_lowercase().as_str() {
            "jwt_user" | "user" | "user_id" | "sub" => {
                if let Some(Value::Object(claims)) = req.get("auth_claims") {
                    if let Some(uid) = claims.get("user_id").and_then(|v| v.as_str()) {
                        if !uid.is_empty() {
                            return format!("user:{}", uid);
                        }
                    }
                    for k in ["sub", "username"] {
                        if let Some(id) = claims.get(k).and_then(|v| v.as_str()) {
                            if !id.is_empty() {
                                return format!("user:{}", id);
                            }
                        }
                    }
                }
                format!("ip:{}", req.client_ip)
            }
            "api_key" | "key" => {
                let mut key = req.header("X-API-Key");
                if key.is_empty() {
                    key = "anonymous_key".to_string();
                }
                format!("apikey:{}", key)
            }
            "tenant_id" | "tenant" => {
                if let Some(Value::Object(claims)) = req.get("auth_claims") {
                    if let Some(t) = claims.get("tenant_id").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            return format!("tenant:{}", t);
                        }
                    }
                }
                let mut tenant = req.header("X-Tenant-ID");
                if tenant.is_empty() {
                    tenant = "default_tenant".to_string();
                }
                format!("tenant:{}", tenant)
            }
            "path" | "route" => format!("path:{}", req.path),
            _ => format!("ip:{}", req.client_ip),
        }
    }

    fn resolve_limit(&self, req: &RequestMeta) -> i32 {
        if !self.tier_limits.is_empty() {
            let mut tier = String::new();
            if let Some(Value::Object(claims)) = req.get("auth_claims") {
                if !self.tier_claim.is_empty() {
                    if let Some(t) = claims.get(&self.tier_claim).and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            tier = t.to_string();
                        }
                    }
                }
                if tier.is_empty() {
                    for k in &["customer_tier", "tier", "subscription_plan", "role"] {
                        if let Some(t) = claims.get(*k).and_then(|v| v.as_str()) {
                            if !t.is_empty() {
                                tier = t.to_string();
                                break;
                            }
                        }
                    }
                }
            }
            if tier.is_empty() {
                let hdr = if self.tier_header.is_empty() {
                    "X-User-Tier"
                } else {
                    &self.tier_header
                };
                tier = req.header(hdr);
            }
            if tier.is_empty() {
                tier = "ANONYMOUS".to_string();
            }
            if let Some(&lim) = self.tier_limits.get(&tier) {
                return lim;
            }
            if let Some(&lim) = self.tier_limits.get(&tier.to_uppercase()) {
                return lim;
            }
        }
        self.default_limit
    }
}

impl Middleware for RateLimitMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let key = self
            .key_hook
            .as_ref()
            .and_then(|hook| hook(req).ok())
            .unwrap_or_else(|| self.resolve_key(req));
        let max_limit = self
            .limit_hook
            .as_ref()
            .and_then(|hook| hook(req).ok())
            .unwrap_or_else(|| self.resolve_limit(req));
        let cutoff = now - (self.window_seconds as i64);

        let mut guard = self.requests.lock().unwrap();
        let timestamps = guard.entry(key.clone()).or_default();
        timestamps.retain(|&ts| ts > cutoff);

        if (timestamps.len() as i32) >= max_limit {
            let oldest = timestamps.first().copied().unwrap_or(now);
            let mut retry_after = (self.window_seconds as i64) - (now - oldest);
            if retry_after < 1 {
                retry_after = 1;
            }
            let reset_ts = now + retry_after;
            drop(guard);

            let mut hdrs = BTreeMap::new();
            hdrs.insert("Retry-After".to_string(), retry_after.to_string());
            hdrs.insert("X-RateLimit-Limit".to_string(), max_limit.to_string());
            hdrs.insert("X-RateLimit-Remaining".to_string(), "0".to_string());
            hdrs.insert("X-RateLimit-Reset".to_string(), reset_ts.to_string());

            return self.errors.reject(
                req,
                new_rate_limit_exceeded(
                    "",
                    max_limit,
                    self.window_seconds,
                    retry_after as i32,
                    &key,
                    hdrs,
                ),
            );
        }

        timestamps.push(now);
        let remaining = (max_limit - (timestamps.len() as i32)).max(0);
        let reset_ts = timestamps[0] + (self.window_seconds as i64);
        drop(guard);

        if !self.include_headers {
            return Decision::next();
        }
        Decision::next()
            .header("X-RateLimit-Limit", &max_limit.to_string())
            .header("X-RateLimit-Remaining", &remaining.to_string())
            .header("X-RateLimit-Reset", &reset_ts.to_string())
    }
}

// --- D2. Role Authorization Middleware ---

/// Lets a request through only when the caller has one of `required_roles`:
/// from the `roles` (or `role`) claim when a bearer token was verified,
/// otherwise from a comma-separated header. No required roles: everyone.
#[derive(Clone)]
pub struct RoleAuthorizationMiddleware {
    pub required_roles: Vec<String>,
    pub roles_header: String,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
    /// The caller's roles from host code; an `Err` refuses the request.
    pub role_extractor: Option<RequestHook<Vec<String>>>,
}

impl RoleAuthorizationMiddleware {
    pub fn new(
        required_roles: Vec<String>,
        roles_header: &str,
        excludes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        Self {
            required_roles,
            roles_header: if roles_header.is_empty() {
                "X-User-Roles".to_string()
            } else {
                roles_header.to_string()
            },
            excludes,
            includes: Vec::new(),
            errors: ErrorResponder::new(exceptions, executor, dispatcher),
            role_extractor: None,
        }
    }

    fn caller_roles(&self, req: &RequestMeta) -> Vec<String> {
        if let Some(Value::Object(claims)) = req.get("auth_claims") {
            return match claims.get("roles").or_else(|| claims.get("role")) {
                Some(Value::Array(items)) => items.iter().map(|v| v.to_display_string()).collect(),
                Some(Value::Null) | None => Vec::new(),
                Some(v) => vec![v.to_display_string()],
            };
        }
        req.header(&self.roles_header)
            .split(',')
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect()
    }
}

impl Middleware for RoleAuthorizationMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if self.required_roles.is_empty() || skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }
        let roles = match &self.role_extractor {
            Some(extract) => match extract(req) {
                Ok(roles) => roles,
                Err(err) => {
                    return self.errors.reject(
                        req,
                        new_user_not_authorized(&format!("Role extraction failed: {}", err)),
                    )
                }
            },
            None => self.caller_roles(req),
        };
        if self.required_roles.iter().any(|r| roles.contains(r)) {
            return Decision::next();
        }
        let listed: Vec<String> = self
            .required_roles
            .iter()
            .map(|r| format!("'{}'", r))
            .collect();
        self.errors.reject(
            req,
            new_user_not_authorized(&format!(
                "User lacks required role(s): [{}]",
                listed.join(", ")
            )),
        )
    }
}

// --- E. IP Whitelist Middleware ---
/// Consulted after the blocked list: `Some` answers, `None` (or an `Err`)
/// leaves it to the allowed list.
pub type IpValidatorCallback =
    Arc<dyn Fn(&str, &RequestMeta) -> Result<Option<bool>, String> + Send + Sync>;
/// The whole answer, in place of the lists; an `Err` refuses.
pub type IpDecisionHook = Arc<dyn Fn(&str, &RequestMeta) -> Result<bool, String> + Send + Sync>;

#[derive(Clone)]
pub struct IpWhitelistMiddleware {
    pub allowed_networks: Vec<IpNet>,
    pub blocked_networks: Vec<IpNet>,
    pub custom_validator: Option<IpValidatorCallback>,
    pub allowed_hook: Option<IpDecisionHook>,
    pub allow_all: bool,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
}

impl IpWhitelistMiddleware {
    pub fn new(
        allowed_ips: Vec<String>,
        blocked_ips: Vec<String>,
        custom_validator: Option<IpValidatorCallback>,
        excludes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        fn parse_nets(ips: &[String]) -> (Vec<IpNet>, bool) {
            let mut nets = Vec::new();
            let mut has_wildcard = false;
            for raw in ips {
                let trimmed = raw.trim();
                if trimmed == "*" {
                    has_wildcard = true;
                    continue;
                }
                if let Ok(net) = IpNet::parse(trimmed) {
                    nets.push(net);
                }
            }
            (nets, has_wildcard)
        }

        let (allowed_networks, allow_all) = parse_nets(&allowed_ips);
        let (blocked_networks, _) = parse_nets(&blocked_ips);

        Self {
            allowed_networks,
            blocked_networks,
            custom_validator,
            allowed_hook: None,
            allow_all,
            excludes,
            includes: Vec::new(),
            errors: ErrorResponder::new(exceptions, executor, dispatcher),
        }
    }

    pub fn is_ip_allowed(&self, client_ip_str: &str, req: &RequestMeta) -> bool {
        if let Some(hook) = &self.allowed_hook {
            return hook(client_ip_str, req).unwrap_or(false);
        }
        let Ok(client_ip) = client_ip_str.parse::<std::net::IpAddr>() else {
            return false;
        };

        for b_net in &self.blocked_networks {
            if b_net.contains(&client_ip) {
                return false;
            }
        }

        if let Some(cb) = &self.custom_validator {
            if let Ok(Some(ok)) = cb(client_ip_str, req) {
                return ok;
            }
        }

        if self.allow_all {
            return true;
        }

        if !self.allowed_networks.is_empty() {
            for a_net in &self.allowed_networks {
                if a_net.contains(&client_ip) {
                    return true;
                }
            }
            return false;
        }

        client_ip.is_loopback()
    }
}

impl Middleware for IpWhitelistMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        if skipped(req, &self.excludes, &self.includes) {
            return Decision::next();
        }

        let client_ip = if req.client_ip.is_empty() {
            "127.0.0.1"
        } else {
            req.client_ip
        };
        if !self.is_ip_allowed(client_ip, req) {
            return self.errors.reject(
                req,
                new_ip_access_denied("", client_ip, req.path, req.method),
            );
        }
        Decision::next()
    }
}

// --- G. Payload & Input Validation Middleware ---
#[derive(Clone)]
pub struct PayloadValidationMiddleware {
    pub max_body_size_bytes: i64,
    /// Refuses a non-empty body that is not declared as JSON.
    pub require_json_body: bool,
    pub required_headers: Vec<String>,
    pub required_fields: Vec<String>,
    pub required_fields_map: HashMap<String, Vec<String>>,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub errors: ErrorResponder,
}

impl PayloadValidationMiddleware {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_body_size_bytes: i64,
        required_headers: Vec<String>,
        required_fields: Vec<String>,
        required_fields_map: HashMap<String, Vec<String>>,
        excludes: Vec<String>,
        includes: Vec<String>,
        exceptions: Vec<MiddlewareExceptionConfig>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        Self {
            max_body_size_bytes,
            require_json_body: false,
            required_headers,
            required_fields,
            required_fields_map,
            excludes,
            includes,
            errors: ErrorResponder::new(exceptions, executor, dispatcher),
        }
    }
}

impl Middleware for PayloadValidationMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        let rm = RouteMatcher::new();
        let path = req.path;
        if rm.should_skip(path, &self.excludes, &self.includes) {
            return Decision::next();
        }

        for req_hdr in &self.required_headers {
            let trimmed = req_hdr.trim();
            if trimmed.is_empty() {
                continue;
            }
            if req.header(trimmed).is_empty() {
                return self.errors.reject(
                    req,
                    new_missing_required_field_exception(&format!(
                        "Missing required header '{}'",
                        trimmed
                    )),
                );
            }
        }

        let method = req.method.to_uppercase();
        if method != "POST" && method != "PUT" && method != "PATCH" {
            return Decision::next();
        }

        let body_len = req.body.len() as i64;
        if self.max_body_size_bytes > 0 && body_len > self.max_body_size_bytes {
            return self.errors.reject(
                req,
                new_payload_too_large_exception(
                    &format!(
                        "Payload size of {} bytes exceeds limit of {} bytes",
                        body_len, self.max_body_size_bytes
                    ),
                    self.max_body_size_bytes,
                    body_len,
                ),
            );
        }

        let ct = req.header("Content-Type").to_lowercase();
        let declared_json = ct.contains("application/json") || ct.contains("+json");
        let body_str = String::from_utf8_lossy(req.body);
        let trimmed = body_str.trim();
        if self.require_json_body && !declared_json && !trimmed.is_empty() {
            return self.errors.reject(
                req,
                new_bad_request_exception(
                    "Content-Type must be application/json for this endpoint",
                ),
            );
        }
        if trimmed.is_empty()
            || !(declared_json || trimmed.starts_with('{') || trimmed.starts_with('['))
        {
            return Decision::next();
        }

        let parsed = match json::parse(trimmed) {
            Ok(v) => v,
            Err(e) => {
                return self.errors.reject(
                    req,
                    new_malformed_json_exception(&format!("Invalid JSON payload: {}", e)),
                );
            }
        };

        let mut fields_to_validate = self.required_fields.clone();
        if !self.required_fields_map.is_empty() {
            if let Some(r_fields) = self.required_fields_map.get(path) {
                fields_to_validate.extend(r_fields.clone());
            } else {
                for (pat, r_fields) in &self.required_fields_map {
                    let clean_pat = pat.trim_start_matches('/');
                    if path.ends_with(clean_pat) || rm.matches(path, pat) {
                        fields_to_validate.extend(r_fields.clone());
                        break;
                    }
                }
            }
        }

        if let Some(parsed_map) = parsed.as_object() {
            for req_field in fields_to_validate {
                let rf = req_field.trim();
                if rf.is_empty() {
                    continue;
                }
                let missing = match parsed_map.get(rf) {
                    None | Some(Value::Null) => true,
                    Some(Value::String(s)) => s.is_empty(),
                    _ => false,
                };
                if missing {
                    return self.errors.reject(
                        req,
                        new_missing_required_field_exception(&format!(
                            "Missing required field '{}' in request payload",
                            rf
                        )),
                    );
                }
            }
        }

        Decision::next()
    }
}

// --- H. Response Audit Middleware (post-request) ---

#[derive(Clone)]
pub struct ResponseAuditMiddleware {
    pub activity: String,
    pub is_async: bool,
    pub excludes: Vec<String>,
    pub executor: Option<Arc<dyn IActivityExecutor>>,
    pub dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
}

impl ResponseAuditMiddleware {
    pub fn new(
        activity: &str,
        is_async: bool,
        excludes: Vec<String>,
        executor: Option<Arc<dyn IActivityExecutor>>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Self {
        Self {
            activity: activity.to_string(),
            is_async,
            excludes,
            executor,
            dispatcher,
        }
    }
}

impl Middleware for ResponseAuditMiddleware {
    fn before(&self, _req: &RequestMeta) -> Decision {
        Decision::next()
    }

    fn after(&self, req: &RequestMeta, resp: &ResponseMeta) {
        if self.activity.is_empty() || skipped(req, &self.excludes, &[]) {
            return;
        }
        let Some(exec) = &self.executor else {
            return;
        };
        let job_id = req.get("job_id").cloned().unwrap_or(Value::Null);
        let user_id = req.get("user_id").cloned().unwrap_or(Value::Null);
        let tier = req.get("customer_tier").cloned().unwrap_or(Value::Null);

        let audit_payload = vmap! {
            "job_id" => job_id.clone(),
            "route" => req.path,
            "method" => req.method,
            "status_code" => resp.status as i64,
            "duration_ms" => resp.duration_ms,
            "user_id" => user_id,
            "customer_tier" => tier,
        };

        if !self.is_async {
            let _ = exec.execute_activity(&self.activity, &audit_payload);
        } else if let Some(disp) = &self.dispatcher {
            let mut job_id_str = job_id.as_str().unwrap_or("").to_string();
            if job_id_str.is_empty() {
                job_id_str = format!("audit_{}", unix_nanos());
            }
            disp.dispatch(
                &self.activity,
                audit_payload,
                &job_id_str,
                "audit_log",
                Arc::clone(exec),
            );
        } else {
            let exec_clone = Arc::clone(exec);
            let act_name = self.activity.clone();
            crate::workers::spawn(move || {
                let _ = exec_clone.execute_activity(&act_name, &audit_payload);
            });
        }
    }
}

// --- I. Response Cache (Redis, with an in-process fallback) ---

/// The request-context key holding the cache key of a request that missed.
pub const CACHE_KEY_CONTEXT: &str = "octopus_cache_key";

#[derive(Clone)]
struct CacheItem {
    content: String,
    expiry: Instant,
}

pub struct RedisCacheMiddleware {
    pub ttl_seconds: u64,
    pub key_strategy: Vec<String>,
    pub methods: Vec<String>,
    pub header_name: String,
    pub prefix: String,
    pub cache_control_headers: bool,
    pub redis_url: String,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    /// The cache key and the bypass test from host code, in place of the
    /// built-in's; on an `Err` the key is the built-in's and nothing bypasses.
    pub key_hook: Option<RequestHook<String>>,
    pub bypass_hook: Option<RequestHook<bool>>,
    redis: RedisKv,
    /// Used while Redis is unreachable.
    fallback_cache: RwLock<HashMap<String, CacheItem>>,
}

impl RedisCacheMiddleware {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ttl_seconds: i32,
        key_strategy: Vec<String>,
        excludes: Vec<String>,
        includes: Vec<String>,
        methods: Vec<String>,
        header_name: &str,
        prefix: &str,
        cache_control_headers: bool,
        redis_url: &str,
    ) -> Self {
        let ttl = if ttl_seconds <= 0 {
            300
        } else {
            ttl_seconds as u64
        };
        let ks = if key_strategy.is_empty() {
            vec![
                "route".to_string(),
                "claim:user_id".to_string(),
                "query".to_string(),
            ]
        } else {
            key_strategy
        };
        let m_list = if methods.is_empty() {
            vec!["GET".to_string()]
        } else {
            methods.into_iter().map(|m| m.to_uppercase()).collect()
        };
        let hdr = if header_name.is_empty() {
            "X-Cache".to_string()
        } else {
            header_name.to_string()
        };
        let pfx = if prefix.is_empty() {
            "octopus:cache".to_string()
        } else {
            prefix.to_string()
        };
        let r_url = if redis_url.is_empty() {
            "127.0.0.1:6379".to_string()
        } else {
            redis_url.to_string()
        };

        Self {
            ttl_seconds: ttl,
            key_strategy: ks,
            methods: m_list,
            header_name: hdr,
            prefix: pfx,
            cache_control_headers,
            redis: RedisKv::new(&r_url),
            redis_url: r_url,
            excludes,
            includes,
            key_hook: None,
            bypass_hook: None,
            fallback_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn build_cache_key(&self, req: &RequestMeta) -> String {
        let mut sb = format!("{}:{}:{}", self.prefix, req.method.to_uppercase(), req.path);
        let as_text = |v: &Value| {
            v.as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_json_string())
        };
        for strategy in &self.key_strategy {
            let s_lower = strategy.to_lowercase();
            if s_lower.starts_with("claim:") {
                let claim_key = &strategy[6..];
                let mut claim_val = String::new();
                if let Some(v) = req.get(claim_key) {
                    if !v.is_null() {
                        claim_val = as_text(v);
                    }
                } else if let Some(Value::Object(claims)) = req.get("auth_claims") {
                    if let Some(v) = claims.get(claim_key) {
                        if !v.is_null() {
                            claim_val = as_text(v);
                        }
                    }
                } else if let Some(Value::Object(inj)) = req.get("workflow_injections") {
                    if let Some(v) = inj.get(claim_key) {
                        if !v.is_null() {
                            claim_val = as_text(v);
                        }
                    }
                }
                if !claim_val.is_empty() {
                    sb.push_str(&format!(":{}={}", claim_key, claim_val));
                }
            } else if s_lower == "query" {
                let mut keys: Vec<&String> = req
                    .query
                    .keys()
                    .filter(|k| {
                        !k.eq_ignore_ascii_case("no_cache")
                            && !k.eq_ignore_ascii_case("force_refresh")
                    })
                    .collect();
                if !keys.is_empty() {
                    keys.sort();
                    let parts: Vec<String> = keys
                        .into_iter()
                        .map(|k| format!("{}={}", k, req.query(k)))
                        .collect();
                    sb.push_str(":q=");
                    sb.push_str(&parts.join("&"));
                }
            } else if s_lower.starts_with("query:") {
                let q_key = &strategy[6..];
                let q_val = req.query(q_key);
                if !q_val.is_empty() {
                    sb.push_str(&format!(":{}={}", q_key, q_val));
                }
            } else if s_lower.starts_with("header:") {
                let hdr_key = &strategy[7..];
                let hdr_val = req.header(hdr_key);
                if !hdr_val.is_empty() {
                    sb.push_str(&format!(":{}={}", hdr_key, hdr_val));
                }
            }
        }
        sb
    }

    fn lookup(&self, cache_key: &str) -> Option<(String, Duration)> {
        match self.redis.get_with_ttl(cache_key) {
            Ok(hit) => hit,
            Err(_) => {
                let mut cache = self.fallback_cache.write().unwrap();
                let item = cache.get(cache_key)?;
                let now = Instant::now();
                if now < item.expiry {
                    Some((item.content.clone(), item.expiry.duration_since(now)))
                } else {
                    cache.remove(cache_key);
                    None
                }
            }
        }
    }
}

impl Middleware for RedisCacheMiddleware {
    fn before(&self, req: &RequestMeta) -> Decision {
        let method = req.method.to_uppercase();
        if !self.methods.iter().any(|mm| mm == &method)
            || skipped(req, &self.excludes, &self.includes)
        {
            return Decision::next();
        }

        if let Some(hook) = &self.bypass_hook {
            if hook(req).unwrap_or(false) {
                return Decision::next().header(&self.header_name, "BYPASS");
            }
        } else if self.cache_control_headers {
            let cc = req.header("Cache-Control").to_lowercase();
            let has_bypass =
                !req.query("no_cache").is_empty() || !req.query("force_refresh").is_empty();
            if cc.contains("no-cache") || cc.contains("no-store") || has_bypass {
                return Decision::next().header(&self.header_name, "BYPASS");
            }
        }

        let cache_key = self
            .key_hook
            .as_ref()
            .and_then(|hook| hook(req).ok())
            .unwrap_or_else(|| self.build_cache_key(req));
        match self.lookup(&cache_key).filter(|(c, _)| !c.is_empty()) {
            Some((content, remaining_ttl)) => {
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let mut d = Decision::next()
                    .header(&self.header_name, "HIT")
                    .header("X-Cache-Key", &cache_key)
                    .header("X-Cache-TTL", &remaining_ttl.as_secs().to_string())
                    .header("X-Job-ID", &format!("job_cache_hit_{}", millis))
                    .header("X-Execution-Status", "COMPLETED")
                    .set(CACHE_KEY_CONTEXT, Value::String(cache_key));
                d.respond = Some(Response {
                    status: 200,
                    content_type: JSON_CONTENT_TYPE.to_string(),
                    body: content.into_bytes(),
                });
                d
            }
            None => Decision::next()
                .header(&self.header_name, "MISS")
                .header("X-Cache-Key", &cache_key)
                .set(CACHE_KEY_CONTEXT, Value::String(cache_key)),
        }
    }

    fn after(&self, req: &RequestMeta, resp: &ResponseMeta) {
        let Some(cache_key) = req.get(CACHE_KEY_CONTEXT).and_then(|v| v.as_str()) else {
            return;
        };
        if resp.status != 200 || resp.body.is_empty() {
            return;
        }
        let body_str = String::from_utf8_lossy(resp.body).to_string();
        let ttl = Duration::from_secs(self.ttl_seconds);
        if self.redis.set_ex(cache_key, &body_str, ttl).is_ok() {
            return;
        }
        self.fallback_cache.write().unwrap().insert(
            cache_key.to_string(),
            CacheItem {
                content: body_str,
                expiry: Instant::now() + ttl,
            },
        );
    }
}
