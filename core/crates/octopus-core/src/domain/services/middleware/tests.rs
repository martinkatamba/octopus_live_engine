//! The built-in middleware's decisions, without any HTTP framework.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use super::*;
use crate::adapters::outbound::CallableActivityExecutor;
use crate::crypto::{base64url_encode, hmac_sha256};
use crate::domain::models::MiddlewareExceptionConfig;
use crate::domain::ports::IActivityExecutor;
use crate::json::{parse, Value, ValueMap};

/// An owned request a test can lend out as [`RequestMeta`].
#[derive(Default)]
struct Req {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    query: BTreeMap<String, Vec<String>>,
    body: Vec<u8>,
    context: ValueMap,
}

impl Req {
    fn new(method: &str, path: &str) -> Self {
        Req {
            method: method.into(),
            path: path.into(),
            ..Default::default()
        }
    }

    fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.insert(k.into(), v.into());
        self
    }

    fn body(mut self, b: &str) -> Self {
        self.body = b.as_bytes().to_vec();
        self
    }

    fn meta(&self) -> RequestMeta<'_> {
        RequestMeta {
            method: &self.method,
            path: &self.path,
            headers: &self.headers,
            query: &self.query,
            client_ip: "10.0.0.7",
            body: &self.body,
            context: &self.context,
        }
    }
}

fn status(d: &Decision) -> Option<u16> {
    d.respond.as_ref().map(|r| r.status)
}

fn body(d: &Decision) -> ValueMap {
    let text = std::str::from_utf8(&d.respond.as_ref().unwrap().body).unwrap();
    parse(text).unwrap().as_object().unwrap().clone()
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.as_object()
        .and_then(|m| m.get(key))
        .unwrap_or(&Value::Null)
}

#[test]
fn rate_limit_counts_per_client_and_answers_429() {
    let mw = RateLimitMiddleware::new(2, 60, "", HashMap::new(), vec![], vec![], None);
    let req = Req::new("GET", "/api/x");
    let first = mw.before(&req.meta());
    assert!(first.is_next());
    assert_eq!(first.headers["X-RateLimit-Remaining"], "1");
    assert!(mw.before(&req.meta()).is_next());
    let third = mw.before(&req.meta());
    assert_eq!(status(&third), Some(429));
    assert_eq!(third.headers["X-RateLimit-Remaining"], "0");
    assert!(third.headers.contains_key("Retry-After"));
    assert_eq!(
        body(&third)["error"],
        Value::String("RateLimitExceeded".into())
    );
}

#[test]
fn exception_override_changes_the_answer_and_runs_its_activity() {
    let executor = CallableActivityExecutor::new();
    let seen = Arc::new(Mutex::new(ValueMap::new()));
    let seen2 = seen.clone();
    executor.register_activity(
        "OnBadKey",
        Arc::new(move |input: &ValueMap| {
            *seen2.lock().unwrap() = input.clone();
            let mut out = ValueMap::new();
            out.insert("ticket".into(), Value::String("T-1".into()));
            Ok(out)
        }),
    );
    let exceptions = vec![MiddlewareExceptionConfig {
        exception: "InvalidApiKeyException".into(),
        code: 403,
        message: "Go away".into(),
        activity: "OnBadKey".into(),
        headers: BTreeMap::from([("X-Why".to_string(), "key".to_string())]),
        ..Default::default()
    }];
    let exec: Arc<dyn IActivityExecutor> = executor;
    let mw = ApiKeyAuthMiddleware::new(vec!["k1".into()], vec![], exceptions, Some(exec), None);

    assert!(mw
        .before(&Req::new("GET", "/a").header("x-api-key", "k1").meta())
        .is_next());
    let d = mw.before(&Req::new("GET", "/a").header("X-API-Key", "nope").meta());
    assert_eq!(status(&d), Some(403));
    assert_eq!(d.headers["X-Why"], "key");
    let b = body(&d);
    assert_eq!(b["detail"], Value::String("Go away".into()));
    assert_eq!(*field(&b["details"], "ticket"), Value::String("T-1".into()));
    assert_eq!(
        seen.lock().unwrap()["client_ip"],
        Value::String("10.0.0.7".into())
    );
}

#[test]
fn excluded_paths_are_not_checked() {
    let mw = ApiKeyAuthMiddleware::new(
        vec!["k1".into()],
        vec!["/health".into()],
        vec![],
        None,
        None,
    );
    assert!(mw.before(&Req::new("GET", "/health").meta()).is_next());
    assert_eq!(
        status(&mw.before(&Req::new("GET", "/api").meta())),
        Some(401)
    );
}

#[test]
fn bearer_tokens_put_their_claims_on_the_request() {
    let secret = "s3cret";
    let head = base64url_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let claims = base64url_encode(br#"{"sub":"u-1","customer_tier":"GOLD"}"#);
    let sig = base64url_encode(&hmac_sha256(
        secret.as_bytes(),
        format!("{}.{}", head, claims).as_bytes(),
    ));
    let token = format!("{}.{}.{}", head, claims, sig);
    let validator: TokenValidatorFunc = Arc::new(move |t: &str| verify_hs256_jwt(t, secret));
    let mw = BearerAuthAndClaimsMiddleware::new(Some(validator), None, vec![], vec![], None, None);

    let ok = mw.before(
        &Req::new("GET", "/me")
            .header("Authorization", &format!("Bearer {}", token))
            .meta(),
    );
    assert!(ok.is_next());
    assert_eq!(
        *field(&ok.context["auth_claims"], "sub"),
        Value::String("u-1".into())
    );
    assert_eq!(
        status(&mw.before(&Req::new("GET", "/me").meta())),
        Some(401)
    );
    let forged = format!("{}.{}.{}", head, claims, base64url_encode(b"wrong"));
    let bad = mw.before(
        &Req::new("GET", "/me")
            .header("Authorization", &format!("Bearer {}", forged))
            .meta(),
    );
    assert_eq!(status(&bad), Some(401));
}

#[test]
fn payload_validation_checks_size_json_and_required_fields() {
    let mw = PayloadValidationMiddleware::new(
        20,
        vec![],
        vec!["amount".into()],
        HashMap::new(),
        vec![],
        vec![],
        vec![],
        None,
        None,
    );
    let post = |b: &str| {
        mw.before(
            &Req::new("POST", "/pay")
                .header("Content-Type", "application/json")
                .body(b)
                .meta(),
        )
    };
    assert!(post(r#"{"amount": 5}"#).is_next());
    assert_eq!(status(&post(r#"{"amount": ""}"#)), Some(400));
    assert_eq!(status(&post(r#"{"amount": 5"#)), Some(400));
    assert_eq!(
        status(&post(r#"{"amount": 5, "note": "too long"}"#)),
        Some(413)
    );
    assert!(mw.before(&Req::new("GET", "/pay").meta()).is_next());
}

#[test]
fn ip_filter_blocks_before_allowing() {
    let mw = IpWhitelistMiddleware::new(
        vec!["10.0.0.0/8".into()],
        vec!["10.0.0.7".into()],
        None,
        vec![],
        vec![],
        None,
        None,
    );
    let d = mw.before(&Req::new("GET", "/x").meta());
    assert_eq!(status(&d), Some(403));
    assert_eq!(body(&d)["error"], Value::String("IpAccessDenied".into()));
}

#[test]
fn cache_stores_after_a_miss_and_answers_the_next_request() {
    // Nothing listens on port 1, so the in-process fallback is used.
    let mw = RedisCacheMiddleware::new(
        60,
        vec![],
        vec![],
        vec![],
        vec![],
        "",
        "",
        true,
        "127.0.0.1:1",
    );
    let mut req = Req::new("GET", "/items");
    let miss = mw.before(&req.meta());
    assert!(miss.is_next());
    assert_eq!(miss.headers["X-Cache"], "MISS");
    req.context = miss.context;
    let resp = ResponseMeta {
        status: 200,
        body: br#"{"items":[]}"#,
        duration_ms: 1.0,
    };
    mw.after(&req.meta(), &resp);

    let hit = mw.before(&Req::new("GET", "/items").meta());
    assert_eq!(hit.headers["X-Cache"], "HIT");
    assert_eq!(hit.respond.unwrap().body, br#"{"items":[]}"#.to_vec());
    let bypass = mw.before(
        &Req::new("GET", "/items")
            .header("Cache-Control", "no-cache")
            .meta(),
    );
    assert!(bypass.is_next());
    assert_eq!(bypass.headers["X-Cache"], "BYPASS");
}

#[test]
fn audit_runs_after_the_response() {
    let executor = CallableActivityExecutor::new();
    let seen = Arc::new(Mutex::new(ValueMap::new()));
    let seen2 = seen.clone();
    executor.register_activity(
        "Audit",
        Arc::new(move |input: &ValueMap| {
            *seen2.lock().unwrap() = input.clone();
            Ok(ValueMap::new())
        }),
    );
    let exec: Arc<dyn IActivityExecutor> = executor;
    let mw = ResponseAuditMiddleware::new("Audit", false, vec![], Some(exec), None);
    let mut req = Req::new("POST", "/orders");
    req.context
        .insert("job_id".into(), Value::String("j-1".into()));
    assert!(mw.before(&req.meta()).is_next());
    mw.after(
        &req.meta(),
        &ResponseMeta {
            status: 201,
            body: b"",
            duration_ms: 2.5,
        },
    );
    let got = seen.lock().unwrap();
    assert_eq!(got["status_code"], Value::Int(201));
    assert_eq!(got["job_id"], Value::String("j-1".into()));
    assert_eq!(got["route"], Value::String("/orders".into()));
}

#[test]
fn the_factory_knows_the_built_in_names() {
    let cfg = |name: &str| crate::domain::models::MiddlewareConfig {
        name: name.into(),
        ..Default::default()
    };
    for name in [
        "rate_limit",
        "ApiKeyAuth",
        "jwt-auth",
        "CorrelationId",
        "cache",
        "validation",
        "ip_filter",
    ] {
        assert!(
            build_middleware(&cfg(name), None, None).is_some(),
            "{}",
            name
        );
    }
    assert!(build_middleware(&cfg("NoSuchThing"), None, None).is_none());
}

fn built(name: &str, config: ValueMap) -> Arc<dyn Middleware> {
    let cfg = crate::domain::models::MiddlewareConfig {
        name: name.to_string(),
        config,
        ..Default::default()
    };
    build_middleware(&cfg, None, None).expect("a built-in")
}

#[test]
fn role_authorization_reads_claims_before_the_header() {
    let mw = built(
        "RoleAuthorizationMiddleware",
        crate::vmap! { "required_roles" => vec!["admin", "ops"] },
    );
    assert!(mw
        .before(
            &Req::new("GET", "/x")
                .header("X-User-Roles", "guest, ops")
                .meta()
        )
        .is_next());
    let denied = mw.before(&Req::new("GET", "/x").header("X-User-Roles", "guest").meta());
    assert_eq!(status(&denied), Some(403));
    assert_eq!(body(&denied)["error"], Value::from("UserNotAuthorized"));
    assert_eq!(
        body(&denied)["detail"],
        Value::from("User lacks required role(s): ['admin', 'ops']")
    );

    // A verified token's roles win over a header the caller could set.
    let mut req = Req::new("GET", "/x").header("X-User-Roles", "admin");
    req.context.insert(
        "auth_claims".into(),
        Value::Object(crate::vmap! { "role" => "guest" }),
    );
    assert_eq!(status(&mw.before(&req.meta())), Some(403));
}

#[test]
fn payload_validation_limits_bodies_to_10_mb_unless_told_otherwise() {
    let mw = built("PayloadValidationMiddleware", ValueMap::new());
    let big = "x".repeat(super::factory::DEFAULT_MAX_BODY_BYTES as usize + 1);
    assert_eq!(
        status(&mw.before(&Req::new("POST", "/x").body(&big).meta())),
        Some(413)
    );

    let strict = built(
        "PayloadValidationMiddleware",
        crate::vmap! { "require_json_body" => true },
    );
    let plain = Req::new("POST", "/x")
        .header("Content-Type", "text/plain")
        .body("hello");
    assert_eq!(status(&strict.before(&plain.meta())), Some(400));
    let json = Req::new("POST", "/x")
        .header("Content-Type", "application/vnd.api+json")
        .body("{}");
    assert!(strict.before(&json.meta()).is_next());
}

#[test]
fn cache_keys_can_name_one_query_parameter() {
    let mw = RedisCacheMiddleware::new(
        60,
        vec!["query:page".into()],
        vec![],
        vec![],
        vec![],
        "",
        "",
        true,
        "",
    );
    let mut req = Req::new("GET", "/items");
    req.query.insert("page".into(), vec!["2".into()]);
    req.query.insert("sort".into(), vec!["asc".into()]);
    assert_eq!(
        mw.build_cache_key(&req.meta()),
        "octopus:cache:GET:/items:page=2"
    );
}

#[test]
fn rate_limit_keys_use_the_username_and_tenant_claims() {
    let by_user = RateLimitMiddleware::new(1, 60, "user", HashMap::new(), vec![], vec![], None);
    let mut req = Req::new("GET", "/x");
    req.context.insert(
        "auth_claims".into(),
        Value::Object(crate::vmap! { "username" => "ann", "tenant_id" => "acme" }),
    );
    assert!(by_user.before(&req.meta()).is_next());
    let again = by_user.before(&req.meta());
    assert_eq!(
        body(&again)["details"].get("rate_limited_key"),
        Some(&Value::from("user:ann"))
    );

    let by_tenant = RateLimitMiddleware::new(1, 60, "tenant", HashMap::new(), vec![], vec![], None);
    assert!(by_tenant.before(&req.meta()).is_next());
    let again = by_tenant.before(&req.meta());
    assert_eq!(
        body(&again)["details"].get("rate_limited_key"),
        Some(&Value::from("tenant:acme"))
    );
}

fn hooked(config: &str, hooks: MiddlewareHooks) -> Arc<dyn Middleware> {
    let cfg = crate::domain::models::MiddlewareConfig::from_value(&parse(config).unwrap()).unwrap();
    build_middleware_with_hooks(&cfg, None, None, &hooks).unwrap()
}

#[test]
fn a_token_validator_given_in_code_wins_and_its_claims_are_mapped() {
    let hooks = MiddlewareHooks {
        token_validator: Some(Arc::new(|token: &str| {
            if token == "good" {
                Ok(parse(r#"{"sub": "u1", "tier": "gold"}"#)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone())
            } else {
                Err("unknown token".to_string())
            }
        })),
        claims_mapper: Some(Arc::new(|claims: &ValueMap| {
            let mut out = ValueMap::new();
            out.insert("customer".to_string(), claims["sub"].clone());
            Ok(out)
        })),
        ..Default::default()
    };
    let mw = hooked(
        r#"{"name": "BearerAuth", "config": {"jwt_secret": "s"}}"#,
        hooks,
    );
    let ok = mw.before(
        &Req::new("GET", "/x")
            .header("Authorization", "Bearer good")
            .meta(),
    );
    assert!(ok.is_next());
    assert_eq!(
        ok.context["workflow_injections"],
        parse(r#"{"customer": "u1"}"#).unwrap()
    );
    let bad = mw.before(
        &Req::new("GET", "/x")
            .header("Authorization", "Bearer bad")
            .meta(),
    );
    assert_eq!(status(&bad), Some(401));
    assert!(body(&bad)["detail"]
        .to_display_string()
        .contains("unknown token"));
}

#[test]
fn a_claims_mapper_error_refuses_as_not_authorized() {
    let hooks = MiddlewareHooks {
        token_validator: Some(Arc::new(|_: &str| Ok(ValueMap::new()))),
        claims_mapper: Some(Arc::new(|_: &ValueMap| Err("no tenant".to_string()))),
        ..Default::default()
    };
    let mw = hooked(r#"{"name": "BearerAuth"}"#, hooks);
    let d = mw.before(
        &Req::new("GET", "/x")
            .header("Authorization", "Bearer t")
            .meta(),
    );
    assert_eq!(status(&d), Some(403));
    assert!(body(&d)["detail"]
        .to_display_string()
        .contains("Claims mapping error: no tenant"));
}

#[test]
fn a_role_extractor_names_the_callers_roles() {
    let hooks = MiddlewareHooks {
        role_extractor: Some(Arc::new(|req: &RequestMeta| {
            match req.header("X-Staff").as_str() {
                "yes" => Ok(vec!["admin".to_string()]),
                "broken" => Err("directory down".to_string()),
                _ => Ok(vec![]),
            }
        })),
        ..Default::default()
    };
    let mw = hooked(
        r#"{"name": "RoleAuthorization", "config": {"required_roles": ["admin"]}}"#,
        hooks,
    );
    assert!(mw
        .before(&Req::new("GET", "/x").header("X-Staff", "yes").meta())
        .is_next());
    assert_eq!(status(&mw.before(&Req::new("GET", "/x").meta())), Some(403));
    let d = mw.before(&Req::new("GET", "/x").header("X-Staff", "broken").meta());
    assert!(body(&d)["detail"]
        .to_display_string()
        .contains("Role extraction failed: directory down"));
}

#[test]
fn rate_limit_hooks_set_the_key_and_the_limit() {
    let hooks = MiddlewareHooks {
        rate_limit_key: Some(Arc::new(|req: &RequestMeta| {
            Ok(format!("acct:{}", req.header("X-Account")))
        })),
        rate_limit: Some(Arc::new(|req: &RequestMeta| {
            if req.header("X-Account") == "big" {
                Ok(3)
            } else {
                Err("use the default".to_string())
            }
        })),
        ..Default::default()
    };
    let mw = hooked(r#"{"name": "RateLimit", "config": {"limit": 1}}"#, hooks);
    let big = Req::new("GET", "/x").header("X-Account", "big");
    for _ in 0..3 {
        assert!(mw.before(&big.meta()).is_next());
    }
    let refused = mw.before(&big.meta());
    assert_eq!(status(&refused), Some(429));
    assert!(body(&refused)
        .values()
        .any(|v| v.to_json_string().contains("acct:big")));
    // The limit hook failed: the configured limit (1), counted per account.
    let small = Req::new("GET", "/x").header("X-Account", "small");
    assert!(mw.before(&small.meta()).is_next());
    assert_eq!(status(&mw.before(&small.meta())), Some(429));
}

#[test]
fn ip_hooks_have_their_say_or_the_whole_answer() {
    let validator = MiddlewareHooks {
        ip_validator: Some(Arc::new(|ip: &str, _: &RequestMeta| {
            Ok(if ip == "10.0.0.7" { Some(true) } else { None })
        })),
        ..Default::default()
    };
    // Not in the allowed list, but the validator says yes.
    let mw = hooked(
        r#"{"name": "IpWhitelist", "config": {"allowed_ips": ["192.168.0.0/16"]}}"#,
        validator,
    );
    assert!(mw.before(&Req::new("GET", "/x").meta()).is_next());
    // The blocked list still comes first.
    let validator = MiddlewareHooks {
        ip_validator: Some(Arc::new(|_: &str, _: &RequestMeta| Ok(Some(true)))),
        ..Default::default()
    };
    let mw = hooked(
        r#"{"name": "IpWhitelist", "config": {"blocked_ips": ["10.0.0.0/8"]}}"#,
        validator,
    );
    assert_eq!(status(&mw.before(&Req::new("GET", "/x").meta())), Some(403));

    let decides = MiddlewareHooks {
        ip_allowed: Some(Arc::new(|_: &str, req: &RequestMeta| {
            Ok(req.header("X-Pass") == "1")
        })),
        ..Default::default()
    };
    let mw = hooked(
        r#"{"name": "IpWhitelist", "config": {"blocked_ips": ["10.0.0.0/8"]}}"#,
        decides,
    );
    assert!(mw
        .before(&Req::new("GET", "/x").header("X-Pass", "1").meta())
        .is_next());
    assert_eq!(status(&mw.before(&Req::new("GET", "/x").meta())), Some(403));
}

#[test]
fn cache_hooks_set_the_key_and_the_bypass() {
    let hooks = MiddlewareHooks {
        cache_key: Some(Arc::new(|req: &RequestMeta| {
            Ok(format!("mine:{}", req.path))
        })),
        cache_bypass: Some(Arc::new(|req: &RequestMeta| {
            Ok(req.header("X-Fresh") == "1")
        })),
        ..Default::default()
    };
    let mw = hooked(
        r#"{"name": "RedisCache", "config": {"methods": ["GET"], "redis_url": "127.0.0.1:1"}}"#,
        hooks,
    );
    let miss = mw.before(&Req::new("GET", "/items").meta());
    assert_eq!(miss.headers["X-Cache-Key"], "mine:/items");
    let bypass = mw.before(
        &Req::new("GET", "/items")
            .header("X-Fresh", "1")
            .header("Cache-Control", "max-age=60")
            .meta(),
    );
    assert_eq!(bypass.headers["X-Cache"], "BYPASS");
    // With a bypass hook, Cache-Control: no-cache is the hook's business.
    let kept = mw.before(
        &Req::new("GET", "/items")
            .header("Cache-Control", "no-cache")
            .meta(),
    );
    assert_eq!(kept.headers["X-Cache"], "MISS");
}
