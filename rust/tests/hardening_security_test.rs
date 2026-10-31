use std::sync::Arc;

use octopus_live_engine::domain::models::{
    APIEndpointConfig, EngineOptions, ToolingAccess, WorkflowBlueprint, WorkflowStep,
};
use octopus_live_engine::{
    vmap, HandlerFunc, HttpContext, HttpRequest, HttpResponse, OctopusLiveEngine, Router, Value,
    ValueMap,
};

fn new_security_engine(
    api_key: &str,
    opts: EngineOptions,
    middlewares: Vec<HandlerFunc>,
) -> Router {
    let mut engine = OctopusLiveEngine::new(api_key);
    engine.set_options(opts);
    engine.register_activity("Me", |in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! {
            "user_id" => in_map.get("user_id").cloned().unwrap_or(Value::Null),
            "claims" => in_map.get("_auth_claims").cloned().unwrap_or(Value::Null),
        })
    });
    engine.register_activity("Boom", |_in_map: &ValueMap| -> Result<ValueMap, String> {
        Err("pq: connection to db-prod-7 failed, password=hunter2".to_string())
    });
    engine.register_activity("Echo", |in_map: &ValueMap| -> Result<ValueMap, String> {
        Ok(vmap! {
            "value" => in_map.get("value").cloned().unwrap_or(Value::Null),
        })
    });

    engine
        .register_blueprint(WorkflowBlueprint {
            workflow_name: "acct".to_string(),
            version: "1.0.0".to_string(),
            api: vec![
                APIEndpointConfig {
                    route: "/api/v1.0.0/acct/me".to_string(),
                    method: "GET".to_string(),
                    steps: vec![WorkflowStep {
                        id: "me".to_string(),
                        activity: "Me".to_string(),
                        is_output: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                APIEndpointConfig {
                    route: "/api/v1.0.0/acct/fail".to_string(),
                    method: "POST".to_string(),
                    steps: vec![WorkflowStep {
                        id: "boom".to_string(),
                        activity: "Boom".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                APIEndpointConfig {
                    route: "/api/v1.0.0/acct/echo".to_string(),
                    method: "POST".to_string(),
                    steps: vec![WorkflowStep {
                        id: "echo".to_string(),
                        activity: "Echo".to_string(),
                        is_output: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .unwrap();

    for mw in middlewares {
        engine.add_middleware(mw);
    }

    let mut router = Router::new();
    engine.mount_routes(&mut router);
    router
}

fn do_req(
    r: &Router,
    method: &str,
    path: &str,
    body: &str,
    remote: &str,
    headers: &[(&str, &str)],
) -> HttpResponse {
    let mut req = HttpRequest::new(method, path);
    if !remote.is_empty() {
        req = req.with_remote_addr(remote);
    }
    if !body.is_empty() {
        req = req
            .with_header("Content-Type", "application/json")
            .with_body(body.as_bytes().to_vec());
    }
    for (k, v) in headers {
        req = req.with_header(k, v);
    }
    r.serve_http(req)
}

fn inject_user(user: &str) -> HandlerFunc {
    let u = user.to_string();
    Arc::new(move |c: &mut HttpContext| {
        c.set(
            "workflow_injections",
            Value::Object(vmap! { "user_id" => u.clone() }),
        );
        c.next();
    })
}

#[test]
fn test_injected_identity_cannot_be_overridden() {
    let r = new_security_engine("", EngineOptions::default(), vec![inject_user("alice")]);
    let w = do_req(
        &r,
        "GET",
        "/api/v1.0.0/acct/me?user_id=mallory",
        "",
        "",
        &[],
    );
    let out = w.json().unwrap();
    assert_eq!(out.get("user_id").and_then(|v| v.as_str()), Some("alice"));

    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let w = do_req(
        &r,
        "GET",
        "/api/v1.0.0/acct/me?_auth_claims=admin",
        "",
        "",
        &[],
    );
    let out = w.json().unwrap();
    assert!(
        out.get("claims").map(|v| v.is_null()).unwrap_or(true),
        "client-supplied _auth_claims reached the activity: {:?}",
        out.get("claims")
    );
}

#[test]
fn test_no_backdoor_api_keys() {
    let r = new_security_engine("real-key", EngineOptions::default(), Vec::new());
    for k in ["katz-secret-key-12345", "secret-key", "default-api-key"] {
        let w = do_req(
            &r,
            "POST",
            "/api/v1.0.0/acct/echo",
            r#"{"value":1}"#,
            "",
            &[("X-API-Key", k)],
        );
        assert_eq!(w.code, 401);
    }
    let w = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo?api_key=real-key",
        r#"{"value":1}"#,
        "",
        &[],
    );
    assert_eq!(w.code, 401);

    let w = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value":1}"#,
        "",
        &[("X-API-Key", "real-key")],
    );
    assert_eq!(w.code, 200);

    let r = new_security_engine(
        "real-key",
        EngineOptions {
            allow_api_key_in_query: true,
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo?api_key=real-key",
        r#"{"value":1}"#,
        "",
        &[],
    );
    assert_eq!(w.code, 200);
}

#[test]
fn test_tooling_access() {
    let tooling = [
        "/api/jobs",
        "/api/visualizer/jobs",
        "/api/visualizer/domains",
        "/visualizer",
        "/docs",
        "/openapi.json",
        "/api/v1.0.0/acct/jobs",
    ];
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    for p in tooling {
        let w = do_req(&r, "GET", p, "", "203.0.113.5:4000", &[]);
        assert_eq!(w.code, 403, "remote {}", p);
        let w = do_req(&r, "GET", p, "", "127.0.0.1:4000", &[]);
        assert_eq!(w.code, 200, "local {}", p);
    }

    let r = new_security_engine("k", EngineOptions::default(), Vec::new());
    let w = do_req(
        &r,
        "GET",
        "/api/jobs",
        "",
        "203.0.113.5:4000",
        &[("X-API-Key", "k")],
    );
    assert_eq!(w.code, 200);
    let w = do_req(&r, "GET", "/api/jobs", "", "127.0.0.1:4000", &[]);
    assert_eq!(w.code, 401);

    let r = new_security_engine(
        "",
        EngineOptions {
            tooling: ToolingAccess::Open,
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(&r, "GET", "/api/jobs", "", "203.0.113.5:4000", &[]);
    assert_eq!(w.code, 200);

    let r = new_security_engine(
        "",
        EngineOptions {
            tooling: ToolingAccess::Disabled,
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(&r, "GET", "/visualizer", "", "127.0.0.1:4000", &[]);
    assert_eq!(w.code, 404);
}

#[test]
fn test_middleware_can_protect_visualizer() {
    let mw: HandlerFunc = Arc::new(|c: &mut HttpContext| {
        if c.get_header("Authorization").is_empty() {
            c.abort_with_status_json(
                401,
                Value::Object(vmap! { "error" => "UserNotAuthenticated" }),
            );
            return;
        }
        c.next();
    });
    let r = new_security_engine("", EngineOptions::default(), vec![mw]);
    let w = do_req(&r, "GET", "/visualizer", "", "127.0.0.1:4000", &[]);
    assert_eq!(w.code, 401);
}

#[test]
fn test_client_job_ids_ignored_by_default() {
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let w1 = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value":"first"}"#,
        "",
        &[("X-Job-ID", "shared-id")],
    );
    let w2 = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value":"second"}"#,
        "",
        &[("X-Job-ID", "shared-id")],
    );
    let id = w1.header("X-Job-ID");
    assert!(id != "shared-id" && id.starts_with("job_"));
    assert!(w2.body_string().contains("second"));

    let r = new_security_engine(
        "",
        EngineOptions {
            allow_client_job_ids: true,
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value":"x"}"#,
        "",
        &[("X-Job-ID", "tx-42")],
    );
    assert_eq!(w.header("X-Job-ID"), "tx-42");

    let w_me = do_req(
        &r,
        "GET",
        "/api/v1.0.0/acct/me",
        "",
        "",
        &[("X-Job-ID", "tx-42")],
    );
    assert!(!w_me.body_string().contains(r#""value""#));

    let w_unsafe = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value":"x"}"#,
        "",
        &[("X-Job-ID", "<script>")],
    );
    assert_ne!(w_unsafe.header("X-Job-ID"), "<script>");
}

#[test]
fn test_forwarded_for_not_trusted_by_default() {
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let w = do_req(
        &r,
        "GET",
        "/api/jobs",
        "",
        "203.0.113.5:4000",
        &[("X-Forwarded-For", "127.0.0.1")],
    );
    assert_eq!(w.code, 403);

    let r = new_security_engine(
        "",
        EngineOptions {
            trusted_proxies: vec!["10.0.0.0/8".to_string()],
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(
        &r,
        "GET",
        "/api/jobs",
        "",
        "10.1.2.3:4000",
        &[("X-Forwarded-For", "127.0.0.1")],
    );
    assert_eq!(w.code, 200);
}

#[test]
fn test_internal_errors_not_leaked() {
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let w = do_req(&r, "POST", "/api/v1.0.0/acct/fail", "{}", "", &[]);
    assert_eq!(w.code, 500);
    assert!(!w.body_string().contains("hunter2"));
    assert!(!w.body_string().contains("db-prod"));
    assert!(w.body_string().contains("job_"));

    let r = new_security_engine(
        "",
        EngineOptions {
            expose_error_details: true,
            ..Default::default()
        },
        Vec::new(),
    );
    let w = do_req(&r, "POST", "/api/v1.0.0/acct/fail", "{}", "", &[]);
    assert!(w.body_string().contains("db-prod"));
}

#[test]
fn test_malformed_json_rejected() {
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let w = do_req(
        &r,
        "POST",
        "/api/v1.0.0/acct/echo",
        r#"{"value": "#,
        "",
        &[],
    );
    assert_eq!(w.code, 400);

    let w_empty = do_req(&r, "POST", "/api/v1.0.0/acct/echo", "", "", &[]);
    assert_eq!(w_empty.code, 200);
}

#[test]
fn test_visualizer_does_not_build_script_from_data() {
    let r = new_security_engine("", EngineOptions::default(), Vec::new());
    let html = do_req(&r, "GET", "/visualizer", "", "127.0.0.1:4000", &[]).body_string();
    for bad in [
        r#"onclick="selectJobFromModal(\'' + jId"#,
        "katz-secret-key",
        "urlParams.get('api_key')",
        r#"'"</strong> Not Found'"#,
    ] {
        assert!(!html.contains(bad), "visualizer still contains {:?}", bad);
    }
    for good in ["data-inspect-job=", "escapeHtml(targetJobId)"] {
        assert!(html.contains(good), "visualizer missing {:?}", good);
    }
}
