use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use crate::adapters::inbound::visualizer_html::render_visualizer_html;
use crate::crypto::constant_time_eq;
use crate::domain::models::{EngineOptions, ExecutionState, ExecutionStatus, WorkflowBlueprint};
use crate::domain::ports::{IVisualizerCompiler, IWorkflowEngine};
use crate::domain::services::{
    apply_injections, choose_job_id, decide_tooling_access, failure_body, public_data,
    run_declared_route, DeclaredRoute, RouteMatcher, RouteRequest, ToolingDecision,
    ERR_NO_SUSPENSION, ERR_SIGNAL_MISMATCH,
};
use crate::http::{HandlerFunc, HttpContext, Router, SseEventMessage};
use crate::json::{Value, ValueMap};
use crate::vmap;

pub type ToolingAuthorizer = Arc<dyn Fn(&mut HttpContext) -> bool + Send + Sync>;

#[derive(Clone, Default)]
pub struct RouteConfig {
    pub api_key: String,
    pub options: EngineOptions,
    pub tooling_authorizer: Option<ToolingAuthorizer>,
    pub registered_models: HashMap<String, Value>,
}

pub fn register_live_routes(
    router: &mut Router,
    workflow_use_case: Arc<dyn IWorkflowEngine>,
    visualizer_use_case: Arc<dyn IVisualizerCompiler>,
    api_key: &str,
    registered_models: HashMap<String, Value>,
) {
    register_live_routes_with_config(
        router,
        workflow_use_case,
        visualizer_use_case,
        RouteConfig {
            api_key: api_key.to_string(),
            options: EngineOptions::default(),
            tooling_authorizer: None,
            registered_models,
        },
    );
}

fn check_auth(c: &mut HttpContext, api_key: &str, opts: &EngineOptions) -> bool {
    if api_key.is_empty() {
        return true;
    }
    let mut req_key = c.get_header("X-API-Key");
    if req_key.is_empty() && opts.allow_api_key_in_query {
        req_key = c.query("api_key");
    }
    if !constant_time_eq(req_key.as_bytes(), api_key.as_bytes()) {
        c.abort_with_status_json(
            401,
            Value::Object(vmap! {
                "error" => "Unauthorized API Key",
            }),
        );
        return false;
    }
    true
}

fn job_id_candidates(c: &HttpContext, payload: &ValueMap) -> Vec<String> {
    let mut candidates = vec![
        c.get_header("X-Job-ID"),
        c.get_header("X-Request-ID"),
        c.get_header("X-Correlation-ID"),
    ];
    if let Some(j) = payload.get("job_id").and_then(|v| v.as_str()) {
        candidates.push(j.to_string());
    }
    candidates.push(c.query("job_id"));
    candidates
}

fn extract_or_create_job_id(c: &HttpContext, payload: &ValueMap, opts: &EngineOptions) -> String {
    choose_job_id(opts, &job_id_candidates(c, payload))
}

fn extract_injections(c: &HttpContext, payload: &mut ValueMap) {
    apply_injections(payload, c.get("workflow_injections"), c.get("auth_claims"));
}

fn bind_json(c: &mut HttpContext) -> Option<ValueMap> {
    if c.body.is_empty() || c.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Some(ValueMap::new());
    }
    let body_str = match std::str::from_utf8(&c.body) {
        Ok(s) => s,
        Err(_) => {
            if !c.content_type().is_empty() && !c.content_type().to_lowercase().contains("json") {
                return Some(ValueMap::new());
            }
            c.abort_with_status_json(
                400,
                Value::Object(vmap! {
                    "error" => "MalformedJsonException",
                    "detail" => "Malformed JSON syntax in request body.",
                    "status_code" => 400i64,
                }),
            );
            return None;
        }
    };
    match Value::parse_json(body_str) {
        Ok(Value::Object(map)) => Some(map),
        Ok(Value::Null) => Some(ValueMap::new()),
        _ => {
            if !c.content_type().is_empty() && !c.content_type().to_lowercase().contains("json") {
                return Some(ValueMap::new());
            }
            c.abort_with_status_json(
                400,
                Value::Object(vmap! {
                    "error" => "MalformedJsonException",
                    "detail" => "Malformed JSON syntax in request body.",
                    "status_code" => 400i64,
                }),
            );
            None
        }
    }
}

fn fail_response(
    c: &mut HttpContext,
    state: Option<&ExecutionState>,
    err: Option<&str>,
    opts: &EngineOptions,
) {
    let (code, body) = failure_body(state, err, opts);
    c.json(code, body);
}

fn format_job_item(s: &ExecutionState) -> Value {
    let status_str = s.status.as_str().to_string();
    let is_completed = s.status == ExecutionStatus::Completed;
    let is_failed = s.status == ExecutionStatus::Failed || s.status == ExecutionStatus::RolledBack;
    let status_code: i64 = if is_failed {
        s.error_code
            .filter(|&c| c > 0)
            .map(|c| c as i64)
            .unwrap_or(500)
    } else if !is_completed {
        0
    } else {
        200
    };

    let client_host = if !s.client_host.is_empty() {
        s.client_host.clone()
    } else if let Some(ch) = s
        .data
        .get("client_host")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
    {
        ch.to_string()
    } else if let Some(ci) = s
        .data
        .get("client_ip")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
    {
        ci.to_string()
    } else {
        "127.0.0.1".to_string()
    };

    let method = if s.method.is_empty() {
        "POST".to_string()
    } else {
        s.method.to_uppercase()
    };
    let route = if s.route.is_empty() {
        "-".to_string()
    } else {
        s.route.clone()
    };

    Value::Object(vmap! {
        "job_id" => s.job_id.clone(),
        "workflow_name" => s.workflow_name.clone(),
        "workflow" => s.workflow_name.clone(),
        "status" => status_str,
        "status_code" => status_code,
        "route" => route,
        "method" => method,
        "source_ip" => client_host.clone(),
        "client_host" => client_host,
        "duration_ms" => s.duration_ms,
        "started_at" => s.started_at.clone(),
        "finished_at" => s.finished_at.clone().map(Value::String).unwrap_or(Value::Null),
        "error" => s.error.clone().map(Value::String).unwrap_or(Value::Null),
        "error_type" => s.error_type.clone().map(Value::String).unwrap_or(Value::Null),
    })
}

pub fn register_live_routes_with_config(
    router: &mut Router,
    workflow_use_case: Arc<dyn IWorkflowEngine>,
    visualizer_use_case: Arc<dyn IVisualizerCompiler>,
    cfg: RouteConfig,
) {
    let api_key = cfg.api_key.clone();
    let opts = cfg.options.clone();
    let authorizer = cfg.tooling_authorizer.clone();
    let registered_models = cfg.registered_models.clone();

    let tooling_mw: HandlerFunc = {
        let api_key = api_key.clone();
        let opts = opts.clone();
        let authorizer = authorizer.clone();
        Arc::new(move |c: &mut HttpContext| {
            let deny = |c: &mut HttpContext, detail: &str| {
                c.abort_with_status_json(
                    403,
                    Value::Object(vmap! {
                        "error" => "ToolingAccessDenied",
                        "detail" => detail,
                    }),
                );
            };
            let is_loopback = c
                .client_ip()
                .parse::<IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false);
            match decide_tooling_access(
                &opts.tooling,
                authorizer.is_some(),
                !api_key.is_empty(),
                is_loopback,
            ) {
                ToolingDecision::Allow => {}
                ToolingDecision::NotFound => {
                    c.abort_with_status(404);
                    return;
                }
                ToolingDecision::AskAuthorizer => {
                    let auth_fn = authorizer.as_ref().expect("decided with an authorizer");
                    if !auth_fn(c) {
                        if !c.is_aborted() {
                            deny(c, "Not allowed to use the engine tooling.");
                        }
                        return;
                    }
                }
                ToolingDecision::RequireApiKey => {
                    if !check_auth(c, &api_key, &opts) {
                        return;
                    }
                }
                ToolingDecision::Deny(detail) => {
                    deny(c, detail);
                    return;
                }
            }
            c.next();
        })
    };

    // 1. System Health
    router.get("/health", |c: &mut HttpContext| {
        c.json(
            200,
            Value::Object(vmap! {
                "status" => "UP",
                "engine" => "OCTOPUS Live In-Memory Engine (Rust)",
                "mode" => "sync",
                "version" => "1.0.0",
            }),
        );
    });

    // 2. Swagger UI & OpenAPI Spec
    {
        let viz = Arc::clone(&visualizer_use_case);
        let api_key_clone = api_key.clone();
        let models_clone = registered_models.clone();
        router.get_with_mw("/openapi.json", tooling_mw.clone(), move |c: &mut HttpContext| {
            let blueprints = viz.list_blueprints().unwrap_or_default();
            let mut paths = ValueMap::new();
            let mut schemas = ValueMap::new();

            for (name, schema_val) in &models_clone {
                if schema_val.is_object() {
                    schemas.insert(name.clone(), schema_val.clone());
                } else {
                    schemas.insert(
                        name.clone(),
                        Value::Object(vmap! {
                            "type" => "object",
                        }),
                    );
                }
            }

            let mut security_schemes = ValueMap::new();
            let mut bearer_excludes: Vec<String> = Vec::new();
            let mut bearer_includes: Vec<String> = Vec::new();
            let mut has_bearer = false;

            for bp in &blueprints {
                for mw in &bp.middlewares {
                    if mw.name.contains("Bearer") {
                        has_bearer = true;
                        bearer_excludes.extend(mw.excludes.iter().cloned());
                        bearer_includes.extend(mw.includes.iter().cloned());
                    }
                }
            }

            if has_bearer {
                security_schemes.insert(
                    "HTTPBearer".to_string(),
                    Value::Object(vmap! {
                        "type" => "http",
                        "scheme" => "bearer",
                        "bearerFormat" => "JWT",
                        "description" => "Enter your Bearer / JWT token (e.g. obtained via /auth/login)",
                    }),
                );
            }
            if !api_key_clone.is_empty() {
                security_schemes.insert(
                    "ApiKeyAuth".to_string(),
                    Value::Object(vmap! {
                        "type" => "apiKey",
                        "in" => "header",
                        "name" => "X-API-Key",
                        "description" => "API Key header authentication",
                    }),
                );
            }

            let route_matcher = RouteMatcher::new();
            for bp in &blueprints {
                for ep in &bp.api {
                    let method_lower = ep.method.to_lowercase();
                    let mut endpoint_params = extract_openapi_path_params(&ep.route);
                    endpoint_params.push(Value::Object(vmap! {
                        "name" => "X-Job-ID",
                        "in" => "header",
                        "required" => false,
                        "description" => "Optional custom Job ID for idempotency and live tracing",
                        "schema" => Value::Object(vmap! { "type" => "string" }),
                    }));

                    let mut op = vmap! {
                        "summary" => ep.summary.clone(),
                        "description" => ep.description.clone(),
                        "tags" => Value::Array(ep.tags.iter().cloned().map(Value::String).collect()),
                        "parameters" => Value::Array(endpoint_params),
                    };

                    if (method_lower == "post" || method_lower == "put" || method_lower == "patch")
                        && !ep.input_model.is_empty() {
                            if !schemas.contains_key(&ep.input_model) {
                                schemas.insert(
                                    ep.input_model.clone(),
                                    Value::Object(vmap! {
                                        "type" => "object",
                                        "description" => format!("Auto-declared schema for {}", ep.input_model),
                                    }),
                                );
                            }
                            op.insert(
                                "requestBody".to_string(),
                                Value::Object(vmap! {
                                    "required" => true,
                                    "description" => format!("Request payload for {}", ep.input_model),
                                    "content" => Value::Object(vmap! {
                                        "application/json" => Value::Object(vmap! {
                                            "schema" => Value::Object(vmap! {
                                                "$ref" => format!("#/components/schemas/{}", ep.input_model),
                                            }),
                                        }),
                                    }),
                                }),
                            );
                        }

                    let status_code = if ep.status_code == 0 { 200 } else { ep.status_code };
                    let mut resp_obj = vmap! {
                        "description" => "Successful operation",
                    };
                    if !ep.output_model.is_empty() {
                        if !schemas.contains_key(&ep.output_model) {
                            schemas.insert(
                                ep.output_model.clone(),
                                Value::Object(vmap! {
                                    "type" => "object",
                                    "description" => format!("Auto-declared schema for {}", ep.output_model),
                                }),
                            );
                        }
                        resp_obj.insert(
                            "content".to_string(),
                            Value::Object(vmap! {
                                "application/json" => Value::Object(vmap! {
                                    "schema" => Value::Object(vmap! {
                                        "$ref" => format!("#/components/schemas/{}", ep.output_model),
                                    }),
                                }),
                            }),
                        );
                    }
                    let mut responses_map = ValueMap::new();
                    responses_map.insert(status_code.to_string(), Value::Object(resp_obj));
                    op.insert("responses".to_string(), Value::Object(responses_map));

                    if has_bearer {
                        if !route_matcher.should_skip(&ep.route, &bearer_excludes, &bearer_includes) {
                            op.insert(
                                "security".to_string(),
                                Value::Array(vec![Value::Object(vmap! {
                                    "HTTPBearer" => Value::Array(Vec::new()),
                                })]),
                            );
                        } else {
                            op.insert("security".to_string(), Value::Array(Vec::new()));
                        }
                    } else if !api_key_clone.is_empty() {
                        op.insert(
                            "security".to_string(),
                            Value::Array(vec![Value::Object(vmap! {
                                "ApiKeyAuth" => Value::Array(Vec::new()),
                            })]),
                        );
                    }

                    let route_entry = paths
                        .entry(ep.route.clone())
                        .or_insert_with(|| Value::Object(ValueMap::new()));
                    if let Value::Object(ref mut m) = route_entry {
                        m.insert(method_lower, Value::Object(op));
                    }
                }
            }

            let mut components = vmap! {
                "schemas" => Value::Object(schemas),
            };
            if !security_schemes.is_empty() {
                components.insert(
                    "securitySchemes".to_string(),
                    Value::Object(security_schemes),
                );
            }

            c.json(
                200,
                Value::Object(vmap! {
                    "openapi" => "3.0.0",
                    "info" => Value::Object(vmap! {
                        "title" => "OCTOPUS Live Engine API",
                        "version" => "1.0.0",
                        "description" => "Declarative Model-Driven In-Memory Synchronous API Framework",
                    }),
                    "paths" => Value::Object(paths),
                    "components" => Value::Object(components),
                }),
            );
        });
    }

    router.get_with_mw("/docs", tooling_mw.clone(), |c: &mut HttpContext| {
        let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>OCTOPUS Live Engine API Docs</title>
    <link rel="stylesheet" type="text/css" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
    <div id="swagger-ui"></div>
    <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
    <script>
        window.onload = () => {
            window.ui = SwaggerUIBundle({
                url: '/openapi.json',
                dom_id: '#swagger-ui',
            });
        };
    </script>
</body>
</html>"#;
        c.data(200, "text/html; charset=utf-8", html.as_bytes().to_vec());
    });

    // 3. Visualizer UI
    {
        let viz = Arc::clone(&visualizer_use_case);
        router.get_with_mw(
            "/visualizer",
            tooling_mw.clone(),
            move |c: &mut HttpContext| {
                let blueprints = viz.list_blueprints().unwrap_or_default();
                let mut selected_domain = c.query("domain");
                if selected_domain.is_empty() && !blueprints.is_empty() {
                    selected_domain = blueprints[0].workflow_name.clone();
                }
                let mut prefix = WorkflowBlueprint {
                    workflow_name: selected_domain.clone(),
                    ..Default::default()
                }
                .api_prefix();
                for bp in &blueprints {
                    if bp.workflow_name == selected_domain {
                        prefix = bp.api_prefix();
                    }
                }
                let rendered = render_visualizer_html(&selected_domain, &prefix);
                c.data(200, "text/html; charset=utf-8", rendered.into_bytes());
            },
        );
    }

    {
        let viz = Arc::clone(&visualizer_use_case);
        router.get_with_mw(
            "/api/visualizer/domains",
            tooling_mw.clone(),
            move |c: &mut HttpContext| {
                let blueprints = viz.list_blueprints().unwrap_or_default();
                let mut res = Vec::with_capacity(blueprints.len());
                for bp in &blueprints {
                    let endpoints_list: Vec<Value> = bp
                        .api
                        .iter()
                        .map(|ep| {
                            Value::Object(vmap! {
                                "route" => ep.route.clone(),
                                "method" => ep.method.clone(),
                                "input_model" => ep.input_model.clone(),
                                "output_model" => ep.output_model.clone(),
                                "status_code" => ep.status_code as i64,
                                "summary" => ep.summary.clone(),
                                "tags" => Value::Array(ep.tags.iter().cloned().map(Value::String).collect()),
                            })
                        })
                        .collect();
                    res.push(Value::Object(vmap! {
                        "domain" => bp.workflow_name.clone(),
                        "namespace" => bp.namespace.clone(),
                        "prefix" => bp.api_prefix(),
                        "workflow_name" => bp.workflow_name.clone(),
                        "version" => bp.version.clone(),
                        "mode" => bp.mode.clone(),
                        "description" => bp.description.clone(),
                        "endpoints" => Value::Array(endpoints_list),
                    }));
                }
                c.json(200, Value::Array(res));
            },
        );
    }

    {
        let wf_uc = Arc::clone(&workflow_use_case);
        router.get_with_mw(
            "/api/visualizer/jobs",
            tooling_mw.clone(),
            move |c: &mut HttpContext| {
                let wf = c.query("workflow");
                let limit = c
                    .query("limit")
                    .parse::<usize>()
                    .ok()
                    .filter(|&v| v > 0)
                    .unwrap_or(100);
                match wf_uc.list_execution_states(&wf, limit) {
                    Ok(states) => {
                        let items: Vec<Value> = states.iter().map(format_job_item).collect();
                        c.json(200, Value::Array(items));
                    }
                    Err(e) => {
                        c.json(500, Value::Object(vmap! { "error" => e }));
                    }
                }
            },
        );
    }

    {
        let wf_uc = Arc::clone(&workflow_use_case);
        router.get_with_mw(
            "/api/jobs",
            tooling_mw.clone(),
            move |c: &mut HttpContext| {
                let wf = c.query("workflow");
                let limit = c
                    .query("limit")
                    .parse::<usize>()
                    .ok()
                    .filter(|&v| v > 0)
                    .unwrap_or(100);
                match wf_uc.list_execution_states(&wf, limit) {
                    Ok(states) => {
                        let items: Vec<Value> = states.iter().map(format_job_item).collect();
                        c.json(200, Value::Array(items));
                    }
                    Err(e) => {
                        c.json(500, Value::Object(vmap! { "error" => e }));
                    }
                }
            },
        );
    }

    {
        let wf_uc = Arc::clone(&workflow_use_case);
        let viz = Arc::clone(&visualizer_use_case);
        router.get_with_mw(
            "/api/visualizer/resolve-job",
            tooling_mw.clone(),
            move |c: &mut HttpContext| {
                let job_id = c.query("job_id");
                let Ok(state) = wf_uc.get_execution_state(&job_id) else {
                    c.json(
                        200,
                        Value::Object(vmap! {
                            "exists" => false,
                            "job_id" => job_id,
                        }),
                    );
                    return;
                };
                let blueprints = viz.list_blueprints().unwrap_or_default();
                let prefix = blueprints
                    .iter()
                    .find(|b| b.workflow_name == state.workflow_name)
                    .map(|b| b.api_prefix())
                    .unwrap_or_else(|| {
                        WorkflowBlueprint {
                            workflow_name: state.workflow_name.clone(),
                            ..Default::default()
                        }
                        .api_prefix()
                    });
                c.json(
                    200,
                    Value::Object(vmap! {
                        "exists" => true,
                        "job_id" => job_id,
                        "domain" => state.workflow_name,
                        "status" => state.status.as_str(),
                        "route" => state.route,
                        "method" => state.method,
                        "prefix" => prefix,
                        "belongs_to_service" => true,
                    }),
                );
            },
        );
    }

    // 4. Register dynamic RPC endpoints & declared custom routes
    let blueprints = visualizer_use_case.list_blueprints().unwrap_or_default();
    for bp in blueprints {
        let wf_name = bp.workflow_name.clone();
        let prefix = bp.api_prefix();
        let blueprint_obj = bp.clone();

        // Execute Sync (Standard RPC)
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            let w_name = wf_name.clone();
            let bp_obj = blueprint_obj.clone();
            let api_key = api_key.clone();
            let opts = opts.clone();
            router.post(
                &format!("{}/execute", prefix),
                move |c: &mut HttpContext| {
                    if !check_auth(c, &api_key, &opts) {
                        return;
                    }
                    let Some(mut payload) = bind_json(c) else {
                        return;
                    };
                    let job_id = extract_or_create_job_id(c, &payload, &opts);
                    payload.insert("job_id".to_string(), Value::String(job_id));
                    let mut client_ip = c.client_ip();
                    if client_ip.is_empty() {
                        client_ip = "127.0.0.1".to_string();
                    }
                    payload.insert("client_host".to_string(), Value::String(client_ip.clone()));
                    payload.insert("client_ip".to_string(), Value::String(client_ip));
                    extract_injections(c, &mut payload);

                    let start_t = Instant::now();
                    let res = wf_uc.execute_sync(&w_name, payload);
                    let duration_ms = start_t.elapsed().as_micros() as f64 / 1000.0;

                    match res {
                        Ok(state) => {
                            c.header("X-Job-ID", &state.job_id);
                            c.header("X-Execution-Status", state.status.as_str());
                            c.header("X-Execution-Duration-Ms", &format!("{:.2}", duration_ms));

                            if state.status == ExecutionStatus::Failed
                                || state.status == ExecutionStatus::RolledBack
                            {
                                fail_response(c, Some(&state), state.error.as_deref(), &opts);
                                return;
                            }

                            if c.query("raw") == "true" || c.query("raw") == "1" {
                                let traces_val: Vec<Value> =
                                    state.traces.iter().map(|t| t.to_value()).collect();
                                c.json(
                                    200,
                                    Value::Object(vmap! {
                                        "job_id" => state.job_id,
                                        "workflow_name" => state.workflow_name,
                                        "status" => state.status.as_str(),
                                        "data" => Value::Object(public_data(&bp_obj, &state.data)),
                                        "traces" => Value::Array(traces_val),
                                    }),
                                );
                                return;
                            }

                            c.json(200, Value::Object(public_data(&bp_obj, &state.data)));
                        }
                        Err((state_opt, err_msg)) => {
                            if let Some(ref state) = state_opt {
                                c.header("X-Job-ID", &state.job_id);
                                c.header("X-Execution-Status", state.status.as_str());
                                c.header("X-Execution-Duration-Ms", &format!("{:.2}", duration_ms));
                            }
                            fail_response(c, state_opt.as_ref(), Some(&err_msg), &opts);
                        }
                    }
                },
            );
        }

        // Signal: resume a workflow paused at an await_signal step
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            let bp_obj = blueprint_obj.clone();
            let opts = opts.clone();
            router.post_with_mw(
                &format!("{}/signal/:job_id", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let Some(body) = bind_json(c) else {
                        return;
                    };
                    let mut name = c.query("signal");
                    if name.is_empty() {
                        if let Some(n) = body.get("signal").and_then(|v| v.as_str()) {
                            name = n.to_string();
                        }
                    }
                    let by = match c.get("auth_claims") {
                        Some(Value::Object(m)) => m.clone(),
                        _ => ValueMap::new(),
                    };
                    let job_id = c.param("job_id");
                    match wf_uc.signal(&job_id, &name, body, by) {
                        Ok(state) => {
                            if state.status == ExecutionStatus::Failed
                                || state.status == ExecutionStatus::RolledBack
                            {
                                fail_response(c, Some(&state), state.error.as_deref(), &opts);
                                return;
                            }
                            c.header("X-Job-ID", &state.job_id);
                            c.header("X-Execution-Status", state.status.as_str());
                            let out_map = state
                                .output_data
                                .as_ref()
                                .map(|od| public_data(&bp_obj, od))
                                .unwrap_or_default();
                            c.json(
                                200,
                                Value::Object(vmap! {
                                    "job_id" => state.job_id,
                                    "status" => state.status.as_str(),
                                    "output" => Value::Object(out_map),
                                }),
                            );
                        }
                        Err((state_opt, err_msg)) => {
                            if err_msg == ERR_NO_SUSPENSION {
                                c.json(
                                    404,
                                    Value::Object(vmap! {
                                        "error" => "NoSuspension",
                                        "detail" => err_msg,
                                    }),
                                );
                                return;
                            }
                            if err_msg == ERR_SIGNAL_MISMATCH {
                                c.json(
                                    409,
                                    Value::Object(vmap! {
                                        "error" => "SignalMismatch",
                                        "detail" => err_msg,
                                    }),
                                );
                                return;
                            }
                            fail_response(c, state_opt.as_ref(), Some(&err_msg), &opts);
                        }
                    }
                },
            );
        }

        // Initiate Async (Standard RPC)
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            let w_name = wf_name.clone();
            let api_key = api_key.clone();
            let opts = opts.clone();
            router.post(
                &format!("{}/initiate", prefix),
                move |c: &mut HttpContext| {
                    if !check_auth(c, &api_key, &opts) {
                        return;
                    }
                    let Some(mut payload) = bind_json(c) else {
                        return;
                    };
                    let job_id = extract_or_create_job_id(c, &payload, &opts);
                    payload.insert("job_id".to_string(), Value::String(job_id));
                    match wf_uc.initiate_async(&w_name, payload) {
                        Ok(state) => {
                            c.header("X-Job-ID", &state.job_id);
                            c.json(
                                200,
                                Value::Object(vmap! {
                                    "status" => "INITIATED",
                                    "job_id" => state.job_id,
                                    "workflow_name" => state.workflow_name,
                                }),
                            );
                        }
                        Err(e) => {
                            c.json(500, Value::Object(vmap! { "error" => e }));
                        }
                    }
                },
            );
        }

        // Status
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            router.get_with_mw(
                &format!("{}/status/:job_id", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let job_id = c.param("job_id");
                    match wf_uc.get_execution_state(&job_id) {
                        Ok(state) => {
                            c.json(
                                200,
                                Value::Object(vmap! {
                                    "job_id" => state.job_id,
                                    "workflow_name" => state.workflow_name,
                                    "lifecycle_state" => state.status.as_str(),
                                    "status" => state.status.as_str(),
                                    "route" => state.route,
                                    "method" => state.method,
                                    "current_step_id" => state.current_step_id,
                                    "current_index" => state.current_index as i64,
                                    "total_steps" => state.total_steps as i64,
                                    "started_at" => state.started_at,
                                    "finished_at" => state.finished_at.map(Value::String).unwrap_or(Value::Null),
                                }),
                            );
                        }
                        Err(_) => {
                            c.json(
                                404,
                                Value::Object(vmap! {
                                    "error" => format!("Job '{}' not found", job_id),
                                }),
                            );
                        }
                    }
                },
            );
        }

        // Timeline
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            router.get_with_mw(
                &format!("{}/timeline/:job_id", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let job_id = c.param("job_id");
                    match wf_uc.get_execution_state(&job_id) {
                        Ok(state) => {
                            let steps: Vec<Value> =
                                state.traces.iter().map(|t| t.to_value()).collect();
                            c.json(
                                200,
                                Value::Object(vmap! {
                                    "job_id" => state.job_id,
                                    "workflow_name" => state.workflow_name,
                                    "status" => state.status.as_str(),
                                    "route" => state.route,
                                    "method" => state.method,
                                    "data" => Value::Object(state.data),
                                    "steps" => Value::Array(steps),
                                }),
                            );
                        }
                        Err(_) => {
                            c.json(
                                404,
                                Value::Object(vmap! {
                                    "error" => format!("Job '{}' not found", job_id),
                                }),
                            );
                        }
                    }
                },
            );
        }

        // SSE Stream
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            router.get_with_mw(
                &format!("{}/stream/:job_id", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let job_id = c.param("job_id");
                    match wf_uc.stream_events(&job_id) {
                        Ok((rx, cleanup)) => {
                            let (sse_tx, sse_rx) = mpsc::channel();
                            std::thread::spawn(move || {
                                while let Ok(ev) = rx.recv() {
                                    let msg = SseEventMessage {
                                        event: ev.event,
                                        data: ev.data.to_json_string(),
                                    };
                                    if sse_tx.send(msg).is_err() {
                                        break;
                                    }
                                }
                                cleanup();
                            });
                            c.sse_receiver = Some(sse_rx);
                        }
                        Err(e) => {
                            c.json(500, Value::Object(vmap! { "error" => e }));
                        }
                    }
                },
            );
        }

        // Diagram X6
        {
            let viz = Arc::clone(&visualizer_use_case);
            let w_name = wf_name.clone();
            router.get_with_mw(
                &format!("{}/diagram/x6", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let job_id = c.query("job_id");
                    let job_id_opt = if job_id.is_empty() {
                        None
                    } else {
                        Some(job_id.as_str())
                    };
                    let route = c.query("route");
                    let route_opt = if route.is_empty() {
                        None
                    } else {
                        Some(route.as_str())
                    };
                    let method = c.query("method");
                    let method_opt = if method.is_empty() {
                        None
                    } else {
                        Some(method.as_str())
                    };
                    match viz.compile_x6_diagram(&w_name, job_id_opt, route_opt, method_opt) {
                        Ok(diagram) => {
                            c.json(200, Value::Object(diagram));
                        }
                        Err(e) => {
                            c.json(500, Value::Object(vmap! { "error" => e }));
                        }
                    }
                },
            );
        }

        // Jobs endpoint per workflow
        {
            let wf_uc = Arc::clone(&workflow_use_case);
            let w_name = wf_name.clone();
            router.get_with_mw(
                &format!("{}/jobs", prefix),
                tooling_mw.clone(),
                move |c: &mut HttpContext| {
                    let limit = c
                        .query("limit")
                        .parse::<usize>()
                        .ok()
                        .filter(|&v| v > 0)
                        .unwrap_or(100);
                    match wf_uc.list_execution_states(&w_name, limit) {
                        Ok(states) => {
                            let items: Vec<Value> = states.iter().map(format_job_item).collect();
                            c.json(200, Value::Array(items));
                        }
                        Err(e) => {
                            c.json(500, Value::Object(vmap! { "error" => e }));
                        }
                    }
                },
            );
        }

        // 4B. Mount Custom Declared API Endpoints
        for ep in &bp.api {
            let declared = DeclaredRoute::new(&wf_name, ep);
            let gin_route = convert_openapi_route_to_gin(&declared.route);
            let ep_method = declared.method.clone();
            let has_body = declared.has_body();

            let wf_uc = Arc::clone(&workflow_use_case);
            let bp_obj = blueprint_obj.clone();
            let api_key = api_key.clone();
            let opts = opts.clone();

            router.handle(
                &ep_method,
                &gin_route,
                Arc::new(move |c: &mut HttpContext| {
                    if !check_auth(c, &api_key, &opts) {
                        return;
                    }

                    let mut payload = ValueMap::new();
                    if has_body {
                        let Some(bound) = bind_json(c) else {
                            return;
                        };
                        payload = bound;
                    }

                    // Ingest query params
                    for (k, vals) in &c.query_params {
                        if let Some(first) = vals.first() {
                            payload.insert(k.clone(), Value::String(first.clone()));
                        }
                    }
                    // Ingest path params
                    for (k, v) in &c.params {
                        payload.insert(k.clone(), Value::String(v.clone()));
                    }

                    let req = RouteRequest {
                        job_id_candidates: job_id_candidates(c, &payload),
                        payload,
                        client_ip: c.client_ip(),
                        injections: c.get("workflow_injections").cloned(),
                        auth_claims: c.get("auth_claims").cloned(),
                        raw: c.query("raw") == "true" || c.query("raw") == "1",
                    };
                    let answer = run_declared_route(wf_uc.as_ref(), &bp_obj, &declared, &opts, req);
                    for (k, v) in &answer.headers {
                        c.header(k, v);
                    }
                    c.json(answer.status, answer.body);
                }),
            );
        }
    }
}

pub fn convert_openapi_route_to_gin(route: &str) -> String {
    let mut out = String::with_capacity(route.len());
    for ch in route.chars() {
        if ch == '{' {
            out.push(':');
        } else if ch == '}' {
            // skip
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn extract_openapi_path_params(route: &str) -> Vec<Value> {
    let mut params = Vec::new();
    for p in route.split('/') {
        if p.starts_with('{') && p.ends_with('}') && p.len() > 2 {
            let name = p.trim_matches(|c| c == '{' || c == '}');
            params.push(Value::Object(vmap! {
                "name" => name,
                "in" => "path",
                "required" => true,
                "description" => format!("Path parameter: {}", name),
                "schema" => Value::Object(vmap! { "type" => "string" }),
            }));
        } else if let Some(name) = p.strip_prefix(':') {
            if !name.is_empty() {
                params.push(Value::Object(vmap! {
                    "name" => name,
                    "in" => "path",
                    "required" => true,
                    "description" => format!("Path parameter: {}", name),
                    "schema" => Value::Object(vmap! { "type" => "string" }),
                }));
            }
        }
    }
    params
}
