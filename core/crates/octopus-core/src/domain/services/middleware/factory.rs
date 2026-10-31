//! Builds the built-in middleware a blueprint's `middlewares:` list names.

use std::collections::HashMap;
use std::sync::Arc;

use super::builtins::*;
use super::request::Middleware;
use crate::domain::models::MiddlewareConfig;
use crate::domain::ports::{IActivityExecutor, IAsyncActivityDispatcher};
use crate::json::{Value, ValueMap};

/// Payload validation's body limit when the blueprint sets none (10 MB).
pub const DEFAULT_MAX_BODY_BYTES: i32 = 10 * 1024 * 1024;

/// Host code that stands in for part of a built-in's check: a function an
/// SDK user gave in code, or a method a subclass overrides. The built-in
/// keeps the rest (the refusal, its headers, `exceptions:`).
#[derive(Clone, Default)]
pub struct MiddlewareHooks {
    /// Bearer auth: wins over `jwt_secret`.
    pub token_validator: Option<TokenValidatorFunc>,
    pub claims_mapper: Option<ClaimsMapperFunc>,
    /// Role authorization: the caller's roles.
    pub role_extractor: Option<RequestHook<Vec<String>>>,
    /// Rate limiting: the key a request counts against, and its limit.
    pub rate_limit_key: Option<RequestHook<String>>,
    pub rate_limit: Option<RequestHook<i32>>,
    /// IP filtering: a say after the blocked list, or the whole answer.
    pub ip_validator: Option<IpValidatorCallback>,
    pub ip_allowed: Option<IpDecisionHook>,
    /// Response cache: the key, and whether to bypass the cache.
    pub cache_key: Option<RequestHook<String>>,
    pub cache_bypass: Option<RequestHook<bool>>,
}

/// The built-in middleware `cfg` names, or `None` for a name the core does not
/// know (hosts check their own custom middleware first).
pub fn build_middleware(
    cfg: &MiddlewareConfig,
    executor: Option<Arc<dyn IActivityExecutor>>,
    dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
) -> Option<Arc<dyn Middleware>> {
    build_middleware_with_hooks(cfg, executor, dispatcher, &MiddlewareHooks::default())
}

/// `build_middleware`, with host code in place of parts of the check.
pub fn build_middleware_with_hooks(
    cfg: &MiddlewareConfig,
    executor: Option<Arc<dyn IActivityExecutor>>,
    dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    hooks: &MiddlewareHooks,
) -> Option<Arc<dyn Middleware>> {
    let norm_name = cfg.name.trim().to_lowercase().replace(['_', '-'], "");

    match norm_name.as_str() {
        "bearerauthandclaimsmiddleware"
        | "bearerauth"
        | "bearerauthandclaims"
        | "jwtauthmiddleware"
        | "jwtauth" => {
            let env_secret = std::env::var("OCTOPUS_JWT_SECRET").unwrap_or_default();
            let secret = get_string_from_map(&cfg.config, "jwt_secret", &env_secret);
            let validator: Option<TokenValidatorFunc> = if let Some(v) = &hooks.token_validator {
                Some(v.clone())
            } else if !secret.is_empty() {
                Some(Arc::new(move |token: &str| {
                    verify_hs256_jwt(token, &secret)
                }))
            } else {
                None
            };
            let mut mw = BearerAuthAndClaimsMiddleware::new(
                validator,
                hooks.claims_mapper.clone(),
                cfg.excludes.clone(),
                cfg.exceptions.clone(),
                executor,
                dispatcher,
            );
            mw.token_header = get_string_from_map(&cfg.config, "token_header", "Authorization");
            mw.includes = cfg.includes.clone();
            Some(Arc::new(mw))
        }
        "ratelimitmiddleware" | "ratelimit" | "ratelimiter" => {
            let def_lim = get_int_from_map(&cfg.config, "default_limit", 60);
            let limit = get_int_from_map(&cfg.config, "limit", def_lim);
            let def_win = get_int_from_map(&cfg.config, "window", 60);
            let window = get_int_from_map(&cfg.config, "window_seconds", def_win);
            let key_by = get_string_from_map(&cfg.config, "key_by", "client_ip");
            let tier_claim = get_string_from_map(&cfg.config, "tier_claim", "customer_tier");
            let tier_header = get_string_from_map(&cfg.config, "tier_header", "X-User-Tier");
            let tier_limits = get_map_string_int_from_map(&cfg.config, "tier_limits");

            let mut mw = RateLimitMiddleware::new(
                limit,
                window,
                &key_by,
                tier_limits,
                cfg.excludes.clone(),
                cfg.exceptions.clone(),
                executor,
            );
            mw.tier_claim = tier_claim;
            mw.tier_header = tier_header;
            mw.include_headers = get_bool_from_map(&cfg.config, "include_headers", true);
            mw.key_hook = hooks.rate_limit_key.clone();
            mw.limit_hook = hooks.rate_limit.clone();
            mw.includes = cfg.includes.clone();
            mw.errors.dispatcher = dispatcher;
            Some(Arc::new(mw))
        }
        "ipwhitelistmiddleware" | "ipwhitelist" | "ipfilter" | "ipfiltermiddleware" => {
            let def_allowed = get_string_slice_from_map(&cfg.config, "whitelist", Vec::new());
            let allowed = get_string_slice_from_map(&cfg.config, "allowed_ips", def_allowed);
            let def_blocked = get_string_slice_from_map(&cfg.config, "blacklist", Vec::new());
            let blocked = get_string_slice_from_map(&cfg.config, "blocked_ips", def_blocked);
            let mut mw = IpWhitelistMiddleware::new(
                allowed,
                blocked,
                hooks.ip_validator.clone(),
                cfg.excludes.clone(),
                cfg.exceptions.clone(),
                executor,
                dispatcher,
            );
            mw.includes = cfg.includes.clone();
            mw.allowed_hook = hooks.ip_allowed.clone();
            Some(Arc::new(mw))
        }
        "roleauthorizationmiddleware" | "roleauthorization" | "rbacmiddleware" | "rbac" => {
            let roles = get_string_slice_from_map(&cfg.config, "required_roles", Vec::new());
            let hdr = get_string_from_map(&cfg.config, "roles_header", "X-User-Roles");
            let mut mw = RoleAuthorizationMiddleware::new(
                roles,
                &hdr,
                cfg.excludes.clone(),
                cfg.exceptions.clone(),
                executor,
                dispatcher,
            );
            mw.includes = cfg.includes.clone();
            mw.role_extractor = hooks.role_extractor.clone();
            Some(Arc::new(mw))
        }
        "correlationidmiddleware" | "correlationid" | "requestidmiddleware" | "requestid" => {
            let hdr = get_string_from_map(&cfg.config, "header_name", "X-Correlation-ID");
            let mut mw = CorrelationIdMiddleware::new(&hdr, cfg.excludes.clone());
            mw.includes = cfg.includes.clone();
            Some(Arc::new(mw))
        }
        "apikeyauthmiddleware" | "apikeyauth" | "apikey" | "apikeyauthmw" => {
            let def_keys = get_string_slice_from_map(&cfg.config, "valid_keys", Vec::new());
            let keys = get_string_slice_from_map(&cfg.config, "api_keys", def_keys);
            let mut mw = ApiKeyAuthMiddleware::new(
                keys,
                cfg.excludes.clone(),
                cfg.exceptions.clone(),
                executor,
                dispatcher,
            );
            mw.header_name = get_string_from_map(&cfg.config, "header_name", "X-API-Key");
            mw.includes = cfg.includes.clone();
            Some(Arc::new(mw))
        }
        "rediscachemiddleware" | "rediscache" | "cachemiddleware" | "cache" => {
            let def_ttl = get_int_from_map(&cfg.config, "ttl", 300);
            let ttl = get_int_from_map(&cfg.config, "ttl_seconds", def_ttl);
            let key_strat = get_string_slice_from_map(&cfg.config, "key_strategy", Vec::new());
            let methods = get_string_slice_from_map(&cfg.config, "methods", Vec::new());
            let hdr_name = get_string_from_map(&cfg.config, "header_name", "X-Cache");
            let pfx = get_string_from_map(&cfg.config, "prefix", "octopus:cache");
            let cc_hdr = get_bool_from_map(&cfg.config, "cache_control_headers", true);
            let def_dsn = get_string_from_map(&cfg.config, "dsn", "127.0.0.1:6379");
            let r_url = get_string_from_map(&cfg.config, "redis_url", &def_dsn);
            let mut mw = RedisCacheMiddleware::new(
                ttl,
                key_strat,
                cfg.excludes.clone(),
                cfg.includes.clone(),
                methods,
                &hdr_name,
                &pfx,
                cc_hdr,
                &r_url,
            );
            mw.key_hook = hooks.cache_key.clone();
            mw.bypass_hook = hooks.cache_bypass.clone();
            Some(Arc::new(mw))
        }
        "payloadvalidationmiddleware"
        | "payloadvalidation"
        | "badrequestmiddleware"
        | "badrequest"
        | "validationmiddleware"
        | "validation"
        | "inputvalidationmiddleware"
        | "inputvalidation" => {
            let def_max = get_int_from_map(&cfg.config, "max_size", DEFAULT_MAX_BODY_BYTES);
            let max_size = get_int_from_map(&cfg.config, "max_body_size_bytes", def_max) as i64;
            let req_headers =
                get_string_slice_from_map(&cfg.config, "required_headers", Vec::new());
            let (req_fields_map, req_fields) =
                get_fields_map_from_config(&cfg.config, "required_fields");
            let mut mw = PayloadValidationMiddleware::new(
                max_size,
                req_headers,
                req_fields,
                req_fields_map,
                cfg.excludes.clone(),
                cfg.includes.clone(),
                cfg.exceptions.clone(),
                executor,
                dispatcher,
            );
            mw.require_json_body = get_bool_from_map(&cfg.config, "require_json_body", false);
            Some(Arc::new(mw))
        }
        "responseauditmiddleware" | "responseaudit" | "auditlogmiddleware" | "auditmiddleware" => {
            Some(Arc::new(ResponseAuditMiddleware::new(
                &cfg.activity,
                cfg.is_async,
                cfg.excludes.clone(),
                executor,
                dispatcher,
            )))
        }
        _ => {
            if cfg.r#type.eq_ignore_ascii_case("post-request") && !cfg.activity.is_empty() {
                Some(Arc::new(ResponseAuditMiddleware::new(
                    &cfg.activity,
                    cfg.is_async,
                    cfg.excludes.clone(),
                    executor,
                    dispatcher,
                )))
            } else {
                None
            }
        }
    }
}

fn get_int_from_map(m: &ValueMap, key: &str, def: i32) -> i32 {
    m.get(key)
        .and_then(|v| v.as_i64())
        .map(|n| n as i32)
        .unwrap_or(def)
}

fn get_string_from_map(m: &ValueMap, key: &str, def: &str) -> String {
    m.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| def.to_string())
}

fn get_string_slice_from_map(m: &ValueMap, key: &str, def: Vec<String>) -> Vec<String> {
    if let Some(Value::Array(arr)) = m.get(key) {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    } else {
        def
    }
}

fn get_map_string_int_from_map(m: &ValueMap, key: &str) -> HashMap<String, i32> {
    let mut res = HashMap::new();
    if let Some(Value::Object(obj)) = m.get(key) {
        for (k, v) in obj {
            if let Some(n) = v.as_i64() {
                res.insert(k.clone(), n as i32);
            }
        }
    }
    res
}

fn get_bool_from_map(m: &ValueMap, key: &str, def: bool) -> bool {
    match m.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true") || s == "1",
        _ => def,
    }
}

fn get_fields_map_from_config(
    cfg: &ValueMap,
    key: &str,
) -> (HashMap<String, Vec<String>>, Vec<String>) {
    match cfg.get(key) {
        Some(Value::Array(arr)) => {
            let list: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            (HashMap::new(), list)
        }
        Some(Value::Object(obj)) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                if let Value::Array(arr) = v {
                    let list: Vec<String> = arr
                        .iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect();
                    map.insert(k.clone(), list);
                }
            }
            (map, Vec::new())
        }
        _ => (HashMap::new(), Vec::new()),
    }
}
