//! `octopus_core._native`: the core engine for Python.
//!
//! Values convert directly between Python objects and the core's JSON values
//! (no JSON text in between). Runs release the GIL; each activity call takes
//! it again, on whichever core thread runs the step.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use octopus_core::adapters::outbound::{ActivityResolver, StoreFactory};
use octopus_core::domain::models::{
    EngineOptions, ExecutionState, LiveEvent, MiddlewareConfig, WorkflowBlueprint,
};
use octopus_core::domain::ports::{IActivityExecutor, IBlueprintRepository};
use octopus_core::domain::services::{
    build_middleware, DeclaredRoute, Middleware, OctopusMiddlewareError, RequestMeta, ResponseMeta,
    RouteRequest, ToolingDecision,
};
use octopus_core::json::{Value, ValueMap};
use octopus_core::net::IpNet;
use octopus_core::visualizer_html::render_visualizer_html_for;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

create_exception!(
    _native,
    OctopusError,
    PyException,
    "A workflow failed. `args` is `(message, state)`; `state` is the job's state dict, or None."
);

// --- Conversions ---

/// Converts core values to Python. A map the core shares (a trace's input
/// and the job's data, for example) becomes one dict, converted once and
/// used wherever the map appears, so a returned state is read-only in
/// spirit: changing one place may change another.
struct ToPy<'py> {
    py: Python<'py>,
    shared: HashMap<*const (), Bound<'py, PyDict>>,
}

impl<'py> ToPy<'py> {
    fn new(py: Python<'py>) -> Self {
        Self {
            py,
            shared: HashMap::new(),
        }
    }

    fn value(&mut self, v: &Value) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        Ok(match v {
            Value::Null => py.None().into_bound(py),
            Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
            Value::Int(i) => i.into_pyobject(py)?.into_any(),
            Value::Float(f) => PyFloat::new(py, *f).into_any(),
            Value::String(s) => PyString::new(py, s).into_any(),
            Value::Array(items) => {
                let list = PyList::empty(py);
                for item in items {
                    list.append(self.value(item)?)?;
                }
                list.into_any()
            }
            Value::Object(m) => self.map(m)?.into_any(),
        })
    }

    fn map(&mut self, m: &ValueMap) -> PyResult<Bound<'py, PyDict>> {
        if !m.is_shared() {
            return self.fill(m);
        }
        if let Some(d) = self.shared.get(&m.as_ptr()) {
            return Ok(d.clone());
        }
        let d = self.fill(m)?;
        self.shared.insert(m.as_ptr(), d.clone());
        Ok(d)
    }

    fn fill(&mut self, m: &ValueMap) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(self.py);
        for (k, v) in m {
            dict.set_item(PyString::new(self.py, k), self.value(v)?)?;
        }
        Ok(dict)
    }
}

fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    ToPy::new(py).value(v)
}

fn map_to_py<'py>(py: Python<'py>, m: &ValueMap) -> PyResult<Bound<'py, PyDict>> {
    ToPy::new(py).map(m)
}

fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    // The exact built-in types first: they are almost all of the data.
    if obj.is_exact_instance_of::<PyString>() {
        return Ok(Value::String(obj.cast::<PyString>()?.to_str()?.to_string()));
    }
    if obj.is_exact_instance_of::<PyDict>() {
        return Ok(Value::Object(dict_to_map(obj.cast::<PyDict>()?)?));
    }
    if obj.is_exact_instance_of::<PyInt>() {
        return match obj.extract::<i64>() {
            Ok(i) => Ok(Value::Int(i)),
            Err(_) => Ok(Value::Float(obj.extract::<f64>()?)),
        };
    }
    if obj.is_exact_instance_of::<PyList>() {
        let l = obj.cast::<PyList>()?;
        let mut items = Vec::with_capacity(l.len());
        for x in l.iter() {
            items.push(from_py(&x)?);
        }
        return Ok(Value::Array(items));
    }
    if obj.is_exact_instance_of::<PyFloat>() {
        return Ok(Value::Float(obj.extract::<f64>()?));
    }
    if obj.is_none() {
        return Ok(Value::Null);
    }
    // bool before int: Python's bool is an int.
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if obj.is_instance_of::<PyInt>() {
        return match obj.extract::<i64>() {
            Ok(i) => Ok(Value::Int(i)),
            Err(_) => Ok(Value::Float(obj.extract::<f64>()?)),
        };
    }
    if obj.is_instance_of::<PyFloat>() {
        return Ok(Value::Float(obj.extract::<f64>()?));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(Value::String(s.to_str()?.to_string()));
    }
    if let Ok(d) = obj.cast::<PyDict>() {
        return Ok(Value::Object(dict_to_map(d)?));
    }
    if let Ok(l) = obj.cast::<PyList>() {
        return l
            .iter()
            .map(|x| from_py(&x))
            .collect::<PyResult<_>>()
            .map(Value::Array);
    }
    if let Ok(t) = obj.cast::<PyTuple>() {
        return t
            .iter()
            .map(|x| from_py(&x))
            .collect::<PyResult<_>>()
            .map(Value::Array);
    }
    // Pydantic models and other objects with a dict form.
    if let Ok(dumped) = obj.call_method0("model_dump") {
        return from_py(&dumped);
    }
    Err(PyTypeError::new_err(format!(
        "cannot convert {} to a workflow value",
        obj.get_type().name()?
    )))
}

fn dict_to_map(d: &Bound<'_, PyDict>) -> PyResult<ValueMap> {
    let mut m = BTreeMap::new();
    for (k, v) in d.iter() {
        let key = match k.cast::<PyString>() {
            Ok(s) => s.to_str()?.to_string(),
            Err(_) => k.str()?.to_str()?.to_string(),
        };
        m.insert(key, from_py(&v)?);
    }
    Ok(m.into())
}

fn opt_map(obj: Option<&Bound<'_, PyAny>>) -> PyResult<ValueMap> {
    match obj {
        None => Ok(ValueMap::new()),
        Some(o) if o.is_none() => Ok(ValueMap::new()),
        Some(o) => match from_py(o)? {
            Value::Object(m) => Ok(m),
            _ => Err(PyTypeError::new_err("expected a dict")),
        },
    }
}

fn state_to_py<'py>(py: Python<'py>, st: &ExecutionState) -> PyResult<Bound<'py, PyAny>> {
    to_py(py, &st.to_value())
}

/// A run's state, with `route_output`: the unmasked output a declared route
/// answers with (decision D5). Stored states never carry it.
fn returned_to_py<'py>(py: Python<'py>, st: &ExecutionState) -> PyResult<Bound<'py, PyAny>> {
    let mut v = st.to_value();
    if let (Some(out), Value::Object(m)) = (&st.route_output, &mut v) {
        m.insert("route_output".to_string(), Value::Object(out.clone()));
    }
    to_py(py, &v)
}

fn run_result(
    py: Python<'_>,
    res: Result<ExecutionState, (Option<ExecutionState>, String)>,
) -> PyResult<Py<PyAny>> {
    match res {
        Ok(st) => Ok(returned_to_py(py, &st)?.unbind()),
        Err((st, msg)) => {
            let state = match st {
                Some(ref s) => returned_to_py(py, s)?.unbind(),
                None => py.None(),
            };
            Err(OctopusError::new_err((msg, state)))
        }
    }
}

/// What an activity returned, as the step's output.
fn activity_result(py: Python<'_>, out: &Bound<'_, PyAny>) -> Result<ValueMap, String> {
    match from_py(out).map_err(|e| activity_error(py, e))? {
        Value::Object(m) => Ok(m),
        Value::Null => Ok(ValueMap::new()),
        _ => Err("TypeError: an activity must return a dict or None".to_string()),
    }
}

/// An activity's Python exception as the error string the engine records:
/// `"TypeName: message"`, so `on_exception` can match the type. An exception
/// with an HTTP status in `code` or `status_code` becomes
/// `"[TypeName:code] message"`, which the core reads as that status (and
/// does not retry when it is a 4xx). A message already in that form is kept
/// as it is.
fn activity_error(py: Python<'_>, err: PyErr) -> String {
    let name = err
        .get_type(py)
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "Exception".to_string());
    let value = err.value(py);
    let msg = value.str().map(|s| s.to_string()).unwrap_or_default();
    // Already in the engine's form (a host formatted it): keep it.
    if OctopusMiddlewareError::parse_from_error_str(&msg).is_some() {
        return msg;
    }
    let code = ["code", "status_code"].iter().find_map(|attr| {
        value
            .getattr(*attr)
            .ok()
            .filter(|c| !c.is_instance_of::<PyBool>())
            .and_then(|c| c.extract::<i64>().ok())
            .filter(|c| (100..600).contains(c))
    });
    match code {
        Some(code) => format!("[{name}:{code}] {msg}"),
        None if msg.is_empty() => name,
        None => format!("{name}: {msg}"),
    }
}

// --- Requests ---

/// A request dict `{method, path, headers, query, client_ip, body, context}`,
/// owned so it can lend a `RequestMeta`.
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
    fn from_py(d: &Bound<'_, PyDict>) -> PyResult<Self> {
        let s = |k: &str| -> PyResult<String> {
            Ok(match d.get_item(k)? {
                Some(v) if !v.is_none() => v.extract::<String>()?,
                _ => String::new(),
            })
        };
        let headers = match d.get_item("headers")? {
            Some(v) if !v.is_none() => v.extract::<BTreeMap<String, String>>()?,
            _ => BTreeMap::new(),
        };
        let mut query = BTreeMap::new();
        if let Some(q) = d.get_item("query")? {
            if let Ok(q) = q.cast::<PyDict>() {
                for (k, v) in q.iter() {
                    let vals = match v.extract::<String>() {
                        Ok(one) => vec![one],
                        Err(_) => v.extract::<Vec<String>>()?,
                    };
                    query.insert(k.extract::<String>()?, vals);
                }
            }
        }
        let body = match d.get_item("body")? {
            Some(v) if !v.is_none() => match v.extract::<Vec<u8>>() {
                Ok(bytes) if !v.is_instance_of::<PyString>() => bytes,
                _ => v.extract::<String>()?.into_bytes(),
            },
            _ => Vec::new(),
        };
        Ok(Self {
            method: s("method")?.to_uppercase(),
            path: s("path")?,
            headers,
            query,
            client_ip: s("client_ip")?,
            body,
            context: opt_map(d.get_item("context")?.as_ref())?,
        })
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

// --- Engine ---

/// The workflow engine. Register blueprints and activities before running
/// workflows: registration waits for running calls to finish.
#[pyclass(module = "octopus_core._native")]
struct Engine {
    inner: RwLock<octopus_core::Engine>,
}

impl Engine {
    fn route(&self, method: &str, route: &str) -> PyResult<DeclaredRoute> {
        let method = method.to_uppercase();
        self.inner
            .read()
            .unwrap()
            .declared_routes()
            .into_iter()
            .find(|r| r.method == method && r.route == route)
            .ok_or_else(|| PyKeyError::new_err(format!("no declared route {} {}", method, route)))
    }

    fn built_in(&self, index: usize) -> PyResult<Arc<dyn Middleware>> {
        let chain = self.inner.read().unwrap().middleware_chain();
        chain
            .get(index)
            .and_then(|l| l.middleware.clone())
            .ok_or_else(|| {
                PyKeyError::new_err(format!("no built-in middleware at index {}", index))
            })
    }
}

#[pymethods]
impl Engine {
    /// `options` uses `EngineOptions`' field names (`allow_client_job_ids`,
    /// `tooling`, ...).
    #[new]
    #[pyo3(signature = (api_key = "", options = None))]
    fn new(api_key: &str, options: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let mut engine = octopus_core::Engine::new(api_key);
        if let Some(o) = options.filter(|o| !o.is_none()) {
            engine.set_options(EngineOptions::from_value(&from_py(o)?));
        }
        Ok(Self {
            inner: RwLock::new(engine),
        })
    }

    /// Registers a blueprint written as YAML or JSON.
    /// Registers a blueprint given as YAML (or JSON) text, or as a dict in
    /// the same shape.
    fn register_blueprint(&self, py: Python<'_>, blueprint: &Bound<'_, PyAny>) -> PyResult<()> {
        if let Ok(text) = blueprint.extract::<String>() {
            return py
                .detach(|| self.inner.write().unwrap().register_blueprint_yaml(&text))
                .map_err(PyValueError::new_err);
        }
        let value = from_py(blueprint)?;
        py.detach(|| {
            let bp = WorkflowBlueprint::from_value(&value)?;
            self.inner.write().unwrap().register_blueprint(bp)
        })
        .map_err(PyValueError::new_err)
    }

    /// Runs every activity that has no handler registered here through
    /// `call(name, payload) -> dict | None`; `has(name) -> bool` says which
    /// names it knows. For an SDK that keeps its own activity registry.
    /// `None` removes the resolver.
    #[pyo3(signature = (call, has = None))]
    fn set_activity_resolver(&self, call: Option<Py<PyAny>>, has: Option<Py<PyAny>>) {
        let Some(call) = call else {
            self.inner.read().unwrap().set_activity_resolver(None);
            return;
        };
        let call = Arc::new(call);
        let has = has.map(Arc::new);
        let resolver = ActivityResolver {
            call: Arc::new(move |name: &str, input: &ValueMap| {
                Python::attach(|py| {
                    let arg = map_to_py(py, input).map_err(|e| activity_error(py, e))?;
                    let out = call
                        .call1(py, (name, arg))
                        .map_err(|e| activity_error(py, e))?;
                    activity_result(py, out.bind(py))
                })
            }),
            has: Arc::new(move |name: &str| match &has {
                None => true,
                Some(f) => Python::attach(|py| {
                    f.call1(py, (name,))
                        .and_then(|r| r.is_truthy(py))
                        .unwrap_or(false)
                }),
            }),
        };
        self.inner
            .read()
            .unwrap()
            .set_activity_resolver(Some(resolver));
    }

    /// A subscription to a job's live events: `next(timeout)` returns the next
    /// event as `{"event", "data"}`, or None when none arrives in time.
    fn subscribe(&self, py: Python<'_>, job_id: &str) -> PyResult<Subscription> {
        let (rx, cancel) = py
            .detach(|| self.inner.read().unwrap().subscribe_events(job_id))
            .map_err(PyValueError::new_err)?;
        Ok(Subscription {
            rx: Mutex::new(Some(rx)),
            cancel: Mutex::new(Some(cancel)),
        })
    }

    fn load_blueprint_file(&self, py: Python<'_>, path: &str) -> PyResult<()> {
        py.detach(|| self.inner.write().unwrap().load_blueprint_file(path))
            .map_err(PyValueError::new_err)
    }

    /// Registers `func(payload: dict) -> dict | None` as an activity.
    fn register_activity(&self, name: &str, func: Py<PyAny>) {
        let func = Arc::new(func);
        self.inner.read().unwrap().register_activity_func(
            name,
            Arc::new(move |input: &ValueMap| {
                Python::attach(|py| {
                    let arg = map_to_py(py, input).map_err(|e| activity_error(py, e))?;
                    let out = func.call1(py, (arg,)).map_err(|e| activity_error(py, e))?;
                    activity_result(py, out.bind(py))
                })
            }),
        );
    }

    /// Runs a workflow and returns the job's state. Raises `OctopusError`
    /// when the run fails. `endpoint_index` picks one of the blueprint's
    /// `api` entries, whose steps then run; `route` and `method` are recorded
    /// on the job and scope its replay key.
    #[pyo3(signature = (workflow, payload = None, endpoint_index = None, route = "", method = ""))]
    fn execute(
        &self,
        py: Python<'_>,
        workflow: &str,
        payload: Option<&Bound<'_, PyAny>>,
        endpoint_index: Option<usize>,
        route: &str,
        method: &str,
    ) -> PyResult<Py<PyAny>> {
        let input = opt_map(payload)?;
        let res = py.detach(|| {
            let engine = self.inner.read().unwrap();
            if endpoint_index.is_none() && route.is_empty() && method.is_empty() {
                return engine.execute_sync(workflow, input);
            }
            let bp = engine
                .blueprint_repo
                .get_blueprint(workflow)
                .map_err(|e| (None, e))?;
            let endpoint = match endpoint_index {
                Some(i) => Some(bp.api.get(i).ok_or_else(|| {
                    (None, format!("workflow '{workflow}' has no api entry {i}"))
                })?),
                None => None,
            };
            engine.execute_endpoint_sync(workflow, endpoint, route, method, input)
        });
        run_result(py, res)
    }

    /// Keeps jobs in a store of the core's (`sqlite`, `redis`, `inmemory`,
    /// ...), configured as a blueprint's `persistence:` would.
    #[pyo3(signature = (store_type, config = None))]
    fn set_state_store(
        &self,
        py: Python<'_>,
        store_type: &str,
        config: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let cfg = opt_map(config)?;
        py.detach(|| {
            let store = StoreFactory::new().create(store_type, Some(&cfg));
            self.inner.write().unwrap().set_state_store(store);
        });
        Ok(())
    }

    /// Stores a state dict (the shape `get_state` returns).
    fn save_state(&self, py: Python<'_>, state: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = from_py(state)?;
        let st = ExecutionState::from_value(&value)
            .ok_or_else(|| PyValueError::new_err("not an execution state"))?;
        py.detach(|| self.inner.read().unwrap().save_state(st))
            .map_err(PyValueError::new_err)
    }

    #[pyo3(signature = (workflow, payload = None))]
    fn initiate_async(
        &self,
        py: Python<'_>,
        workflow: &str,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let input = opt_map(payload)?;
        let res = py.detach(|| self.inner.read().unwrap().initiate_async(workflow, input));
        run_result(py, res.map_err(|m| (None, m)))
    }

    #[pyo3(signature = (job_id, name, data = None, by = None))]
    fn signal(
        &self,
        py: Python<'_>,
        job_id: &str,
        name: &str,
        data: Option<&Bound<'_, PyAny>>,
        by: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let (data, by) = (opt_map(data)?, opt_map(by)?);
        let res = py.detach(|| self.inner.read().unwrap().signal(job_id, name, data, by));
        run_result(py, res)
    }

    /// The job's state, or None.
    fn get_state(&self, py: Python<'_>, job_id: &str) -> PyResult<Py<PyAny>> {
        let res = py.detach(|| self.inner.read().unwrap().get_execution_state(job_id));
        match res {
            Ok(st) => Ok(state_to_py(py, &st)?.unbind()),
            Err(_) => Ok(py.None()),
        }
    }

    #[pyo3(signature = (workflow = None, limit = 100))]
    fn list_states(
        &self,
        py: Python<'_>,
        workflow: Option<&str>,
        limit: usize,
    ) -> PyResult<Py<PyAny>> {
        let res = py.detach(|| {
            self.inner
                .read()
                .unwrap()
                .list_execution_states(workflow.unwrap_or(""), limit)
        });
        let states = res.map_err(PyValueError::new_err)?;
        let list = PyList::empty(py);
        for st in &states {
            list.append(state_to_py(py, st)?)?;
        }
        Ok(list.into_any().unbind())
    }

    /// `[{workflow_name, method, route, status_code, has_body}]`; `route` is
    /// in OpenAPI form (`/orders/{id}`).
    fn declared_routes(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        for r in self.inner.read().unwrap().declared_routes() {
            let d = PyDict::new(py);
            d.set_item("workflow_name", &r.workflow_name)?;
            d.set_item("method", &r.method)?;
            d.set_item("route", &r.route)?;
            d.set_item("status_code", r.status_code)?;
            d.set_item("has_body", r.has_body())?;
            d.set_item("input_model", &r.endpoint.input_model)?;
            d.set_item("output_model", &r.endpoint.output_model)?;
            d.set_item("summary", &r.endpoint.summary)?;
            list.append(d)?;
        }
        Ok(list.into_any().unbind())
    }

    /// Runs a declared route; returns `(status, headers, body)`. `request`
    /// may hold `payload`, `job_id_candidates`, `client_ip`, `injections`,
    /// `auth_claims` and `raw`.
    #[pyo3(signature = (method, route, request = None))]
    fn call_route(
        &self,
        py: Python<'_>,
        method: &str,
        route: &str,
        request: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<(u16, Py<PyAny>, Py<PyAny>)> {
        let declared = self.route(method, route)?;
        let mut req = RouteRequest::default();
        if let Some(r) = request {
            req.payload = opt_map(r.get_item("payload")?.as_ref())?;
            if let Some(c) = r.get_item("job_id_candidates")? {
                req.job_id_candidates = c
                    .extract::<Vec<Option<String>>>()?
                    .into_iter()
                    .flatten()
                    .collect();
            }
            if let Some(ip) = r.get_item("client_ip")?.filter(|v| !v.is_none()) {
                req.client_ip = ip.extract()?;
            }
            req.injections = r.get_item("injections")?.map(|v| from_py(&v)).transpose()?;
            req.auth_claims = r
                .get_item("auth_claims")?
                .map(|v| from_py(&v))
                .transpose()?;
            if let Some(raw) = r.get_item("raw")? {
                req.raw = raw.is_truthy()?;
            }
        }
        let answer = py.detach(|| self.inner.read().unwrap().call_route(&declared, req));
        let headers = PyDict::new(py);
        for (k, v) in &answer.headers {
            headers.set_item(k, v)?;
        }
        Ok((
            answer.status,
            headers.into_any().unbind(),
            to_py(py, &answer.body)?.unbind(),
        ))
    }

    /// The visualizer graph for a workflow, with a job's progress when
    /// `job_id` names one.
    /// `route` and `method` pick one of the blueprint's declared routes.
    #[pyo3(signature = (workflow, job_id = None, route = None, method = None))]
    fn compile_diagram(
        &self,
        py: Python<'_>,
        workflow: &str,
        job_id: Option<&str>,
        route: Option<&str>,
        method: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let some = |v: Option<&str>| v.filter(|s| !s.is_empty()).map(str::to_string);
        let (job_id, route, method) = (some(job_id), some(route), some(method));
        let res = py.detach(|| {
            self.inner.read().unwrap().compile_diagram(
                workflow,
                job_id.as_deref(),
                route.as_deref(),
                method.as_deref(),
            )
        });
        let graph = res.map_err(PyKeyError::new_err)?;
        Ok(map_to_py(py, &graph)?.into_any().unbind())
    }

    /// `(decision, detail)`; `decision` is `allow`, `not_found`,
    /// `ask_authorizer`, `require_api_key` or `deny`.
    fn tooling_decision(
        &self,
        has_authorizer: bool,
        client_is_loopback: bool,
    ) -> (&'static str, &'static str) {
        match self
            .inner
            .read()
            .unwrap()
            .tooling_decision(has_authorizer, client_is_loopback)
        {
            ToolingDecision::Allow => ("allow", ""),
            ToolingDecision::NotFound => ("not_found", ""),
            ToolingDecision::AskAuthorizer => ("ask_authorizer", ""),
            ToolingDecision::RequireApiKey => ("require_api_key", ""),
            ToolingDecision::Deny(d) => ("deny", d),
        }
    }

    #[pyo3(signature = (header = "", query = ""))]
    fn api_key_allows(&self, header: &str, query: &str) -> bool {
        self.inner.read().unwrap().api_key_allows(header, query)
    }

    /// `[(name, built_in)]`, in order.
    fn middleware_chain(&self) -> Vec<(String, bool)> {
        self.inner
            .read()
            .unwrap()
            .middleware_chain()
            .iter()
            .map(|l| (l.config.name.clone(), l.middleware.is_some()))
            .collect()
    }

    /// Asks built-in middleware `index` about a request; returns
    /// `(headers, context, respond)`, where `respond` is None (go on) or
    /// `(status, content_type, body_bytes)`.
    fn middleware_before(
        &self,
        py: Python<'_>,
        index: usize,
        request: &Bound<'_, PyDict>,
    ) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
        let mw = self.built_in(index)?;
        middleware_before(py, mw.as_ref(), request)
    }

    /// Tells built-in middleware `index` how the request was answered.
    fn middleware_after(
        &self,
        py: Python<'_>,
        index: usize,
        request: &Bound<'_, PyDict>,
        status: u16,
        body: &[u8],
        duration_ms: f64,
    ) -> PyResult<()> {
        let mw = self.built_in(index)?;
        middleware_after(py, mw.as_ref(), request, status, body, duration_ms);
        Ok(())
    }

    /// A built-in middleware made from one `middlewares:` entry (`{name,
    /// config, excludes, includes}`), outside any blueprint. Its rejections
    /// carry the error but run no `exceptions` handlers: the host does that.
    fn build_middleware(&self, config: &Bound<'_, PyAny>) -> PyResult<CoreMiddleware> {
        let value = from_py(config)?;
        let mut cfg = MiddlewareConfig::from_value(&value)
            .ok_or_else(|| PyValueError::new_err("a middleware config is a dict with a name"))?;
        cfg.exceptions.clear();
        let (executor, dispatcher) = {
            let engine = self.inner.read().unwrap();
            let executor: Arc<dyn IActivityExecutor> = engine.activity_executor.clone();
            (executor, engine.async_dispatcher.clone())
        };
        // The executor and dispatcher run a response audit's activity.
        let mw = build_middleware(&cfg, Some(executor), Some(dispatcher)).ok_or_else(|| {
            PyKeyError::new_err(format!("no built-in middleware named '{}'", cfg.name))
        })?;
        Ok(CoreMiddleware { mw })
    }

    /// The client's IP: `X-Forwarded-For` / `X-Real-IP` count only when the
    /// peer is one of the engine's `trusted_proxies`.
    #[pyo3(signature = (peer, forwarded_for = "", real_ip = ""))]
    fn client_ip(&self, peer: &str, forwarded_for: &str, real_ip: &str) -> String {
        let trusted: Vec<IpNet> = self
            .inner
            .read()
            .unwrap()
            .options()
            .trusted_proxies
            .iter()
            .filter_map(|p| IpNet::parse(p).ok())
            .collect();
        octopus_core::net::client_ip(peer, forwarded_for, real_ip, &trusted)
    }

    fn start_worker(&self) -> PyResult<()> {
        self.inner
            .read()
            .unwrap()
            .start_embedded_worker()
            .map_err(PyValueError::new_err)
    }

    fn stop_worker(&self) {
        self.inner.read().unwrap().stop_embedded_worker();
    }
}

/// The visualizer page.
#[pyfunction]
#[pyo3(signature = (route_prefix, default_domain, activity_language = "Python"))]
fn visualizer_html(route_prefix: &str, default_domain: &str, activity_language: &str) -> String {
    render_visualizer_html_for(default_domain, route_prefix, activity_language)
}

/// A built-in middleware on its own (`Engine.build_middleware`).
#[pyclass]
struct CoreMiddleware {
    mw: Arc<dyn Middleware>,
}

#[pymethods]
impl CoreMiddleware {
    /// `(headers, context, respond)`, as `Engine.middleware_before`.
    fn before(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyDict>,
    ) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
        middleware_before(py, self.mw.as_ref(), request)
    }

    fn after(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyDict>,
        status: u16,
        body: &[u8],
        duration_ms: f64,
    ) -> PyResult<()> {
        middleware_after(py, self.mw.as_ref(), request, status, body, duration_ms);
        Ok(())
    }
}

fn middleware_before(
    py: Python<'_>,
    mw: &dyn Middleware,
    request: &Bound<'_, PyDict>,
) -> PyResult<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> {
    let req = OwnedRequest::from_py(request)?;
    let d = py.detach(|| mw.before(&req.meta()));
    let headers = PyDict::new(py);
    for (k, v) in &d.headers {
        headers.set_item(k, v)?;
    }
    let respond = match d.respond {
        None => py.None(),
        Some(r) => (
            r.status,
            r.content_type,
            pyo3::types::PyBytes::new(py, &r.body),
        )
            .into_pyobject(py)?
            .into_any()
            .unbind(),
    };
    Ok((
        headers.into_any().unbind(),
        map_to_py(py, &d.context)?.into_any().unbind(),
        respond,
    ))
}

fn middleware_after(
    py: Python<'_>,
    mw: &dyn Middleware,
    request: &Bound<'_, PyDict>,
    status: u16,
    body: &[u8],
    duration_ms: f64,
) {
    let Ok(req) = OwnedRequest::from_py(request) else {
        return;
    };
    let body = body.to_vec();
    py.detach(|| {
        mw.after(
            &req.meta(),
            &ResponseMeta {
                status,
                body: &body,
                duration_ms,
            },
        )
    });
}

#[pyclass]
struct Subscription {
    rx: Mutex<Option<Receiver<LiveEvent>>>,
    cancel: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[pymethods]
impl Subscription {
    /// The next event, waiting up to `timeout` seconds (with the GIL
    /// released). None when nothing arrived or the subscription is closed.
    #[pyo3(signature = (timeout = 1.0))]
    fn next(&self, py: Python<'_>, timeout: f64) -> PyResult<Py<PyAny>> {
        let got = py.detach(|| {
            let guard = self.rx.lock().unwrap();
            let rx = guard.as_ref()?;
            rx.recv_timeout(Duration::from_secs_f64(timeout.max(0.0)))
                .ok()
        });
        match got {
            Some(ev) => {
                let d = PyDict::new(py);
                d.set_item("event", ev.event)?;
                d.set_item("data", to_py(py, &ev.data)?)?;
                Ok(d.into_any().unbind())
            }
            None => Ok(py.None()),
        }
    }

    fn close(&self) {
        self.rx.lock().unwrap().take();
        if let Some(cancel) = self.cancel.lock().unwrap().take() {
            cancel();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.get_mut().unwrap().take() {
            cancel();
        }
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Engine>()?;
    m.add_class::<Subscription>()?;
    m.add_class::<CoreMiddleware>()?;
    m.add_function(wrap_pyfunction!(visualizer_html, m)?)?;
    m.add("OctopusError", m.py().get_type::<OctopusError>())?;
    m.add("CORE_VERSION", octopus_core::VERSION)?;
    m.add("CONTRACT_VERSION", octopus_core::CONTRACT_VERSION)?;
    Ok(())
}
