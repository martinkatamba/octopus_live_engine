//! What the engine's HTTP routes answer, decided once for every host: which
//! job ID to trust, what a declared `api:` route responds with, and how a
//! failed run is reported. The host parses the request and writes the answer.

use std::collections::BTreeMap;
use std::time::Instant;

use crate::crypto::new_job_id;
use crate::domain::models::{
    APIEndpointConfig, EngineOptions, ExecutionState, ExecutionStatus, WorkflowBlueprint,
};
use crate::domain::ports::IWorkflowEngine;
use crate::domain::services::mask_payload;
use crate::json::{Value, ValueMap};
use crate::vmap;

/// Client job IDs: 1–128 characters of `[A-Za-z0-9._:-]`.
pub fn is_valid_client_job_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// The first valid client-supplied ID when the options trust them, otherwise
/// a new one. `candidates` are in priority order: the `X-Job-ID`,
/// `X-Request-ID` and `X-Correlation-ID` headers, the body's `job_id`, then
/// `?job_id=`.
pub fn choose_job_id(opts: &EngineOptions, candidates: &[String]) -> String {
    if opts.allow_client_job_ids {
        if let Some(id) = candidates.iter().find(|id| is_valid_client_job_id(id)) {
            return id.clone();
        }
    }
    new_job_id()
}

/// Replaces any client-sent injection keys with the ones middleware set:
/// each injection as itself and as `_injected_<key>`, and the auth claims
/// as `_auth_claims`.
pub fn apply_injections(
    payload: &mut ValueMap,
    injections: Option<&Value>,
    auth_claims: Option<&Value>,
) {
    payload.retain(|k, _| !k.starts_with("_injected_") && k != "_auth_claims");
    if let Some(Value::Object(inj)) = injections {
        for (k, v) in inj {
            payload.insert(k.clone(), v.clone());
            payload.insert(format!("_injected_{}", k), v.clone());
        }
    }
    if let Some(Value::Object(claims)) = auth_claims {
        payload.insert("_auth_claims".to_string(), Value::Object(claims.clone()));
    }
}

/// Job data as clients may see it: masked, without injections.
pub fn public_data(bp: &WorkflowBlueprint, data: &ValueMap) -> ValueMap {
    let (fields, mask_val) = bp.all_mask_fields();
    let mut out = mask_payload(data, &fields, &mask_val);
    out.retain(|k, _| !k.starts_with("_injected_") && k != "_auth_claims");
    out
}

/// The status and body for a failed run. Unexpected errors are hidden behind
/// a reference to the job unless the options expose them; an error the
/// blueprint raised on purpose (it has an error type) is shown.
pub fn failure_body(
    state: Option<&ExecutionState>,
    err: Option<&str>,
    opts: &EngineOptions,
) -> (u16, Value) {
    let mut code: u16 = 500;
    let mut err_type = "WorkflowExecutionError".to_string();
    let mut job_id = String::new();
    let mut status_val = Value::Null;
    let mut intentional = false;

    if let Some(st) = state {
        job_id = st.job_id.clone();
        status_val = Value::String(st.status.as_str().to_string());
        if let Some(ec) = st.error_code {
            if ec > 0 {
                code = ec as u16;
            }
        }
        if let Some(ref et) = st.error_type {
            if !et.is_empty() {
                err_type = et.clone();
                intentional = true;
            }
        }
    }

    let mut msg = if !job_id.is_empty() {
        format!("An internal error occurred (reference {}).", job_id)
    } else {
        "An internal error occurred.".to_string()
    };

    let state_error = state
        .and_then(|s| s.error.as_ref())
        .filter(|e| !e.is_empty());
    if let (true, Some(e)) = (intentional || opts.expose_error_details, state_error) {
        msg = e.clone();
    } else if opts.expose_error_details {
        if let Some(e) = err.filter(|e| !e.is_empty()) {
            msg = e.to_string();
        }
    }

    (
        code,
        Value::Object(vmap! {
            "error" => err_type,
            "detail" => msg,
            "status_code" => code as i64,
            "job_id" => job_id,
            "status" => status_val,
        }),
    )
}

/// A declared `api:` route, as a host mounts it.
#[derive(Clone, Debug)]
pub struct DeclaredRoute {
    pub workflow_name: String,
    /// Upper case; POST when the blueprint leaves it out.
    pub method: String,
    /// With a leading `/`; path parameters in OpenAPI form (`/orders/{id}`).
    pub route: String,
    /// The success status, 200 unless the blueprint sets one.
    pub status_code: u16,
    pub endpoint: APIEndpointConfig,
}

impl DeclaredRoute {
    pub fn new(workflow_name: &str, ep: &APIEndpointConfig) -> Self {
        let method = if ep.method.is_empty() {
            "POST".to_string()
        } else {
            ep.method.to_uppercase()
        };
        let route = if ep.route.starts_with('/') {
            ep.route.clone()
        } else {
            format!("/{}", ep.route)
        };
        let status_code = if ep.status_code == 0 {
            200
        } else {
            ep.status_code
        };
        Self {
            workflow_name: workflow_name.to_string(),
            method,
            route,
            status_code,
            endpoint: ep.clone(),
        }
    }

    pub fn has_body(&self) -> bool {
        matches!(self.method.as_str(), "POST" | "PUT" | "PATCH")
    }
}

/// The declared routes of every blueprint, in blueprint order.
pub fn declared_routes(blueprints: &[WorkflowBlueprint]) -> Vec<DeclaredRoute> {
    blueprints
        .iter()
        .flat_map(|bp| {
            bp.api
                .iter()
                .map(|ep| DeclaredRoute::new(&bp.workflow_name, ep))
        })
        .collect()
}

/// A call to a declared route, as the host read it.
#[derive(Clone, Debug, Default)]
pub struct RouteRequest {
    /// The JSON body, then query parameters, then path parameters, each
    /// overriding the one before.
    pub payload: ValueMap,
    /// The job ID candidates, in [`choose_job_id`] order.
    pub job_id_candidates: Vec<String>,
    pub client_ip: String,
    /// What middleware stored on the request.
    pub injections: Option<Value>,
    pub auth_claims: Option<Value>,
    /// `?raw=true`: answer with the job's traces instead of its output.
    pub raw: bool,
}

/// How to answer a declared route.
#[derive(Clone, Debug)]
pub struct RouteAnswer {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

/// Runs a declared route's workflow and builds the answer.
pub fn run_declared_route(
    engine: &dyn IWorkflowEngine,
    bp: &WorkflowBlueprint,
    route: &DeclaredRoute,
    opts: &EngineOptions,
    req: RouteRequest,
) -> RouteAnswer {
    let mut payload = req.payload;
    let job_id = choose_job_id(opts, &req.job_id_candidates);
    payload.insert("job_id".to_string(), Value::String(job_id));
    let client_ip = if req.client_ip.is_empty() {
        "127.0.0.1".to_string()
    } else {
        req.client_ip
    };
    payload.insert("client_host".to_string(), Value::String(client_ip.clone()));
    payload.insert("client_ip".to_string(), Value::String(client_ip));
    apply_injections(
        &mut payload,
        req.injections.as_ref(),
        req.auth_claims.as_ref(),
    );

    let start = Instant::now();
    let res = engine.execute_endpoint_sync(
        &route.workflow_name,
        Some(&route.endpoint),
        &route.route,
        &route.method,
        payload,
    );
    let duration_ms = start.elapsed().as_micros() as f64 / 1000.0;

    let mut headers = BTreeMap::new();
    let mut job_headers = |st: &ExecutionState| {
        headers.insert("X-Job-ID".to_string(), st.job_id.clone());
        headers.insert(
            "X-Execution-Status".to_string(),
            st.status.as_str().to_string(),
        );
        headers.insert(
            "X-Execution-Duration-Ms".to_string(),
            format!("{:.2}", duration_ms),
        );
    };

    let (status, body) = match res {
        Ok(state) => {
            job_headers(&state);
            if state.status == ExecutionStatus::Failed
                || state.status == ExecutionStatus::RolledBack
            {
                failure_body(Some(&state), state.error.as_deref(), opts)
            } else if req.raw {
                let traces: Vec<Value> = state.traces.iter().map(|t| t.to_value()).collect();
                (
                    route.status_code,
                    Value::Object(vmap! {
                        "job_id" => state.job_id.clone(),
                        "workflow_name" => state.workflow_name.clone(),
                        "route" => route.route.clone(),
                        "method" => route.method.clone(),
                        "status" => state.status.as_str(),
                        "data" => Value::Object(public_data(bp, &state.data)),
                        "traces" => Value::Array(traces),
                    }),
                )
            } else {
                // A declared route responds with the real output; the stored
                // state is masked (decision D5).
                let out = state
                    .route_output
                    .clone()
                    .unwrap_or_else(|| public_data(bp, &state.data));
                (route.status_code, Value::Object(out))
            }
        }
        Err((state, err)) => {
            if let Some(ref st) = state {
                job_headers(st);
            }
            failure_body(state.as_ref(), Some(&err), opts)
        }
    };
    RouteAnswer {
        status,
        headers,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(allow_ids: bool, expose: bool) -> EngineOptions {
        EngineOptions {
            allow_client_job_ids: allow_ids,
            expose_error_details: expose,
            ..Default::default()
        }
    }

    #[test]
    fn client_job_ids_only_when_allowed_and_valid() {
        let ids = vec![String::new(), "bad id!".to_string(), "job-7".to_string()];
        assert_eq!(choose_job_id(&opts(true, false), &ids), "job-7");
        assert_ne!(choose_job_id(&opts(false, false), &ids), "job-7");
        assert!(!is_valid_client_job_id(&"x".repeat(129)));
    }

    #[test]
    fn unexpected_errors_are_hidden_unless_exposed() {
        let state = ExecutionState {
            job_id: "j1".into(),
            status: ExecutionStatus::Failed,
            error: Some("db password wrong".into()),
            ..Default::default()
        };
        let (code, body) =
            failure_body(Some(&state), Some("db password wrong"), &opts(false, false));
        assert_eq!(code, 500);
        let detail = body.as_object().unwrap()["detail"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(detail, "An internal error occurred (reference j1).");
        let (_, body) = failure_body(Some(&state), None, &opts(false, true));
        assert_eq!(
            body.as_object().unwrap()["detail"].as_str(),
            Some("db password wrong")
        );

        let raised = ExecutionState {
            error_type: Some("OutOfStock".into()),
            error_code: Some(409),
            ..state
        };
        let (code, body) = failure_body(Some(&raised), None, &opts(false, false));
        assert_eq!(code, 409);
        assert_eq!(
            body.as_object().unwrap()["error"].as_str(),
            Some("OutOfStock")
        );
        assert_eq!(
            body.as_object().unwrap()["detail"].as_str(),
            Some("db password wrong")
        );
    }

    #[test]
    fn injections_replace_what_the_client_sent() {
        let mut payload = ValueMap::new();
        payload.insert("_injected_role".into(), Value::String("admin".into()));
        payload.insert("_auth_claims".into(), Value::String("forged".into()));
        let inj = Value::Object(vmap! { "user_id" => "u1" });
        apply_injections(&mut payload, Some(&inj), None);
        assert!(!payload.contains_key("_injected_role"));
        assert!(!payload.contains_key("_auth_claims"));
        assert_eq!(payload["_injected_user_id"].as_str(), Some("u1"));
        assert_eq!(payload["user_id"].as_str(), Some("u1"));
        let bp = WorkflowBlueprint::default();
        assert!(!public_data(&bp, &payload).contains_key("_injected_user_id"));
    }
}
