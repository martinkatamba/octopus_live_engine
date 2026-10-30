//! Middleware: the errors they raise, route matching, and the built-in
//! middleware's decisions. A host reads each request from its framework into a
//! [`RequestMeta`], asks the middleware for a [`Decision`] and applies it.

mod builtins;
mod factory;
mod request;
#[cfg(test)]
mod tests;

pub use builtins::*;
pub use factory::build_middleware;
pub use request::{
    Decision, ErrorResponder, Middleware, RequestMeta, Response, ResponseMeta, JSON_CONTENT_TYPE,
};

use std::collections::BTreeMap;

use crate::json::{Value, ValueMap};
use crate::vmap;

// ============================================================================
// 1. Standard Built-in Middleware Error Hierarchy
// ============================================================================

#[derive(Debug, Clone)]
pub struct OctopusMiddlewareError {
    pub name: String,
    pub code: i32,
    pub message: String,
    pub details: ValueMap,
    pub headers: BTreeMap<String, String>,
}

impl std::fmt::Display for OctopusMiddlewareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}:{}] {}", self.name, self.code, self.message)
    }
}

impl std::error::Error for OctopusMiddlewareError {}

impl OctopusMiddlewareError {
    pub fn to_value(&self) -> Value {
        let mut m = vmap! {
            "error" => self.name.clone(),
            "status_code" => self.code as i64,
            "detail" => self.message.clone(),
        };
        if !self.details.is_empty() {
            m.insert("details".to_string(), Value::Object(self.details.clone()));
        }
        Value::Object(m)
    }

    pub fn parse_from_error_str(s: &str) -> Option<Self> {
        if let Some(rest) = s.strip_prefix('[') {
            if let Some(bracket_end) = rest.find(']') {
                let tag = &rest[..bracket_end];
                let msg = rest[bracket_end + 1..].trim().to_string();
                if let Some((name, code_str)) = tag.split_once(':') {
                    if let Ok(code) = code_str.parse::<i32>() {
                        return Some(Self {
                            name: name.to_string(),
                            code,
                            message: msg,
                            details: ValueMap::new(),
                            headers: BTreeMap::new(),
                        });
                    }
                }
            }
        }
        None
    }
}

pub fn new_user_not_authenticated(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "User is not authenticated or token is missing/invalid".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "UserNotAuthenticated".to_string(),
        code: 401,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_invalid_credentials(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "Invalid username or password. Please try again.".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "InvalidCredentials".to_string(),
        code: 401,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_user_not_authorized(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "User lacks authorization or required permissions".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "UserNotAuthorized".to_string(),
        code: 403,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_invalid_api_key_exception(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "Invalid or missing API key".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "InvalidApiKeyException".to_string(),
        code: 401,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_rate_limit_exceeded(
    msg: &str,
    limit: i32,
    window_seconds: i32,
    retry_after: i32,
    key: &str,
    headers: BTreeMap<String, String>,
) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        format!(
            "Rate limit exceeded. Quota: {} req/{}s. Retry in {}s.",
            limit, window_seconds, retry_after
        )
    } else {
        msg.to_string()
    };
    let details = vmap! {
        "limit" => limit as i64,
        "window_seconds" => window_seconds as i64,
        "retry_after_seconds" => retry_after as i64,
        "rate_limited_key" => key.to_string(),
    };
    OctopusMiddlewareError {
        name: "RateLimitExceeded".to_string(),
        code: 429,
        message,
        details,
        headers,
    }
}

pub fn new_ip_access_denied(
    msg: &str,
    client_ip: &str,
    path: &str,
    method: &str,
) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        format!(
            "Access denied: Client IP '{}' is not in the allowed IP whitelist",
            client_ip
        )
    } else {
        msg.to_string()
    };
    let details = vmap! {
        "client_ip" => client_ip.to_string(),
        "path" => path.to_string(),
        "method" => method.to_string(),
    };
    OctopusMiddlewareError {
        name: "IpAccessDenied".to_string(),
        code: 403,
        message,
        details,
        headers: BTreeMap::new(),
    }
}

pub fn new_bad_request_exception(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "Invalid or malformed request payload. Please verify syntax and parameters.".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "BadRequestException".to_string(),
        code: 400,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_malformed_json_exception(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "Malformed JSON syntax in request body.".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "MalformedJsonException".to_string(),
        code: 400,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_validation_exception(msg: &str) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        "Request validation rules failed.".to_string()
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "ValidationException".to_string(),
        code: 400,
        message,
        details: ValueMap::new(),
        headers: BTreeMap::new(),
    }
}

pub fn new_missing_required_field_exception(field_or_msg: &str) -> OctopusMiddlewareError {
    let message = if field_or_msg.is_empty() {
        "Missing required field in request payload.".to_string()
    } else if !field_or_msg.contains(' ') {
        format!(
            "Missing required field '{}' in request payload.",
            field_or_msg
        )
    } else {
        field_or_msg.to_string()
    };
    OctopusMiddlewareError {
        name: "MissingRequiredFieldException".to_string(),
        code: 400,
        message,
        details: vmap! { "field" => field_or_msg.to_string() },
        headers: BTreeMap::new(),
    }
}

pub fn new_payload_too_large_exception(
    msg: &str,
    max_size: i64,
    actual_size: i64,
) -> OctopusMiddlewareError {
    let message = if msg.is_empty() {
        format!(
            "Request payload size {} bytes exceeds maximum limit of {} bytes.",
            actual_size, max_size
        )
    } else {
        msg.to_string()
    };
    OctopusMiddlewareError {
        name: "PayloadTooLargeException".to_string(),
        code: 413,
        message,
        details: vmap! {
            "max_size_bytes" => max_size,
            "actual_size_bytes" => actual_size,
        },
        headers: BTreeMap::new(),
    }
}

// ============================================================================
// 2. Route Matching Utility
// ============================================================================

pub const SYSTEM_EXCLUDES: &[&str] = &["/favicon.ico", "/favicon.png", "/robots.txt", "/health*"];

#[derive(Debug, Default, Clone)]
pub struct RouteMatcher;

impl RouteMatcher {
    pub fn new() -> Self {
        Self
    }

    pub fn matches(&self, path: &str, pattern: &str) -> bool {
        if pattern.is_empty() || path.is_empty() {
            return false;
        }
        let clean_path = path.trim_end_matches('/');
        let clean_pat = pattern.trim_end_matches('/');

        if clean_path == clean_pat {
            return true;
        }

        if (pattern.contains('*') || pattern.contains('?'))
            && (glob_match(pattern, path) || glob_match(clean_pat, clean_path))
        {
            return true;
        }

        if let Some(prefix) = pattern.strip_suffix('*') {
            if path.starts_with(prefix) {
                return true;
            }
        }
        false
    }

    pub fn should_skip(&self, path: &str, excludes: &[String], includes: &[String]) -> bool {
        for sys_exc in SYSTEM_EXCLUDES {
            if self.matches(path, sys_exc) {
                return true;
            }
        }

        if !includes.is_empty() {
            let mut included = false;
            for inc in includes {
                if self.matches(path, inc) {
                    included = true;
                    break;
                }
            }
            if !included {
                return true;
            }
        }

        if !excludes.is_empty() {
            for exc in excludes {
                if self.matches(path, exc) {
                    return true;
                }
            }
        }
        false
    }
}

pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(si) = star_idx {
            pi = si + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
