//! The engine exports: one `OctopusEngine` per host engine, blueprints,
//! activity callbacks, runs, jobs, routes, middleware, live events and the
//! visualizer.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use octopus_core::domain::models::{EngineOptions, ExecutionState};
use octopus_core::domain::services::{RequestMeta, ResponseMeta, RouteRequest, ToolingDecision};
use octopus_core::json::{self, Value, ValueMap};
use octopus_core::visualizer_html::render_visualizer_html;
use octopus_core::{vmap, Engine};

use crate::{
    guard, json_arg, opt_text, text, write_json, write_out, Error, OctopusBuf,
    OCTOPUS_ERR_WORKFLOW, OCTOPUS_OK,
};

/// An engine. Opaque to the host; create it with [`octopus_engine_new`] and
/// free it with [`octopus_engine_free`].
///
/// Register blueprints and activities before running workflows: registration
/// waits for running calls to finish.
pub struct OctopusEngine {
    engine: RwLock<Engine>,
    subscriptions: Mutex<HashMap<u64, Box<dyn FnOnce() + Send>>>,
    next_subscription: AtomicU64,
}

/// Collects an activity's result; see [`octopus_out_write`].
pub struct OctopusOut {
    bytes: Vec<u8>,
}

/// An activity implemented by the host. It gets the step input as JSON and
/// writes its result with [`octopus_out_write`]: a JSON object when it returns
/// `OCTOPUS_OK`, otherwise an error message. It may run on any thread the
/// core owns, concurrently with other calls.
pub type OctopusActivityFn = Option<
    unsafe extern "C" fn(
        user_data: *mut c_void,
        input: *const u8,
        input_len: usize,
        out: *mut OctopusOut,
    ) -> i32,
>;

/// Receives one live event as JSON `{event, data}`, on a thread the core owns.
pub type OctopusEventFn =
    Option<unsafe extern "C" fn(user_data: *mut c_void, event: *const u8, event_len: usize)>;

/// A host pointer handed back to its callbacks. The host keeps it valid for as
/// long as the callback is registered and makes it safe to use from any thread.
#[derive(Clone, Copy)]
struct UserData(*mut c_void);

unsafe impl Send for UserData {}
unsafe impl Sync for UserData {}

unsafe fn engine_ref<'a>(engine: *const OctopusEngine) -> Result<&'a OctopusEngine, Error> {
    engine
        .as_ref()
        .ok_or_else(|| Error::argument("the engine is null"))
}

fn state_value(state: &ExecutionState) -> Value {
    state.to_value()
}

fn object(v: Value, what: &str) -> Result<ValueMap, Error> {
    match v {
        Value::Object(m) => Ok(m),
        Value::Null => Ok(ValueMap::new()),
        _ => Err(Error::argument(format!("{} must be a JSON object", what))),
    }
}

fn workflow_error(state: Option<&ExecutionState>, message: &str) -> Error {
    let kind = state
        .and_then(|s| s.error_type.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "WorkflowError".to_string());
    Error::new(OCTOPUS_ERR_WORKFLOW, &kind, message)
}

// --- Lifecycle ---

/// A new engine. `config_json` (may be null) is `{api_key, options}`, where
/// `options` uses `EngineOptions`' field names. Returns null on failure; see
/// [`octopus_last_error`].
///
/// # Safety
/// `config_json` is null or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn octopus_engine_new(config_json: *const c_char) -> *mut OctopusEngine {
    let mut created: *mut OctopusEngine = std::ptr::null_mut();
    guard(|| {
        let cfg = json_arg(config_json, "config_json")?;
        let get = |k: &str| cfg.as_object().and_then(|m| m.get(k));
        let api_key = get("api_key").and_then(|v| v.as_str()).unwrap_or("");
        let mut engine = Engine::new(api_key);
        if let Some(opts) = get("options") {
            engine.set_options(EngineOptions::from_value(opts));
        }
        created = Box::into_raw(Box::new(OctopusEngine {
            engine: RwLock::new(engine),
            subscriptions: Mutex::new(HashMap::new()),
            next_subscription: AtomicU64::new(1),
        }));
        Ok(OCTOPUS_OK)
    });
    created
}

/// Frees an engine and ends its event subscriptions.
///
/// # Safety
/// `engine` came from [`octopus_engine_new`], no call on it is running, and it
/// is not used again.
#[no_mangle]
pub unsafe extern "C" fn octopus_engine_free(engine: *mut OctopusEngine) {
    if engine.is_null() {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let engine = Box::from_raw(engine);
        let subs = std::mem::take(&mut *engine.subscriptions.lock().unwrap());
        for (_, unsubscribe) in subs {
            unsubscribe();
        }
        engine.engine.read().unwrap().stop_embedded_worker();
    }));
}

// --- Blueprints and activities ---

/// Registers a blueprint written as YAML or JSON.
///
/// # Safety
/// `engine` is a live engine; `blueprint` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn octopus_register_blueprint(
    engine: *const OctopusEngine,
    blueprint: *const c_char,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let text = text(blueprint, "blueprint")?;
        e.engine
            .write()
            .unwrap()
            .register_blueprint_yaml(text)
            .map_err(Error::engine)?;
        Ok(OCTOPUS_OK)
    })
}

/// Registers an activity the host implements. Registering a name again
/// replaces it.
///
/// # Safety
/// `engine` is a live engine and `name` a NUL-terminated string. `callback`
/// and `user_data` stay valid while the engine lives.
#[no_mangle]
pub unsafe extern "C" fn octopus_register_activity(
    engine: *const OctopusEngine,
    name: *const c_char,
    callback: OctopusActivityFn,
    user_data: *mut c_void,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let name = text(name, "name")?;
        let callback = callback.ok_or_else(|| Error::argument("the callback is null"))?;
        let user_data = UserData(user_data);
        e.engine.read().unwrap().register_activity_func(
            name,
            Arc::new(move |input: &ValueMap| {
                let user_data = user_data;
                let input = Value::Object(input.clone()).to_json_string();
                let mut out = OctopusOut { bytes: Vec::new() };
                let status = callback(user_data.0, input.as_ptr(), input.len(), &mut out);
                let text = String::from_utf8_lossy(&out.bytes);
                if status != OCTOPUS_OK {
                    return Err(if text.is_empty() {
                        format!("activity failed with status {}", status)
                    } else {
                        text.into_owned()
                    });
                }
                match json::parse(&text) {
                    Ok(Value::Object(m)) => Ok(m),
                    Ok(Value::Null) => Ok(ValueMap::new()),
                    Ok(_) => Err("the activity's result is not a JSON object".to_string()),
                    Err(e) => Err(format!("the activity's result is not JSON: {}", e)),
                }
            }),
        );
        Ok(OCTOPUS_OK)
    })
}

/// Sets an activity's result, replacing anything written before.
///
/// # Safety
/// `out` is the pointer the activity was called with, and `bytes` points to
/// `len` readable bytes (or is null when `len` is 0).
#[no_mangle]
pub unsafe extern "C" fn octopus_out_write(out: *mut OctopusOut, bytes: *const u8, len: usize) {
    if let Some(out) = out.as_mut() {
        out.bytes.clear();
        if !bytes.is_null() && len > 0 {
            out.bytes
                .extend_from_slice(std::slice::from_raw_parts(bytes, len));
        }
    }
}

// --- Runs and jobs ---

/// Runs a workflow to its end (or to its first asynchronous step) and writes
/// the job's state to `out`. Returns `OCTOPUS_ERR_WORKFLOW` when the run
/// failed; `out` then holds the state if the job was created.
///
/// # Safety
/// `engine` is a live engine, `workflow` a NUL-terminated string, `input_json`
/// null or a NUL-terminated JSON object, and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_execute(
    engine: *const OctopusEngine,
    workflow: *const c_char,
    input_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let workflow = text(workflow, "workflow")?;
        let input = object(json_arg(input_json, "input_json")?, "input_json")?;
        let res = e.engine.read().unwrap().execute_sync(workflow, input);
        match res {
            Ok(state) => {
                write_json(out, &state_value(&state))?;
                Ok(OCTOPUS_OK)
            }
            Err((state, msg)) => {
                if let Some(ref st) = state {
                    write_json(out, &state_value(st))?;
                }
                Err(workflow_error(state.as_ref(), &msg))
            }
        }
    })
}

/// Starts a workflow in the background and writes the new job's state.
///
/// # Safety
/// As [`octopus_execute`].
#[no_mangle]
pub unsafe extern "C" fn octopus_initiate_async(
    engine: *const OctopusEngine,
    workflow: *const c_char,
    input_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let workflow = text(workflow, "workflow")?;
        let input = object(json_arg(input_json, "input_json")?, "input_json")?;
        let state = e
            .engine
            .read()
            .unwrap()
            .initiate_async(workflow, input)
            .map_err(|m| workflow_error(None, &m))?;
        write_json(out, &state_value(&state))?;
        Ok(OCTOPUS_OK)
    })
}

/// Sends a signal to a job waiting on `await_signal`, resumes it and writes
/// its state. `data_json` and `by_json` may be null.
///
/// # Safety
/// As [`octopus_execute`]; `job_id` and `name` are NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn octopus_signal(
    engine: *const OctopusEngine,
    job_id: *const c_char,
    name: *const c_char,
    data_json: *const c_char,
    by_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let job_id = text(job_id, "job_id")?;
        let name = text(name, "name")?;
        let data = object(json_arg(data_json, "data_json")?, "data_json")?;
        let by = object(json_arg(by_json, "by_json")?, "by_json")?;
        let res = e.engine.read().unwrap().signal(job_id, name, data, by);
        match res {
            Ok(state) => {
                write_json(out, &state_value(&state))?;
                Ok(OCTOPUS_OK)
            }
            Err((state, msg)) => {
                if let Some(ref st) = state {
                    write_json(out, &state_value(st))?;
                }
                Err(workflow_error(state.as_ref(), &msg))
            }
        }
    })
}

/// Writes a job's state, as the state store holds it.
///
/// # Safety
/// As [`octopus_execute`]; `job_id` is a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn octopus_get_state(
    engine: *const OctopusEngine,
    job_id: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let job_id = text(job_id, "job_id")?;
        let state = e
            .engine
            .read()
            .unwrap()
            .get_execution_state(job_id)
            .map_err(Error::not_found)?;
        write_json(out, &state_value(&state))?;
        Ok(OCTOPUS_OK)
    })
}

/// Writes the newest jobs as a JSON array; `workflow` null (or empty) means
/// every workflow.
///
/// # Safety
/// As [`octopus_execute`].
#[no_mangle]
pub unsafe extern "C" fn octopus_list_states(
    engine: *const OctopusEngine,
    workflow: *const c_char,
    limit: usize,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let workflow = opt_text(workflow, "workflow")?.unwrap_or("");
        let states = e
            .engine
            .read()
            .unwrap()
            .list_execution_states(workflow, limit)
            .map_err(Error::engine)?;
        write_json(out, &Value::Array(states.iter().map(state_value).collect()))?;
        Ok(OCTOPUS_OK)
    })
}

// --- Declared routes ---

/// The declared `api:` routes as a JSON array of
/// `{workflow_name, method, route, status_code, has_body}`; `route` uses the
/// OpenAPI path form (`/orders/{id}`).
///
/// # Safety
/// `engine` is a live engine and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_declared_routes(
    engine: *const OctopusEngine,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let routes: Vec<Value> = e
            .engine
            .read()
            .unwrap()
            .declared_routes()
            .iter()
            .map(|r| {
                Value::Object(vmap! {
                    "workflow_name" => r.workflow_name.clone(),
                    "method" => r.method.clone(),
                    "route" => r.route.clone(),
                    "status_code" => r.status_code as i64,
                    "has_body" => r.has_body(),
                })
            })
            .collect();
        write_json(out, &Value::Array(routes))?;
        Ok(OCTOPUS_OK)
    })
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Runs a declared route and writes the answer `{status, headers, body}`.
/// `request_json` is `{payload, job_id_candidates, client_ip, injections,
/// auth_claims, raw}`; every field is optional.
///
/// # Safety
/// `engine` is a live engine; `method`, `route` and `request_json` are
/// NUL-terminated strings; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_call_route(
    engine: *const OctopusEngine,
    method: *const c_char,
    route: *const c_char,
    request_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let method = text(method, "method")?.to_uppercase();
        let route = text(route, "route")?;
        let req = object(json_arg(request_json, "request_json")?, "request_json")?;
        let engine = e.engine.read().unwrap();
        let declared = engine
            .declared_routes()
            .into_iter()
            .find(|r| r.method == method && r.route == route)
            .ok_or_else(|| Error::not_found(format!("no declared route {} {}", method, route)))?;
        let request = RouteRequest {
            payload: object(
                req.get("payload").cloned().unwrap_or(Value::Null),
                "payload",
            )?,
            job_id_candidates: string_list(req.get("job_id_candidates")),
            client_ip: req
                .get("client_ip")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            injections: req.get("injections").cloned(),
            auth_claims: req.get("auth_claims").cloned(),
            raw: req.get("raw").and_then(|v| v.as_bool()).unwrap_or(false),
        };
        let answer = engine.call_route(&declared, request);
        let headers: ValueMap = answer
            .headers
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        write_json(
            out,
            &Value::Object(vmap! {
                "status" => answer.status as i64,
                "headers" => Value::Object(headers),
                "body" => answer.body,
            }),
        )?;
        Ok(OCTOPUS_OK)
    })
}

// --- Access ---

/// Writes `{decision, detail}`, where `decision` is `allow`, `not_found`,
/// `ask_authorizer`, `require_api_key` or `deny`. `request_json` is
/// `{has_authorizer, client_is_loopback}`.
///
/// # Safety
/// `engine` is a live engine, `request_json` a NUL-terminated string and
/// `out` writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_tooling_decision(
    engine: *const OctopusEngine,
    request_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let req = object(json_arg(request_json, "request_json")?, "request_json")?;
        let flag = |k: &str| req.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let decision = e
            .engine
            .read()
            .unwrap()
            .tooling_decision(flag("has_authorizer"), flag("client_is_loopback"));
        let (name, detail) = match decision {
            ToolingDecision::Allow => ("allow", ""),
            ToolingDecision::NotFound => ("not_found", ""),
            ToolingDecision::AskAuthorizer => ("ask_authorizer", ""),
            ToolingDecision::RequireApiKey => ("require_api_key", ""),
            ToolingDecision::Deny(d) => ("deny", d),
        };
        write_json(
            out,
            &Value::Object(vmap! { "decision" => name, "detail" => detail }),
        )?;
        Ok(OCTOPUS_OK)
    })
}

/// Whether the `X-API-Key` header (or, when the options allow it, the
/// `api_key` query parameter) opens the engine's protected routes. Null
/// arguments count as empty; always true when the engine has no key.
///
/// # Safety
/// `engine` is a live engine; `header` and `query` are null or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn octopus_api_key_allows(
    engine: *const OctopusEngine,
    header: *const c_char,
    query: *const c_char,
) -> bool {
    let mut allowed = false;
    guard(|| {
        let e = engine_ref(engine)?;
        let header = opt_text(header, "header")?.unwrap_or("");
        let query = opt_text(query, "query")?.unwrap_or("");
        allowed = e.engine.read().unwrap().api_key_allows(header, query);
        Ok(OCTOPUS_OK)
    });
    allowed
}

// --- Middleware ---

/// Writes the blueprints' middleware chain as `[{name, built_in}]`, in order.
/// The host runs each link: a built-in one through
/// [`octopus_middleware_before`] and [`octopus_middleware_after`] with its
/// index, any other with its own implementation of that name.
///
/// # Safety
/// `engine` is a live engine and `out` writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_middleware_chain(
    engine: *const OctopusEngine,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let links: Vec<Value> = e
            .engine
            .read()
            .unwrap()
            .middleware_chain()
            .iter()
            .map(|l| {
                Value::Object(vmap! {
                    "name" => l.config.name.clone(),
                    "built_in" => l.middleware.is_some(),
                })
            })
            .collect();
        write_json(out, &Value::Array(links))?;
        Ok(OCTOPUS_OK)
    })
}

/// A request as JSON `{method, path, headers, query, client_ip, body,
/// context}`, owned so it can lend a `RequestMeta`. `query` maps each name to
/// a list of values; `body` is text.
struct OwnedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    query: BTreeMap<String, Vec<String>>,
    client_ip: String,
    body: Vec<u8>,
    context: ValueMap,
}

impl OwnedRequest {
    fn from_value(v: &ValueMap) -> Self {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let headers = match v.get("headers") {
            Some(Value::Object(m)) => m
                .iter()
                .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                .collect(),
            _ => BTreeMap::new(),
        };
        let query = match v.get("query") {
            Some(Value::Object(m)) => m
                .iter()
                .map(|(k, x)| {
                    let vals = match x {
                        Value::String(s) => vec![s.clone()],
                        other => string_list(Some(other)),
                    };
                    (k.clone(), vals)
                })
                .collect(),
            _ => BTreeMap::new(),
        };
        let context = match v.get("context") {
            Some(Value::Object(m)) => m.clone(),
            _ => ValueMap::new(),
        };
        Self {
            method: s("method").to_uppercase(),
            path: s("path"),
            headers,
            query,
            client_ip: s("client_ip"),
            body: s("body").into_bytes(),
            context,
        }
    }

    fn meta(&self) -> RequestMeta<'_> {
        RequestMeta {
            method: &self.method,
            path: &self.path,
            headers: &self.headers,
            query: &self.query,
            client_ip: &self.client_ip,
            body: &self.body,
            context: &self.context,
        }
    }
}

unsafe fn built_in(
    e: &OctopusEngine,
    index: usize,
) -> Result<Arc<dyn octopus_core::domain::services::Middleware>, Error> {
    let chain = e.engine.read().unwrap().middleware_chain();
    chain
        .get(index)
        .and_then(|l| l.middleware.clone())
        .ok_or_else(|| Error::not_found(format!("no built-in middleware at index {}", index)))
}

/// Asks a built-in middleware about a request and writes its decision
/// `{headers, context, respond}`, where `respond` is null (go on) or
/// `{status, content_type, body}` (answer with this and stop).
///
/// # Safety
/// `engine` is a live engine, `request_json` a NUL-terminated string and
/// `out` writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_middleware_before(
    engine: *const OctopusEngine,
    index: usize,
    request_json: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let mw = built_in(e, index)?;
        let req = OwnedRequest::from_value(&object(
            json_arg(request_json, "request_json")?,
            "request_json",
        )?);
        let d = mw.before(&req.meta());
        let headers: ValueMap = d
            .headers
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        let respond = match d.respond {
            None => Value::Null,
            Some(r) => Value::Object(vmap! {
                "status" => r.status as i64,
                "content_type" => r.content_type,
                "body" => String::from_utf8_lossy(&r.body).into_owned(),
            }),
        };
        write_json(
            out,
            &Value::Object(vmap! {
                "headers" => Value::Object(headers),
                "context" => Value::Object(d.context),
                "respond" => respond,
            }),
        )?;
        Ok(OCTOPUS_OK)
    })
}

/// Tells a built-in middleware how the request was answered, once the rest
/// of the chain has run. `response_json` is `{status, body, duration_ms}`;
/// `request_json.context` holds what the handler left there.
///
/// # Safety
/// `engine` is a live engine; both JSON arguments are NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn octopus_middleware_after(
    engine: *const OctopusEngine,
    index: usize,
    request_json: *const c_char,
    response_json: *const c_char,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let mw = built_in(e, index)?;
        let req = OwnedRequest::from_value(&object(
            json_arg(request_json, "request_json")?,
            "request_json",
        )?);
        let resp = object(json_arg(response_json, "response_json")?, "response_json")?;
        let body = resp
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .as_bytes()
            .to_vec();
        mw.after(
            &req.meta(),
            &ResponseMeta {
                status: resp.get("status").and_then(|v| v.as_i64()).unwrap_or(200) as u16,
                body: &body,
                duration_ms: resp
                    .get("duration_ms")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
            },
        );
        Ok(OCTOPUS_OK)
    })
}

// --- Live events ---

/// Calls `callback` with each live event of a job until
/// [`octopus_unsubscribe`]. Returns the subscription ID, or 0 on failure.
///
/// # Safety
/// `engine` is a live engine and `job_id` a NUL-terminated string. `callback`
/// and `user_data` stay valid until the subscription ends.
#[no_mangle]
pub unsafe extern "C" fn octopus_subscribe_events(
    engine: *const OctopusEngine,
    job_id: *const c_char,
    callback: OctopusEventFn,
    user_data: *mut c_void,
) -> u64 {
    let mut id = 0;
    guard(|| {
        let e = engine_ref(engine)?;
        let job_id = text(job_id, "job_id")?;
        let callback = callback.ok_or_else(|| Error::argument("the callback is null"))?;
        let (rx, unsubscribe) = e
            .engine
            .read()
            .unwrap()
            .subscribe_events(job_id)
            .map_err(Error::engine)?;
        let user_data = UserData(user_data);
        std::thread::spawn(move || {
            let user_data = user_data;
            // Ends when the subscription is dropped and the channel closes.
            while let Ok(ev) = rx.recv() {
                let text = Value::Object(vmap! { "event" => ev.event, "data" => ev.data })
                    .to_json_string();
                callback(user_data.0, text.as_ptr(), text.len());
            }
        });
        id = e.next_subscription.fetch_add(1, Ordering::Relaxed);
        e.subscriptions.lock().unwrap().insert(id, unsubscribe);
        Ok(OCTOPUS_OK)
    });
    id
}

/// Ends a subscription. Events already queued may still arrive.
///
/// # Safety
/// `engine` is a live engine.
#[no_mangle]
pub unsafe extern "C" fn octopus_unsubscribe(engine: *const OctopusEngine, subscription: u64) {
    guard(|| {
        let e = engine_ref(engine)?;
        let unsubscribe = e.subscriptions.lock().unwrap().remove(&subscription);
        if let Some(unsubscribe) = unsubscribe {
            unsubscribe();
        }
        Ok(OCTOPUS_OK)
    });
}

// --- Visualizer and worker ---

/// Writes the visualizer graph JSON for a workflow, with a job's progress when
/// `job_id` (may be null) names one.
///
/// # Safety
/// `engine` is a live engine, `workflow` a NUL-terminated string and `out`
/// writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_compile_diagram(
    engine: *const OctopusEngine,
    workflow: *const c_char,
    job_id: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        let workflow = text(workflow, "workflow")?;
        let job_id = opt_text(job_id, "job_id")?.filter(|j| !j.is_empty());
        let graph = e
            .engine
            .read()
            .unwrap()
            .compile_diagram(workflow, job_id, None, None)
            .map_err(Error::not_found)?;
        write_json(out, &Value::Object(graph))?;
        Ok(OCTOPUS_OK)
    })
}

/// Writes the visualizer page (HTML).
///
/// # Safety
/// Both arguments are NUL-terminated strings; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn octopus_visualizer_html(
    route_prefix: *const c_char,
    default_domain: *const c_char,
    out: *mut OctopusBuf,
) -> i32 {
    guard(|| {
        let prefix = text(route_prefix, "route_prefix")?;
        let domain = text(default_domain, "default_domain")?;
        write_out(out, render_visualizer_html(domain, prefix).into_bytes())?;
        Ok(OCTOPUS_OK)
    })
}

/// Starts the embedded Temporal worker, when a blueprint configured Temporal
/// and the core was built with it. Otherwise does nothing.
///
/// # Safety
/// `engine` is a live engine.
#[no_mangle]
pub unsafe extern "C" fn octopus_start_worker(engine: *const OctopusEngine) -> i32 {
    guard(|| {
        let e = engine_ref(engine)?;
        e.engine
            .read()
            .unwrap()
            .start_embedded_worker()
            .map_err(Error::engine)?;
        Ok(OCTOPUS_OK)
    })
}

/// Stops the embedded Temporal worker.
///
/// # Safety
/// `engine` is a live engine.
#[no_mangle]
pub unsafe extern "C" fn octopus_stop_worker(engine: *const OctopusEngine) -> i32 {
    guard(|| {
        engine_ref(engine)?
            .engine
            .read()
            .unwrap()
            .stop_embedded_worker();
        Ok(OCTOPUS_OK)
    })
}
