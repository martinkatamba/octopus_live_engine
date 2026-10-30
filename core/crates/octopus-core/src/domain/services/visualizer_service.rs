use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::domain::models::{
    APIEndpointConfig, ExecutionState, ExecutionStatus, MiddlewareConfig, WorkflowBlueprint,
    WorkflowStep,
};
use crate::domain::ports::{IBlueprintRepository, IStateStore, IVisualizerCompiler};
use crate::domain::services::condition_evaluator::ConditionEvaluator;
use crate::domain::services::masking_service::mask_payload;
use crate::domain::services::middleware::glob_match;
use crate::json::{Value, ValueMap};
use crate::vmap;

#[derive(Clone, Debug)]
struct PrevItem {
    id: String,
    label: String,
    color: String,
    executed: bool,
}

#[derive(Clone, Debug)]
struct ExcConfigItem {
    activity: String,
    exception: String,
    code: i32,
    is_async: bool,
    is_middleware: bool,
    source_step_id: String,
    source_name: String,
    message: String,
}

pub struct VisualizerService {
    blueprint_repo: Arc<dyn IBlueprintRepository>,
    state_store: RwLock<Arc<dyn IStateStore>>,
}

impl VisualizerService {
    pub fn new(repo: Arc<dyn IBlueprintRepository>, store: Arc<dyn IStateStore>) -> Arc<Self> {
        Arc::new(Self {
            blueprint_repo: repo,
            state_store: RwLock::new(store),
        })
    }

    pub fn set_state_store(&self, store: Arc<dyn IStateStore>) {
        *self.state_store.write().unwrap() = store;
    }
}

fn extract_runtime_variable(state: Option<&ExecutionState>, var_name: &str) -> Value {
    let Some(state) = state else {
        return Value::Null;
    };
    if var_name.is_empty() {
        return Value::Null;
    }
    let eval = ConditionEvaluator::new();
    if let Some(val) = eval.get_nested_field(&state.data, var_name) {
        if !val.is_null() {
            return val;
        }
    }
    for t in state.traces.iter().rev() {
        if let Some(val) = eval.get_nested_field(&t.output_payload, var_name) {
            if !val.is_null() {
                return val;
            }
        }
        if let Some(val) = eval.get_nested_field(&t.input_payload, var_name) {
            if !val.is_null() {
                return val;
            }
        }
    }
    Value::Null
}

impl IVisualizerCompiler for VisualizerService {
    fn list_blueprints(&self) -> Result<Vec<WorkflowBlueprint>, String> {
        self.blueprint_repo.list_blueprints()
    }

    fn compile_x6_diagram(
        &self,
        workflow_name: &str,
        job_id: Option<&str>,
        route: Option<&str>,
        method: Option<&str>,
    ) -> Result<ValueMap, String> {
        let bp = self
            .blueprint_repo
            .get_blueprint(workflow_name)
            .map_err(|e| format!("blueprint '{}' not found: {}", workflow_name, e))?;

        let store = self.state_store.read().unwrap().clone();
        let mut state: Option<ExecutionState> = None;
        if let Some(jid) = job_id.filter(|s| !s.is_empty()) {
            state = store.get_state(jid).ok();
            if state.is_none() {
                let wf_prefix = format!("wf-{}-", workflow_name);
                if let Some(clean_id) = jid.strip_prefix(&wf_prefix) {
                    state = store.get_state(clean_id).ok();
                } else if jid.starts_with("wf-") {
                    let parts: Vec<&str> = jid.splitn(3, '-').collect();
                    if parts.len() == 3 {
                        state = store.get_state(parts[2]).ok();
                    }
                }
            }
        }

        compile_diagram(&bp, job_id, state.as_ref(), route, method)
    }
}

fn ep_effective_method(ep: &APIEndpointConfig) -> &str {
    if ep.method.is_empty() {
        "POST"
    } else {
        ep.method.as_str()
    }
}

fn route_matches_exact(ep_route: &str, target_route: &str) -> bool {
    let a = ep_route.trim_end_matches('/');
    let b = target_route.trim_end_matches('/');
    a.eq_ignore_ascii_case(b)
}

fn route_matches_flexible(ep_route: &str, target_route: &str) -> bool {
    if route_matches_exact(ep_route, target_route) {
        return true;
    }
    if target_route.ends_with(ep_route) || ep_route.ends_with(target_route) {
        return true;
    }
    let a_parts: Vec<&str> = ep_route.trim_matches('/').split('/').collect();
    let b_parts: Vec<&str> = target_route.trim_matches('/').split('/').collect();
    if a_parts.len() == b_parts.len() && !a_parts.is_empty() {
        let all_match = a_parts.iter().zip(b_parts.iter()).all(|(ap, bp)| {
            ap.eq_ignore_ascii_case(bp)
                || (ap.starts_with('{') && ap.ends_with('}'))
                || ap.starts_with(':')
                || (bp.starts_with('{') && bp.ends_with('}'))
                || bp.starts_with(':')
        });
        if all_match {
            return true;
        }
    }
    false
}

fn step_list_contains_trace(steps: &[WorkflowStep], trace_step_ids: &HashSet<String>) -> bool {
    for s in steps {
        if (!s.id.is_empty() && trace_step_ids.contains(&s.id.to_lowercase()))
            || (!s.activity.is_empty() && trace_step_ids.contains(&s.activity.to_lowercase()))
        {
            return true;
        }
        if step_list_contains_trace(&s.get_then_steps(), trace_step_ids)
            || step_list_contains_trace(&s.get_else_steps(), trace_step_ids)
        {
            return true;
        }
    }
    false
}

fn collect_step_exceptions(steps: &[WorkflowStep], exc_configs: &mut Vec<ExcConfigItem>) {
    for step in steps {
        let s_id = if !step.id.is_empty() {
            step.id.clone()
        } else {
            step.activity.clone()
        };
        for exc in &step.exceptions {
            if !exc.activity.is_empty() {
                exc_configs.push(ExcConfigItem {
                    activity: exc.activity.clone(),
                    exception: exc.exception.clone(),
                    code: exc.code,
                    is_async: exc.is_async,
                    is_middleware: false,
                    source_step_id: s_id.clone(),
                    source_name: s_id.clone(),
                    message: exc.message.clone(),
                });
            }
        }
        if !step.get_then_steps().is_empty() {
            collect_step_exceptions(&step.get_then_steps(), exc_configs);
        }
        if !step.get_else_steps().is_empty() {
            collect_step_exceptions(&step.get_else_steps(), exc_configs);
        }
    }
}

fn is_path_excluded(route: &str, excludes: &[String]) -> bool {
    if route.is_empty() || excludes.is_empty() {
        return false;
    }
    let clean_route = route.trim_end_matches('/');
    for raw in excludes {
        let pattern = raw.trim();
        if pattern.is_empty() {
            continue;
        }
        let clean_pattern = pattern.trim_end_matches('/');
        if clean_route == clean_pattern {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if route.starts_with(prefix) || route.contains(prefix) {
                return true;
            }
        }
        if (pattern.contains('*') || pattern.contains('?'))
            && (glob_match(pattern, route) || glob_match(clean_pattern, clean_route))
        {
            return true;
        }
        if route.eq_ignore_ascii_case(pattern)
            || route.ends_with(pattern)
            || pattern.ends_with(route)
        {
            return true;
        }
    }
    false
}

fn is_middleware_active_for_route(m: &MiddlewareConfig, route: &str) -> bool {
    if route.is_empty() || route.eq_ignore_ascii_case("default") {
        return true;
    }
    if is_path_excluded(route, &m.excludes) {
        return false;
    }
    if !m.includes.is_empty() {
        for inc in &m.includes {
            if let Some(prefix) = inc.strip_suffix('*') {
                if route.starts_with(prefix) || route.contains(prefix) {
                    return true;
                }
            }
            if route.eq_ignore_ascii_case(inc) || route.ends_with(inc) || inc.ends_with(route) {
                return true;
            }
        }
        return false;
    }
    true
}

pub fn classify_trace_status(st: &str) -> (String, bool) {
    match st {
        "COMPLETED"
        | "ASYNC_DISPATCHED"
        | "BACKGROUND_DISPATCHED"
        | "DISPATCHED"
        | "ASYNC_COMPLETED"
        | "SIGNAL_RECEIVED"
        | "SIGNAL_TIMED_OUT" => ("COMPLETED".to_string(), true),
        "FAILED" | "ASYNC_FAILED" => ("FAILED".to_string(), true),
        "RUNNING" | "ROLLED_BACK" | "AWAITING_SIGNAL" | "AWAITING_APPROVAL" => {
            (st.to_string(), true)
        }
        "" => ("PENDING".to_string(), false),
        _ => (st.to_string(), false),
    }
}

/// Compiles a blueprint, and optionally one job's state, into the
/// visualizer's X6 graph JSON. A pure function: the host (or
/// `VisualizerService`) looks up the blueprint and the job. `job_id` is the
/// job that was asked for; with no `state`, the graph reports it as missing.
pub fn compile_diagram(
    bp: &WorkflowBlueprint,
    job_id: Option<&str>,
    state: Option<&ExecutionState>,
    route: Option<&str>,
    method: Option<&str>,
) -> Result<ValueMap, String> {
    let mut status_map: HashMap<String, String> = HashMap::new();
    let mut duration_map: HashMap<String, f64> = HashMap::new();
    let mut input_map: HashMap<String, ValueMap> = HashMap::new();
    let mut output_map: HashMap<String, ValueMap> = HashMap::new();
    let mut error_map: HashMap<String, String> = HashMap::new();

    if let Some(st) = state {
        for trace in &st.traces {
            if !trace.step_id.is_empty() {
                status_map.insert(trace.step_id.clone(), trace.status.clone());
                duration_map.insert(trace.step_id.clone(), trace.duration_ms);
                if !trace.input_payload.is_empty() {
                    input_map.insert(trace.step_id.clone(), trace.input_payload.clone());
                }
                if !trace.output_payload.is_empty() {
                    output_map.insert(trace.step_id.clone(), trace.output_payload.clone());
                }
                if !trace.error_message.is_empty() {
                    error_map.insert(trace.step_id.clone(), trace.error_message.clone());
                }
            }
            if !trace.activity_name.is_empty() && trace.step_id.is_empty() {
                status_map.insert(trace.activity_name.clone(), trace.status.clone());
                if !trace.input_payload.is_empty() {
                    input_map.insert(trace.activity_name.clone(), trace.input_payload.clone());
                }
                if !trace.output_payload.is_empty() {
                    output_map.insert(trace.activity_name.clone(), trace.output_payload.clone());
                }
            }
        }
    }

    let mut routes_list: Vec<Value> = Vec::with_capacity(bp.api.len());
    for ep in &bp.api {
        let m = if ep.method.is_empty() {
            "POST".to_string()
        } else {
            ep.method.to_uppercase()
        };
        routes_list.push(Value::Object(vmap! {
            "route" => ep.route.clone(),
            "method" => m,
            "summary" => ep.summary.clone(),
            "description" => ep.description.clone(),
            "step_count" => ep.steps.len() as i64,
        }));
    }

    let mut selected_ep: Option<&APIEndpointConfig> = None;
    let mut selected_route = String::new();
    let mut selected_method = String::new();

    // 1. Match by live state route + method or trace step IDs
    if let Some(st) = state {
        let mut trace_step_ids = HashSet::new();
        for t in &st.traces {
            if !t.step_id.is_empty() {
                trace_step_ids.insert(t.step_id.to_lowercase());
            }
            if !t.activity_name.is_empty() {
                trace_step_ids.insert(t.activity_name.to_lowercase());
            }
        }

        let job_method = if !st.method.is_empty() {
            Some(st.method.as_str())
        } else {
            method.filter(|m| !m.is_empty())
        };

        if !st.route.is_empty() && !bp.api.is_empty() {
            // Pass 1a: Exact route + method match
            if let Some(jm) = job_method {
                for ep in &bp.api {
                    if route_matches_exact(&ep.route, &st.route)
                        && ep_effective_method(ep).eq_ignore_ascii_case(jm)
                    {
                        selected_ep = Some(ep);
                        break;
                    }
                }
            }
            // Pass 1b: Flexible/template route + method match
            if selected_ep.is_none() {
                if let Some(jm) = job_method {
                    for ep in &bp.api {
                        if route_matches_flexible(&ep.route, &st.route)
                            && ep_effective_method(ep).eq_ignore_ascii_case(jm)
                        {
                            selected_ep = Some(ep);
                            break;
                        }
                    }
                }
            }
            // Pass 1c: Route + trace step IDs match
            if selected_ep.is_none() && !trace_step_ids.is_empty() {
                for ep in &bp.api {
                    if route_matches_flexible(&ep.route, &st.route)
                        && step_list_contains_trace(&ep.steps, &trace_step_ids)
                    {
                        selected_ep = Some(ep);
                        break;
                    }
                }
            }
            // Pass 1d: Route-only fallback
            if selected_ep.is_none() {
                for ep in &bp.api {
                    if route_matches_flexible(&ep.route, &st.route) {
                        selected_ep = Some(ep);
                        break;
                    }
                }
            }
        }

        if selected_ep.is_none() && !bp.api.is_empty() && !trace_step_ids.is_empty() {
            for ep in &bp.api {
                if step_list_contains_trace(&ep.steps, &trace_step_ids) {
                    selected_ep = Some(ep);
                    break;
                }
            }
        }
    }

    // 2. Match by explicit requested route / method
    if selected_ep.is_none() {
        if let Some(req_route) = route.filter(|r| !r.is_empty()) {
            let req_method = method.filter(|m| !m.is_empty());
            // Pass 2a: Exact route + method match
            for ep in &bp.api {
                let match_route = route_matches_exact(&ep.route, req_route);
                let match_method = req_method
                    .map(|m| ep_effective_method(ep).eq_ignore_ascii_case(m))
                    .unwrap_or(true);
                if match_route && match_method {
                    selected_ep = Some(ep);
                    break;
                }
            }
            // Pass 2b: Flexible route + method match
            if selected_ep.is_none() {
                for ep in &bp.api {
                    let match_route = route_matches_flexible(&ep.route, req_route);
                    let match_method = req_method
                        .map(|m| ep_effective_method(ep).eq_ignore_ascii_case(m))
                        .unwrap_or(true);
                    if match_route && match_method {
                        selected_ep = Some(ep);
                        break;
                    }
                }
            }
        }
    }

    // 3. Resolve the step list to render
    let steps_to_visualize: Vec<WorkflowStep> =
        if let Some(ep) = selected_ep.filter(|ep| !ep.steps.is_empty()) {
            selected_route = ep.route.clone();
            selected_method = if ep.method.is_empty() {
                "POST".to_string()
            } else {
                ep.method.to_uppercase()
            };
            ep.steps.clone()
        } else if !bp.steps.is_empty() {
            selected_route = "default".to_string();
            selected_method = "EXECUTE".to_string();
            bp.steps.clone()
        } else if !bp.api.is_empty() && !bp.api[0].steps.is_empty() {
            let ep = &bp.api[0];
            selected_ep = Some(ep);
            selected_route = ep.route.clone();
            selected_method = if ep.method.is_empty() {
                "POST".to_string()
            } else {
                ep.method.to_uppercase()
            };
            ep.steps.clone()
        } else {
            Vec::new()
        };

    let has_live_execution = state.is_some();
    let start_status = if has_live_execution {
        "COMPLETED"
    } else {
        "PENDING"
    };

    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();

    // 1. Start Node
    let mut start_node = vmap! {
        "id" => "node__start",
        "shape" => "octopus-start-node",
        "label" => "START",
        "width" => 120i64,
        "height" => 50i64,
        "data" => Value::Object(vmap! {
            "step_id" => "start",
            "type" => "start",
            "title" => "Workflow Start",
            "status" => start_status,
            "executed" => has_live_execution,
        }),
    };
    if has_live_execution {
        start_node.insert(
            "attrs".to_string(),
            Value::Object(vmap! {
                "body" => Value::Object(vmap! {
                    "rx" => 25i64,
                    "ry" => 25i64,
                    "fill" => "#064e3b",
                    "stroke" => "#10b981",
                    "strokeWidth" => 3i64,
                    "filter" => "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    "opacity" => 1.0f64,
                }),
                "label" => Value::Object(vmap! {
                    "fill" => "#6ee7b7",
                    "fontSize" => 13i64,
                    "fontWeight" => 700i64,
                }),
            }),
        );
    }
    nodes.push(Value::Object(start_node));

    // 2. Ingress Middleware Guards Node
    let mut pre_middlewares: Vec<MiddlewareConfig> = Vec::new();
    let mut post_middlewares: Vec<MiddlewareConfig> = Vec::new();
    for m in &bp.middlewares {
        if is_middleware_active_for_route(m, &selected_route) {
            if m.r#type.eq_ignore_ascii_case("post-request") {
                post_middlewares.push(m.clone());
            } else {
                pre_middlewares.push(m.clone());
            }
        }
    }

    let has_ingress_guards = !pre_middlewares.is_empty();
    let has_egress_guards = !post_middlewares.is_empty();
    let initial_prev: Vec<PrevItem>;

    if has_ingress_guards {
        let mut mw_exc_names: HashSet<String> = HashSet::new();
        for m in &pre_middlewares {
            for e in &m.exceptions {
                if !e.exception.is_empty() {
                    mw_exc_names.insert(e.exception.to_lowercase());
                }
            }
        }

        let mut is_ingress_failed = false;
        if let Some(st) = state {
            if st.status == ExecutionStatus::Failed {
                let state_err = st.error.as_deref().unwrap_or("").to_lowercase();
                for me in &mw_exc_names {
                    if state_err.contains(me) {
                        is_ingress_failed = true;
                        break;
                    }
                }
                if !is_ingress_failed {
                    for t in &st.traces {
                        let t_err = t.error_message.to_lowercase();
                        for me in &mw_exc_names {
                            if t_err.contains(me) {
                                is_ingress_failed = true;
                                break;
                            }
                        }
                        if is_ingress_failed {
                            break;
                        }
                    }
                }
            }
        }

        let ingress_status = if is_ingress_failed {
            "FAILED"
        } else if has_live_execution {
            "COMPLETED"
        } else {
            "PENDING"
        };

        let mw_names_list: Vec<Value> = pre_middlewares
            .iter()
            .filter(|m| !m.name.is_empty())
            .map(|m| Value::String(m.name.clone()))
            .collect();

        let mut ingress_node = vmap! {
            "id" => "node__ingress_guards",
            "shape" => "octopus-activity-node",
            "label" => "🛡️ Ingress Middleware Guards",
            "width" => 220i64,
            "height" => 55i64,
            "data" => Value::Object(vmap! {
                "step_id" => "ingress_guards",
                "type" => "ingress_guards",
                "title" => "🛡️ Ingress Middleware Guards",
                "description" => "Pre-execution security filters, JWT authentication, IP whitelisting, and rate-limiting guards executed prior to workflow pipeline dispatch.",
                "status" => ingress_status,
                "executed" => has_live_execution,
                "middlewares" => Value::Array(mw_names_list),
            }),
        };

        if is_ingress_failed {
            ingress_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#ef4444",
                        "strokeWidth" => 3i64,
                        "fill" => "#2d1215",
                        "filter" => "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                        "opacity" => 1.0f64,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => "#fca5a5",
                        "fontWeight" => 700i64,
                    }),
                }),
            );
        } else if has_live_execution {
            ingress_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#10b981",
                        "strokeWidth" => 3i64,
                        "fill" => "#0f2e22",
                        "filter" => "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity" => 1.0f64,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => "#6ee7b7",
                        "fontWeight" => 600i64,
                    }),
                }),
            );
        }
        nodes.push(Value::Object(ingress_node));

        let start_to_ingress_stroke = if is_ingress_failed {
            "#ef4444"
        } else if has_live_execution {
            "#10b981"
        } else {
            "#6366f1"
        };

        edges.push(Value::Object(vmap! {
            "id" => "edge__node__start__to__node__ingress_guards",
            "source" => "node__start",
            "target" => "node__ingress_guards",
            "attrs" => Value::Object(vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => start_to_ingress_stroke,
                    "strokeWidth" => 2i64,
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 8i64,
                        "height" => 6i64,
                        "fill" => start_to_ingress_stroke,
                    }),
                }),
            }),
        }));

        initial_prev = vec![PrevItem {
            id: "node__ingress_guards".to_string(),
            label: "✅ Quota & Security OK".to_string(),
            color: "#10b981".to_string(),
            executed: has_live_execution && !is_ingress_failed,
        }];
    } else {
        initial_prev = vec![PrevItem {
            id: "node__start".to_string(),
            label: String::new(),
            color: "#6366f1".to_string(),
            executed: has_live_execution,
        }];
    }

    let branch_taken = |raw_step_id: &str, step: &WorkflowStep| -> (bool, bool, bool) {
        if let Some(out) = output_map.get(raw_step_id) {
            if let Some(r) = out.get("result").and_then(|v| v.as_bool()) {
                if out.get("branch").is_some() {
                    return (r, !r, true);
                }
            }
        }
        let ran = |list: &[WorkflowStep]| -> bool {
            for s in list {
                if let Some(st) = status_map.get(&s.id) {
                    if classify_trace_status(st).1 {
                        return true;
                    }
                }
                if !s.activity.is_empty() {
                    if let Some(st) = status_map.get(&s.activity) {
                        if classify_trace_status(st).1 {
                            return true;
                        }
                    }
                }
            }
            false
        };
        let then_ran = ran(&step.get_then_steps());
        let else_ran = ran(&step.get_else_steps());
        (then_ran, else_ran, then_ran || else_ran)
    };

    #[allow(clippy::too_many_arguments)]
    fn process_step_list(
        bp: &WorkflowBlueprint,
        state: Option<&ExecutionState>,
        status_map: &HashMap<String, String>,
        input_map: &HashMap<String, ValueMap>,
        output_map: &HashMap<String, ValueMap>,
        error_map: &HashMap<String, String>,
        branch_taken: &dyn Fn(&str, &WorkflowStep) -> (bool, bool, bool),
        nodes: &mut Vec<Value>,
        edges: &mut Vec<Value>,
        step_list: &[WorkflowStep],
        prev_heads: Vec<PrevItem>,
        scope_prefix: &str,
        has_live_execution: bool,
    ) -> Vec<PrevItem> {
        let mut curr_prev = prev_heads;

        for (idx, step) in step_list.iter().enumerate() {
            let raw_step_id = if step.id.is_empty() {
                format!("step_{}", idx)
            } else {
                step.id.clone()
            };
            let node_id = if scope_prefix.is_empty() {
                format!("node__{}", raw_step_id)
            } else {
                format!("node__{}__{}", scope_prefix, raw_step_id)
            };

            let (mut step_status, mut executed) = if let Some(st) = status_map.get(&raw_step_id) {
                classify_trace_status(st)
            } else if !step.activity.is_empty()
                && status_map
                    .get(&step.activity)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            {
                classify_trace_status(&status_map[&step.activity])
            } else if state
                .map(|s| s.current_step_id == raw_step_id)
                .unwrap_or(false)
            {
                ("RUNNING".to_string(), true)
            } else if has_live_execution {
                ("SKIPPED".to_string(), false)
            } else {
                ("PENDING".to_string(), false)
            };

            let is_loop = step.r#loop.is_some() || step.for_each.is_some();
            let is_async = step.r#async || step.is_async;
            let has_branches = !step.get_then_steps().is_empty()
                || !step.get_else_steps().is_empty()
                || !step.branching_step.is_empty()
                || step.condition.is_some()
                || step.conditions.is_some();

            let mut shape = "octopus-activity-node";
            let mut type_str = "activity";
            let mut node_label = raw_step_id.clone();

            if is_loop {
                shape = "octopus-loop-node";
                type_str = "loop";
                node_label = format!("⚏ {}\n[Loop Iteration]", raw_step_id);
            } else if is_async {
                shape = "octopus-activity-node";
                type_str = "async";
                node_label = format!("⚡ {}\n[Async Background]", raw_step_id);
            } else if has_branches {
                shape = "octopus-branch-node";
                type_str = "branch";
                node_label = format!("⚏ {}", raw_step_id);
            } else if !step.human_step.is_empty() && step.await_signal.is_empty() {
                shape = "octopus-human-node";
                type_str = "human";
            } else if !step.await_signal.is_empty() {
                shape = "octopus-human-node";
                type_str = "signal";
                node_label = format!("⏳ {}\n[waits for {}]", raw_step_id, step.await_signal);
            }

            let (fields, mask_val) = bp.get_mask_config(Some(step));
            let inp_raw = input_map
                .get(&raw_step_id)
                .or_else(|| input_map.get(&step.activity));
            let out_raw = output_map
                .get(&raw_step_id)
                .or_else(|| output_map.get(&step.activity));

            let inp_masked = inp_raw
                .map(|m| Value::Object(mask_payload(m, &fields, &mask_val)))
                .unwrap_or(Value::Null);
            let out_masked = out_raw
                .map(|m| Value::Object(mask_payload(m, &fields, &mask_val)))
                .unwrap_or(Value::Null);

            let mut node_data = vmap! {
                "step_id" => raw_step_id.clone(),
                "title" => raw_step_id.clone(),
                "activity" => step.activity.clone(),
                "status" => step_status.clone(),
                "executed" => executed,
                "type" => type_str,
                "is_loop" => is_loop,
                "loop" => step.r#loop.as_ref().map(|m| Value::Object(m.clone())).unwrap_or(Value::Null),
                "for_each" => step.for_each.as_ref().map(|m| Value::Object(m.clone())).unwrap_or(Value::Null),
                "is_async" => is_async,
                "is_output" => step.is_output || step.output,
                "condition" => step.condition.as_ref().map(|c| c.to_value()).unwrap_or(Value::Null),
                "conditions" => step.conditions.as_ref().map(|c| c.to_value()).unwrap_or(Value::Null),
                "compensation" => step.compensation.clone(),
                "description" => step.description.clone(),
                "timeout_seconds" => step.timeout_seconds as i64,
                "signal" => step.signal_name(),
                "index" => (idx + 1) as i64,
                "input_payload" => inp_masked,
                "output_payload" => out_masked,
                "error" => error_map.get(&raw_step_id).cloned().unwrap_or_default(),
            };

            if has_branches {
                let then_step_ids: Vec<Value> = step
                    .get_then_steps()
                    .iter()
                    .map(|s| {
                        Value::String(if !s.id.is_empty() {
                            s.id.clone()
                        } else {
                            s.activity.clone()
                        })
                    })
                    .collect();
                let else_step_ids: Vec<Value> = step
                    .get_else_steps()
                    .iter()
                    .map(|s| {
                        Value::String(if !s.id.is_empty() {
                            s.id.clone()
                        } else {
                            s.activity.clone()
                        })
                    })
                    .collect();

                let (then_executed, else_executed, _) = branch_taken(&raw_step_id, step);
                if then_executed || else_executed {
                    executed = true;
                    step_status = "COMPLETED".to_string();
                    node_data.insert("executed".to_string(), Value::Bool(true));
                    node_data.insert("status".to_string(), Value::String("COMPLETED".to_string()));
                }

                let mut branch_info = vmap! {
                    "has_branches" => true,
                    "condition" => step.condition.as_ref().map(|c| c.to_value()).unwrap_or(Value::Null),
                    "conditions" => step.conditions.as_ref().map(|c| c.to_value()).unwrap_or(Value::Null),
                    "then_step_ids" => Value::Array(then_step_ids),
                    "else_step_ids" => Value::Array(else_step_ids),
                };

                if has_live_execution && (then_executed || else_executed || executed) {
                    let verdict = if then_executed {
                        "TRUE"
                    } else if else_executed {
                        "FALSE"
                    } else {
                        "UNKNOWN"
                    };
                    let verdict_label = if then_executed {
                        "TRUE (Match)"
                    } else if else_executed && step.get_else_steps().is_empty() {
                        "FALSE (Skip)"
                    } else if else_executed {
                        "FALSE (Else)"
                    } else {
                        "EXECUTED"
                    };
                    let b_taken = if then_executed {
                        "THEN"
                    } else if else_executed {
                        "ELSE"
                    } else {
                        "NONE"
                    };

                    let mut eval_info = vmap! {
                        "evaluated" => true,
                        "verdict" => verdict,
                        "verdict_label" => verdict_label,
                        "branch_taken" => b_taken,
                    };

                    if let Some(ref cond) = step.condition {
                        let var_name = if !cond.variable.is_empty() {
                            &cond.variable
                        } else {
                            &cond.field
                        };
                        if !var_name.is_empty() {
                            eval_info.insert(
                                "variable_name".to_string(),
                                Value::String(var_name.clone()),
                            );
                            eval_info.insert(
                                "runtime_value".to_string(),
                                extract_runtime_variable(state, var_name),
                            );
                            eval_info.insert("expected_value".to_string(), cond.value.clone());
                            let op = if cond.operator.is_empty() {
                                "=="
                            } else {
                                &cond.operator
                            };
                            eval_info.insert("operator".to_string(), Value::String(op.to_string()));
                        }
                        if !cond.rule.is_empty() {
                            eval_info.insert("rule".to_string(), Value::String(cond.rule.clone()));
                        }
                    }
                    branch_info.insert("evaluation".to_string(), Value::Object(eval_info));
                }
                node_data.insert("branch_info".to_string(), Value::Object(branch_info));
            }

            let mut node = vmap! {
                "id" => node_id.clone(),
                "shape" => shape,
                "width" => 220i64,
                "height" => 75i64,
                "label" => node_label,
                "data" => Value::Object(node_data),
            };

            if step_status == "FAILED" || step_status == "ROLLED_BACK" {
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "stroke" => "#ef4444",
                            "strokeWidth" => 3i64,
                            "fill" => "#2d1215",
                            "filter" => "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                            "opacity" => 1.0f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#fca5a5",
                            "fontWeight" => 700i64,
                        }),
                    }),
                );
            } else if step_status == "TIMED_OUT" {
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "stroke" => "#f97316",
                            "strokeWidth" => 3i64,
                            "fill" => "#2b180d",
                            "filter" => "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
                            "opacity" => 1.0f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#fdba74",
                            "fontWeight" => 700i64,
                        }),
                    }),
                );
            } else if step_status == "AWAITING_APPROVAL"
                || step_status == "RUNNING"
                || step_status == "AWAITING_SIGNAL"
            {
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "stroke" => "#f59e0b",
                            "strokeWidth" => 3i64,
                            "fill" => "#2a1d08",
                            "filter" => "drop-shadow(0 0 14px rgba(245, 158, 11, 0.85))",
                            "opacity" => 1.0f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#fde68a",
                            "fontWeight" => 700i64,
                        }),
                    }),
                );
            } else if executed {
                let mut body_attrs = vmap! {
                    "stroke" => "#10b981",
                    "strokeWidth" => 3i64,
                    "fill" => "#0f2e22",
                    "filter" => "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                    "opacity" => 1.0f64,
                };
                if is_loop {
                    body_attrs.insert(
                        "strokeDasharray".to_string(),
                        Value::String("5 4".to_string()),
                    );
                }
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(body_attrs),
                        "label" => Value::Object(vmap! {
                            "fill" => "#6ee7b7",
                            "fontWeight" => 600i64,
                        }),
                    }),
                );
            } else if is_loop {
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "stroke" => "#38bdf8",
                            "strokeWidth" => 2i64,
                            "strokeDasharray" => "5 4",
                            "fill" => "#0c2333",
                            "filter" => "drop-shadow(0 0 12px rgba(56, 189, 248, 0.45))",
                            "opacity" => if has_live_execution { 0.35f64 } else { 1.0f64 },
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#7dd3fc",
                            "fontWeight" => 600i64,
                            "opacity" => if has_live_execution { 0.45f64 } else { 1.0f64 },
                        }),
                    }),
                );
            } else if has_live_execution {
                node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "stroke" => "#334155",
                            "strokeWidth" => 1.5f64,
                            "fill" => "#111827",
                            "opacity" => 0.35f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#64748b",
                            "opacity" => 0.45f64,
                        }),
                    }),
                );
            }

            nodes.push(Value::Object(node));

            // Connect predecessors to this node
            for p in &curr_prev {
                let edge_attrs = if p.executed && executed {
                    vmap! {
                        "line" => Value::Object(vmap! {
                            "stroke" => "#10b981",
                            "strokeWidth" => 3.5f64,
                            "targetMarker" => Value::Object(vmap! {
                                "name" => "block",
                                "width" => 10i64,
                                "height" => 8i64,
                                "fill" => "#10b981",
                            }),
                            "filter" => "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                            "opacity" => 1.0f64,
                        }),
                    }
                } else if p.executed
                    && (step_status == "RUNNING" || step_status == "AWAITING_APPROVAL")
                {
                    vmap! {
                        "line" => Value::Object(vmap! {
                            "stroke" => "#f59e0b",
                            "strokeWidth" => 3.5f64,
                            "strokeDasharray" => "6 4",
                            "targetMarker" => Value::Object(vmap! {
                                "name" => "block",
                                "width" => 10i64,
                                "height" => 8i64,
                                "fill" => "#f59e0b",
                            }),
                            "filter" => "drop-shadow(0 0 10px rgba(245, 158, 11, 0.75))",
                            "opacity" => 1.0f64,
                        }),
                    }
                } else if has_live_execution {
                    vmap! {
                        "line" => Value::Object(vmap! {
                            "stroke" => "#1e293b",
                            "strokeWidth" => 1.5f64,
                            "strokeDasharray" => "4 4",
                            "targetMarker" => Value::Object(vmap! {
                                "name" => "block",
                                "width" => 6i64,
                                "height" => 4i64,
                                "fill" => "#334155",
                            }),
                            "opacity" => 0.25f64,
                        }),
                    }
                } else {
                    vmap! {
                        "line" => Value::Object(vmap! {
                            "stroke" => "#64748b",
                            "strokeWidth" => 2i64,
                            "targetMarker" => Value::Object(vmap! {
                                "name" => "block",
                                "width" => 8i64,
                                "height" => 6i64,
                                "fill" => "#64748b",
                            }),
                            "opacity" => 1.0f64,
                        }),
                    }
                };

                let mut edge_obj = vmap! {
                    "id" => format!("edge_{}_to_{}", p.id, node_id),
                    "source" => p.id.clone(),
                    "target" => node_id.clone(),
                    "attrs" => Value::Object(edge_attrs),
                };
                if !p.label.is_empty() {
                    let label_color = if p.executed && executed {
                        "#10b981".to_string()
                    } else {
                        p.color.clone()
                    };
                    edge_obj.insert(
                        "labels".to_string(),
                        Value::Array(vec![Value::Object(vmap! {
                            "attrs" => Value::Object(vmap! {
                                "text" => Value::Object(vmap! {
                                    "text" => p.label.clone(),
                                    "fill" => label_color,
                                    "fontSize" => 11i64,
                                    "fontWeight" => "bold",
                                }),
                            }),
                        })]),
                    );
                }
                edges.push(Value::Object(edge_obj));
            }

            if has_branches {
                let mut branch_outs: Vec<PrevItem> = Vec::new();
                let then_steps = step.get_then_steps();
                let else_steps = step.get_else_steps();
                let (then_taken, else_taken, _) = branch_taken(&raw_step_id, step);

                if !then_steps.is_empty() {
                    let then_scope = if scope_prefix.is_empty() {
                        format!("{}_then", raw_step_id)
                    } else {
                        format!("{}_{}_then", scope_prefix, raw_step_id)
                    };
                    let then_heads = vec![PrevItem {
                        id: node_id.clone(),
                        label: "TRUE (Match)".to_string(),
                        color: "#10b981".to_string(),
                        executed: executed && then_taken,
                    }];
                    let then_ends = process_step_list(
                        bp,
                        state,
                        status_map,
                        input_map,
                        output_map,
                        error_map,
                        branch_taken,
                        nodes,
                        edges,
                        &then_steps,
                        then_heads,
                        &then_scope,
                        has_live_execution,
                    );
                    branch_outs.extend(then_ends);
                }
                if !else_steps.is_empty() {
                    let else_scope = if scope_prefix.is_empty() {
                        format!("{}_else", raw_step_id)
                    } else {
                        format!("{}_{}_else", scope_prefix, raw_step_id)
                    };
                    let else_heads = vec![PrevItem {
                        id: node_id.clone(),
                        label: "FALSE (Else)".to_string(),
                        color: "#38bdf8".to_string(),
                        executed: executed && else_taken,
                    }];
                    let else_ends = process_step_list(
                        bp,
                        state,
                        status_map,
                        input_map,
                        output_map,
                        error_map,
                        branch_taken,
                        nodes,
                        edges,
                        &else_steps,
                        else_heads,
                        &else_scope,
                        has_live_execution,
                    );
                    branch_outs.extend(else_ends);
                }
                if then_steps.is_empty() {
                    branch_outs.push(PrevItem {
                        id: node_id.clone(),
                        label: "TRUE (Skip)".to_string(),
                        color: "#94a3b8".to_string(),
                        executed: executed && then_taken,
                    });
                }
                if else_steps.is_empty() {
                    branch_outs.push(PrevItem {
                        id: node_id.clone(),
                        label: "FALSE (Skip)".to_string(),
                        color: "#94a3b8".to_string(),
                        executed: executed && else_taken,
                    });
                }
                curr_prev = branch_outs;
            } else {
                curr_prev = vec![PrevItem {
                    id: node_id,
                    label: String::new(),
                    color: "#64748b".to_string(),
                    executed,
                }];
            }
        }

        curr_prev
    }

    let mut final_heads = process_step_list(
        bp,
        state,
        &status_map,
        &input_map,
        &output_map,
        &error_map,
        &branch_taken,
        &mut nodes,
        &mut edges,
        &steps_to_visualize,
        initial_prev,
        "",
        has_live_execution,
    );

    // 3. Exception Handlers & Failure Terminal Nodes
    let mut exc_configs: Vec<ExcConfigItem> = Vec::new();
    if let Some(gh) = bp
        .error_handling
        .as_ref()
        .and_then(|eh| eh.global_handler.as_ref())
    {
        if !gh.activity.is_empty() {
            let g_code = if gh.code <= 0 { 500 } else { gh.code };
            let g_msg = if gh.message.is_empty() {
                "Internal Server Error".to_string()
            } else {
                gh.message.clone()
            };
            exc_configs.push(ExcConfigItem {
                activity: gh.activity.clone(),
                exception: "*".to_string(),
                code: g_code,
                is_async: gh.is_async,
                is_middleware: false,
                source_step_id: String::new(),
                source_name: "GlobalExceptionHandler".to_string(),
                message: g_msg,
            });
        }
    }

    for mw in &pre_middlewares {
        for exc in &mw.exceptions {
            if !exc.activity.is_empty() {
                exc_configs.push(ExcConfigItem {
                    activity: exc.activity.clone(),
                    exception: exc.exception.clone(),
                    code: exc.code,
                    is_async: exc.is_async,
                    is_middleware: true,
                    source_step_id: String::new(),
                    source_name: mw.name.clone(),
                    message: exc.message.clone(),
                });
            }
        }
    }

    collect_step_exceptions(&steps_to_visualize, &mut exc_configs);

    if let Some(st) = state {
        for trace in &st.traces {
            let s_id = &trace.step_id;
            let a_name = &trace.activity_name;
            let is_already_present = exc_configs
                .iter()
                .any(|e| e.activity.eq_ignore_ascii_case(a_name));
            if (s_id.starts_with("exc_") || a_name.to_lowercase().contains("audit"))
                && !is_already_present
            {
                let err_m = if trace.error_message.is_empty() {
                    "Exception".to_string()
                } else {
                    trace.error_message.clone()
                };
                exc_configs.push(ExcConfigItem {
                    activity: a_name.clone(),
                    exception: err_m,
                    code: 400,
                    is_async: true,
                    is_middleware: false,
                    source_step_id: String::new(),
                    source_name: String::new(),
                    message: String::new(),
                });
            }
        }
    }

    let mut processed_exc_ids: HashSet<String> = HashSet::new();
    for exc in &exc_configs {
        let exc_name = if exc.exception.is_empty() {
            "MiddlewareException".to_string()
        } else {
            exc.exception.clone()
        };
        let exc_code = if exc.code <= 0 { 400 } else { exc.code };
        let exc_act_name = &exc.activity;
        let is_global_exc =
            exc_name == "*" || exc_code == 500 || exc_act_name.to_lowercase().contains("global");
        let exc_node_id = if is_global_exc {
            format!("node__exc_{}", exc_act_name.to_lowercase())
        } else {
            format!("node__exc_{}", exc_name.to_lowercase())
        };

        let mut src_id = "node__ingress_guards".to_string();
        if !exc.source_step_id.is_empty() {
            src_id = format!("node__{}", exc.source_step_id);
        } else if is_global_exc {
            if let Some(st) = state {
                for t in &st.traces {
                    if t.status == "FAILED" && !t.step_id.starts_with("exc_") {
                        src_id = format!("node__{}", t.step_id);
                        break;
                    }
                }
            }
            if src_id == "node__ingress_guards" && !steps_to_visualize.is_empty() {
                src_id = format!("node__{}", steps_to_visualize[0].id);
            }
        } else if !has_ingress_guards {
            src_id = "node__start".to_string();
        }

        let src_exists = nodes.iter().any(|n| {
            n.as_object()
                .and_then(|o| o.get("id"))
                .and_then(|v| v.as_str())
                == Some(&src_id)
        });
        if !src_exists {
            src_id = if has_ingress_guards {
                "node__ingress_guards".to_string()
            } else {
                "node__start".to_string()
            };
        }

        if !processed_exc_ids.insert(exc_node_id.clone()) {
            continue;
        }

        let mut is_exc_executed = false;
        if let Some(st) = state {
            let s_err = st.error.as_deref().unwrap_or("").to_lowercase();
            let exc_src_step = exc.source_step_id.to_lowercase();

            if is_global_exc {
                if st.status == ExecutionStatus::Failed {
                    for t in &st.traces {
                        if t.activity_name.eq_ignore_ascii_case(exc_act_name)
                            || t.step_id
                                .eq_ignore_ascii_case(&format!("exc_{}", exc_act_name))
                        {
                            is_exc_executed = true;
                            break;
                        }
                    }
                }
            } else {
                for t in &st.traces {
                    let t_err = t.error_message.to_lowercase();
                    let t_step = t.step_id.to_lowercase();
                    let inp_exc = t
                        .input_payload
                        .get("exception")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let inp_err = t
                        .input_payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let inp_step = t
                        .input_payload
                        .get("step_id")
                        .or_else(|| t.input_payload.get("failing_step_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let inp_code = t
                        .input_payload
                        .get("status_code")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as i32;

                    if !exc_src_step.is_empty() {
                        if (t.activity_name.eq_ignore_ascii_case(exc_act_name)
                            || t_step
                                == format!("exc_{}_{}", exc_src_step, exc_name.to_lowercase())
                            || t_step == format!("exc_{}", exc_act_name.to_lowercase()))
                            && (inp_step == exc_src_step
                                || inp_exc == exc_name.to_lowercase()
                                || inp_err == exc_name.to_lowercase()
                                || inp_exc.contains(&exc_name.to_lowercase())
                                || (inp_code > 0 && inp_code == exc_code))
                        {
                            is_exc_executed = true;
                            break;
                        }
                    } else if t.activity_name.eq_ignore_ascii_case(exc_act_name)
                        || t_step == format!("exc_{}", exc_name.to_lowercase())
                        || t_step == format!("exc_{}", exc_act_name.to_lowercase())
                    {
                        if !inp_step.is_empty()
                            && inp_step != "ingress_guards"
                            && inp_step != "egress_guards"
                            && inp_step != "middleware"
                        {
                            continue;
                        }
                        if inp_exc == exc_name.to_lowercase()
                            || inp_err == exc_name.to_lowercase()
                            || t_err.contains(&exc_name.to_lowercase())
                            || t_step == format!("exc_{}", exc_name.to_lowercase())
                        {
                            is_exc_executed = true;
                            break;
                        }
                    }
                }

                if !is_exc_executed {
                    if let Some(d_exc) = st.data.get("exception").and_then(|v| v.as_str()) {
                        if d_exc.eq_ignore_ascii_case(&exc_name) {
                            is_exc_executed = true;
                        }
                    } else if let Some(d_err) = st.data.get("error").and_then(|v| v.as_str()) {
                        if d_err.eq_ignore_ascii_case(&exc_name) {
                            is_exc_executed = true;
                        }
                    }
                    if !is_exc_executed && s_err.contains(&exc_name.to_lowercase()) {
                        is_exc_executed = true;
                    }
                }
            }
        }

        let exc_status = if is_exc_executed {
            "COMPLETED"
        } else if has_live_execution {
            "SKIPPED"
        } else {
            "PENDING"
        };

        let mut exc_edge_lbl = format!("On {} ({})", exc_name, exc_code);
        let mut term_lbl = format!("🛑 FAILED ({})", exc_code);
        let mut term_id = format!("node__failed_term_{}", exc_code);

        let exc_lower = exc_name.to_lowercase();
        if is_global_exc {
            exc_edge_lbl = "💥 500 Unhandled Exception\n(Catch-All / Runtime Crash)".to_string();
            term_lbl = "🛑 500 INTERNAL SERVER ERROR".to_string();
            term_id = "node__failed_term_500".to_string();
        } else if exc_lower.contains("rate") {
            exc_edge_lbl = "⚠️ 429 RateLimitExceeded\n(Quota > 60 req/min)".to_string();
            term_lbl = "🛑 429 TOO MANY REQUESTS".to_string();
            term_id = "node__failed_term_429".to_string();
        } else if exc_lower.contains("ip") || exc_lower.contains("denied") {
            exc_edge_lbl = "🚫 403 IpAccessDenied\n(Blocked IP / Untrusted Subnet)".to_string();
            term_lbl = "🛑 403 FORBIDDEN".to_string();
            term_id = "node__failed_term_403".to_string();
        } else if exc_lower.contains("authorized") {
            exc_edge_lbl = "🚫 403 UserNotAuthorized\n(Forbidden Role/Permissions)".to_string();
            term_lbl = "🛑 403 FORBIDDEN".to_string();
            term_id = "node__failed_term_403".to_string();
        } else if exc_lower.contains("invalidcredentials") || exc_lower.contains("credentials") {
            exc_edge_lbl = "🔒 401 InvalidCredentials\n(Invalid Username/Password)".to_string();
            term_lbl = "🛑 401 UNAUTHORIZED".to_string();
            term_id = "node__failed_term_401".to_string();
        } else if exc_lower.contains("authenticated") {
            exc_edge_lbl = "🔒 401 UserNotAuthenticated\n(Missing/Invalid Token)".to_string();
            term_lbl = "🛑 401 UNAUTHORIZED".to_string();
            term_id = "node__failed_term_401".to_string();
        } else if exc_code == 400
            || exc_lower.contains("badrequest")
            || exc_lower.contains("validation")
            || exc_lower.contains("malformed")
            || exc_lower.contains("field")
        {
            exc_edge_lbl = format!("🛑 400 {}\n(Bad Request / Schema Violation)", exc_name);
            term_lbl = "🛑 400 BAD REQUEST".to_string();
            term_id = "node__failed_term_400".to_string();
        } else if exc_code == 413 || exc_lower.contains("toolarge") || exc_lower.contains("payload")
        {
            exc_edge_lbl = "🛑 413 PayloadTooLarge\n(Payload Size Exceeded)".to_string();
            term_lbl = "🛑 413 PAYLOAD TOO LARGE".to_string();
            term_id = "node__failed_term_413".to_string();
        }

        let mut origin_name = "Workflow Exception Handler".to_string();
        let mut desc = format!("Invoked upon exception {} ({}).", exc_name, exc_code);
        if exc.is_middleware {
            origin_name = format!("Middleware Pipeline ({})", exc.source_name);
            desc = format!(
                "Invoked by {} upon {} ({}).",
                exc.source_name, exc_name, exc_code
            );
        } else if !exc.source_step_id.is_empty() {
            origin_name = format!("Workflow Step ({})", exc.source_step_id);
            desc = format!(
                "Invoked when step '{}' encounters exception {} ({}).",
                exc.source_step_id, exc_name, exc_code
            );
        }
        if !exc.message.is_empty() {
            desc.push_str(" Policy: ");
            desc.push_str(&exc.message);
        }

        let (fields, mask_val) = bp.get_mask_config(None);
        let exc_key = format!("exc_{}", exc_act_name.to_lowercase());
        let inp_masked = input_map
            .get(&exc_key)
            .or_else(|| input_map.get(exc_act_name))
            .map(|m| Value::Object(mask_payload(m, &fields, &mask_val)))
            .unwrap_or(Value::Null);
        let out_masked = output_map
            .get(&exc_key)
            .or_else(|| output_map.get(exc_act_name))
            .map(|m| Value::Object(mask_payload(m, &fields, &mask_val)))
            .unwrap_or(Value::Null);

        let mut exc_node = vmap! {
            "id" => exc_node_id.clone(),
            "shape" => "octopus-activity-node",
            "label" => format!("🚨 {}", exc_act_name),
            "width" => 220i64,
            "height" => 55i64,
            "data" => Value::Object(vmap! {
                "step_id" => exc_key,
                "activity" => exc_act_name.clone(),
                "type" => "exception_handler",
                "is_async" => exc.is_async,
                "exception_type" => exc_name.clone(),
                "status_code" => exc_code as i64,
                "source_type" => if exc.is_middleware {
                    "Middleware Exception Pipeline"
                } else {
                    "Workflow Step Exception Handler"
                },
                "source_location" => origin_name,
                "source_name" => exc.source_name.clone(),
                "source_step_id" => exc.source_step_id.clone(),
                "is_middleware" => exc.is_middleware,
                "title" => format!("🚨 Exception Handler: {}", exc_act_name),
                "description" => desc,
                "status" => exc_status,
                "executed" => is_exc_executed,
                "input_payload" => inp_masked,
                "output_payload" => out_masked,
            }),
        };

        if is_exc_executed {
            exc_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#10b981",
                        "strokeWidth" => 3i64,
                        "fill" => "#0f2e22",
                        "filter" => "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity" => 1.0f64,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => "#6ee7b7",
                        "fontWeight" => 600i64,
                    }),
                }),
            );
        } else if has_live_execution {
            exc_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#334155",
                        "strokeWidth" => 1.5f64,
                        "fill" => "#111827",
                        "opacity" => 0.35f64,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => "#64748b",
                        "opacity" => 0.45f64,
                    }),
                }),
            );
        }
        nodes.push(Value::Object(exc_node));

        let (stroke_color, stroke_width, opacity, marker_color) = if is_exc_executed {
            ("#ef4444", 2.5f64, 1.0f64, "#ef4444")
        } else if has_live_execution {
            ("#1e293b", 1.5f64, 0.25f64, "#334155")
        } else {
            ("#f59e0b", 1.5f64, 0.8f64, "#f59e0b")
        };

        let (text_fill, rect_fill, rect_stroke, label_opacity) = if is_exc_executed {
            ("#f87171", "#2d1215", "#ef4444", 1.0f64)
        } else if has_live_execution {
            ("#475569", "#090d16", "#1e293b", 0.3f64)
        } else {
            ("#fbbf24", "#0f172a", "#f59e0b", 1.0f64)
        };

        edges.push(Value::Object(vmap! {
            "id" => format!("edge__{}__to__{}", src_id, exc_node_id),
            "source" => src_id,
            "target" => exc_node_id.clone(),
            "attrs" => Value::Object(vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => stroke_color,
                    "strokeWidth" => stroke_width,
                    "strokeDasharray" => "5 5",
                    "opacity" => opacity,
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 8i64,
                        "height" => 6i64,
                        "fill" => marker_color,
                    }),
                }),
            }),
            "labels" => Value::Array(vec![Value::Object(vmap! {
                "position" => 0.4f64,
                "attrs" => Value::Object(vmap! {
                    "text" => Value::Object(vmap! {
                        "text" => exc_edge_lbl,
                        "fill" => text_fill,
                        "fontSize" => 10i64,
                        "fontWeight" => if is_exc_executed { "bold" } else { "normal" },
                        "opacity" => label_opacity,
                    }),
                    "rect" => Value::Object(vmap! {
                        "fill" => rect_fill,
                        "rx" => 4i64,
                        "ry" => 4i64,
                        "stroke" => rect_stroke,
                        "strokeWidth" => 1i64,
                        "opacity" => label_opacity,
                    }),
                }),
            })]),
        }));

        // Terminal Failure Node
        let existing_pos = nodes.iter().position(|n| {
            n.as_object()
                .and_then(|o| o.get("id"))
                .and_then(|v| v.as_str())
                == Some(&term_id)
        });

        if let Some(pos) = existing_pos {
            if is_exc_executed {
                if let Value::Object(ref mut existing_node) = nodes[pos] {
                    if let Some(Value::Object(ref mut t_data)) = existing_node.get_mut("data") {
                        t_data.insert("status".to_string(), Value::String("FAILED".to_string()));
                        t_data.insert("executed".to_string(), Value::Bool(true));
                    }
                    existing_node.insert(
                        "attrs".to_string(),
                        Value::Object(vmap! {
                            "body" => Value::Object(vmap! {
                                "rx" => 25i64,
                                "ry" => 25i64,
                                "fill" => "#450a0a",
                                "stroke" => "#ef4444",
                                "strokeWidth" => 3i64,
                                "filter" => "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                                "opacity" => 1.0f64,
                            }),
                            "label" => Value::Object(vmap! {
                                "fill" => "#fca5a5",
                                "fontSize" => 13i64,
                                "fontWeight" => 700i64,
                            }),
                        }),
                    );
                }
            }
        } else {
            let failed_term_status = if is_exc_executed {
                "FAILED"
            } else if has_live_execution {
                "SKIPPED"
            } else {
                "PENDING"
            };
            let term_width = (term_lbl.len() * 10 + 40).max(160) as i64;
            let mut term_node = vmap! {
                "id" => term_id.clone(),
                "shape" => "octopus-end-node",
                "label" => term_lbl,
                "width" => term_width,
                "height" => 50i64,
                "data" => Value::Object(vmap! {
                    "step_id" => "failed",
                    "type" => "end_failed",
                    "title" => format!("Failed: {}", exc_name),
                    "status" => failed_term_status,
                    "executed" => is_exc_executed,
                }),
            };
            if is_exc_executed {
                term_node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "rx" => 25i64,
                            "ry" => 25i64,
                            "fill" => "#450a0a",
                            "stroke" => "#ef4444",
                            "strokeWidth" => 3i64,
                            "filter" => "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                            "opacity" => 1.0f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#fca5a5",
                            "fontSize" => 13i64,
                            "fontWeight" => 700i64,
                        }),
                    }),
                );
            } else if has_live_execution {
                term_node.insert(
                    "attrs".to_string(),
                    Value::Object(vmap! {
                        "body" => Value::Object(vmap! {
                            "rx" => 25i64,
                            "ry" => 25i64,
                            "fill" => "#0f172a",
                            "stroke" => "#334155",
                            "strokeWidth" => 1.5f64,
                            "opacity" => 0.35f64,
                        }),
                        "label" => Value::Object(vmap! {
                            "fill" => "#64748b",
                            "fontSize" => 13i64,
                            "fontWeight" => 600i64,
                            "opacity" => 0.45f64,
                        }),
                    }),
                );
            }
            nodes.push(Value::Object(term_node));
        }

        edges.push(Value::Object(vmap! {
            "id" => format!("edge__{}__to__{}", exc_node_id, term_id),
            "source" => exc_node_id,
            "target" => term_id,
            "attrs" => Value::Object(vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => stroke_color,
                    "strokeWidth" => stroke_width,
                    "strokeDasharray" => "5 5",
                    "opacity" => opacity,
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 8i64,
                        "height" => 6i64,
                        "fill" => marker_color,
                    }),
                }),
            }),
        }));
    }

    // 4. Egress Post-Execution Middleware Guards Node
    if has_egress_guards {
        let has_pred_exec = final_heads.iter().any(|p| p.executed);
        let egress_executed = state
            .as_ref()
            .map(|s| s.status == ExecutionStatus::Completed && has_pred_exec)
            .unwrap_or(false);
        let egress_status = if egress_executed {
            "COMPLETED"
        } else if has_live_execution {
            "SKIPPED"
        } else {
            "PENDING"
        };

        let post_mw_names_list: Vec<Value> = post_middlewares
            .iter()
            .filter(|m| !m.name.is_empty())
            .map(|m| Value::String(m.name.clone()))
            .collect();

        let mut egress_node = vmap! {
            "id" => "node__egress_guards",
            "shape" => "octopus-activity-node",
            "label" => "📊 Egress Middleware Guards",
            "width" => 220i64,
            "height" => 55i64,
            "data" => Value::Object(vmap! {
                "step_id" => "egress_guards",
                "type" => "egress_guards",
                "title" => "📊 Egress Middleware Guards",
                "description" => "Post-execution response auditing, latency logging, and metrics dispatch executed following workflow pipeline completion.",
                "status" => egress_status,
                "executed" => egress_executed,
                "middlewares" => Value::Array(post_mw_names_list),
            }),
        };

        if egress_executed {
            egress_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#10b981",
                        "strokeWidth" => 3i64,
                        "fill" => "#0f2e22",
                        "filter" => "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity" => 1.0f64,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => "#6ee7b7",
                        "fontWeight" => 600i64,
                    }),
                }),
            );
        } else {
            let (opacity_val, label_fill) = if has_live_execution {
                (0.35f64, "#475569")
            } else {
                (0.6f64, "#64748b")
            };
            egress_node.insert(
                "attrs".to_string(),
                Value::Object(vmap! {
                    "body" => Value::Object(vmap! {
                        "stroke" => "#334155",
                        "strokeWidth" => 1.5f64,
                        "fill" => "#111827",
                        "opacity" => opacity_val,
                    }),
                    "label" => Value::Object(vmap! {
                        "fill" => label_fill,
                        "fontWeight" => 600i64,
                    }),
                }),
            );
        }
        nodes.push(Value::Object(egress_node));

        for p in &final_heads {
            let edge_attrs = if p.executed && egress_executed {
                vmap! {
                    "line" => Value::Object(vmap! {
                        "stroke" => "#10b981",
                        "strokeWidth" => 3.5f64,
                        "targetMarker" => Value::Object(vmap! {
                            "name" => "block",
                            "width" => 10i64,
                            "height" => 8i64,
                            "fill" => "#10b981",
                        }),
                        "filter" => "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                        "opacity" => 1.0f64,
                    }),
                }
            } else if has_live_execution {
                vmap! {
                    "line" => Value::Object(vmap! {
                        "stroke" => "#334155",
                        "strokeWidth" => 1.5f64,
                        "targetMarker" => Value::Object(vmap! {
                            "name" => "block",
                            "width" => 6i64,
                            "height" => 5i64,
                            "fill" => "#334155",
                        }),
                        "opacity" => 0.35f64,
                    }),
                }
            } else {
                vmap! {
                    "line" => Value::Object(vmap! {
                        "stroke" => "#6366f1",
                        "strokeWidth" => 2i64,
                        "targetMarker" => Value::Object(vmap! {
                            "name" => "block",
                            "width" => 8i64,
                            "height" => 6i64,
                            "fill" => "#6366f1",
                        }),
                    }),
                }
            };
            edges.push(Value::Object(vmap! {
                "id" => format!("edge__{}__to__node__egress_guards", p.id),
                "source" => p.id.clone(),
                "target" => "node__egress_guards",
                "attrs" => Value::Object(edge_attrs),
            }));
        }

        final_heads = vec![PrevItem {
            id: "node__egress_guards".to_string(),
            label: String::new(),
            color: "#10b981".to_string(),
            executed: egress_executed,
        }];
    }

    // 5. End Node
    let has_exec_pred = final_heads.iter().any(|p| p.executed);
    let end_executed = state
        .as_ref()
        .map(|s| s.status == ExecutionStatus::Completed && has_exec_pred)
        .unwrap_or(false);
    let end_status = if end_executed {
        "COMPLETED"
    } else if has_live_execution {
        "SKIPPED"
    } else {
        "PENDING"
    };

    let success_status_code: Option<u16> =
        if let Some(ep) = selected_ep.filter(|ep| ep.status_code > 0) {
            Some(ep.status_code)
        } else if !bp.api.is_empty() {
            Some(200)
        } else {
            None
        };

    let (end_label, end_title) = if let Some(code) = success_status_code {
        let lbl = match code {
            200 => "✅ 200 OK".to_string(),
            201 => "✅ 201 CREATED".to_string(),
            202 => "✅ 202 ACCEPTED".to_string(),
            204 => "✅ 204 NO CONTENT".to_string(),
            _ => format!("✅ {} OK", code),
        };
        (lbl, format!("Completed: {}", code))
    } else {
        ("✅ COMPLETED".to_string(), "Workflow Completed".to_string())
    };

    let end_width = (end_label.len() * 10 + 36).max(140) as i64;
    let mut end_data = vmap! {
        "step_id" => "end",
        "type" => "end",
        "title" => end_title,
        "status" => end_status,
        "executed" => end_executed,
    };
    if let Some(code) = success_status_code {
        end_data.insert("status_code".to_string(), Value::Int(code as i64));
    }

    let mut end_node = vmap! {
        "id" => "node__end",
        "shape" => "octopus-end-node",
        "width" => end_width,
        "height" => 50i64,
        "label" => end_label,
        "data" => Value::Object(end_data),
    };

    if end_executed {
        end_node.insert(
            "attrs".to_string(),
            Value::Object(vmap! {
                "body" => Value::Object(vmap! {
                    "rx" => 25i64,
                    "ry" => 25i64,
                    "fill" => "#064e3b",
                    "stroke" => "#10b981",
                    "strokeWidth" => 3i64,
                    "filter" => "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    "opacity" => 1.0f64,
                }),
                "label" => Value::Object(vmap! {
                    "fill" => "#6ee7b7",
                    "fontSize" => 13i64,
                    "fontWeight" => 700i64,
                }),
            }),
        );
    } else {
        let (opacity_val, label_fill) = if has_live_execution {
            (0.35f64, "#475569")
        } else {
            (0.6f64, "#64748b")
        };
        end_node.insert(
            "attrs".to_string(),
            Value::Object(vmap! {
                "body" => Value::Object(vmap! {
                    "rx" => 25i64,
                    "ry" => 25i64,
                    "fill" => "#0f172a",
                    "stroke" => "#334155",
                    "strokeWidth" => 1.5f64,
                    "opacity" => opacity_val,
                }),
                "label" => Value::Object(vmap! {
                    "fill" => label_fill,
                    "fontSize" => 13i64,
                    "fontWeight" => 600i64,
                }),
            }),
        );
    }
    nodes.push(Value::Object(end_node));

    for p in &final_heads {
        let edge_attrs = if p.executed && end_status == "COMPLETED" {
            vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => "#10b981",
                    "strokeWidth" => 3.5f64,
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 10i64,
                        "height" => 8i64,
                        "fill" => "#10b981",
                    }),
                    "filter" => "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                    "opacity" => 1.0f64,
                }),
            }
        } else if has_live_execution {
            vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => "#1e293b",
                    "strokeWidth" => 1.5f64,
                    "strokeDasharray" => "4 4",
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 6i64,
                        "height" => 4i64,
                        "fill" => "#334155",
                    }),
                    "opacity" => 0.25f64,
                }),
            }
        } else {
            vmap! {
                "line" => Value::Object(vmap! {
                    "stroke" => "#64748b",
                    "strokeWidth" => 2i64,
                    "targetMarker" => Value::Object(vmap! {
                        "name" => "block",
                        "width" => 8i64,
                        "height" => 6i64,
                        "fill" => "#64748b",
                    }),
                    "opacity" => 1.0f64,
                }),
            }
        };

        edges.push(Value::Object(vmap! {
            "id" => format!("edge_{}_to_node__end", p.id),
            "source" => p.id.clone(),
            "target" => "node__end",
            "attrs" => Value::Object(edge_attrs),
        }));
    }

    let (status_str, overall_status_str) = if let Some(st) = state {
        (
            st.status.as_str().to_string(),
            st.status.as_str().to_string(),
        )
    } else {
        ("IDLE".to_string(), String::new())
    };

    let job_exists = job_id.map(|s| s.is_empty()).unwrap_or(true) || state.is_some();

    let api_list: Vec<Value> = bp
        .api
        .iter()
        .map(|ep| {
            let tags: Vec<Value> = ep.tags.iter().map(|t| Value::String(t.clone())).collect();
            Value::Object(vmap! {
                "route" => ep.route.clone(),
                "method" => ep.method.clone(),
                "input_model" => ep.input_model.clone(),
                "output_model" => ep.output_model.clone(),
                "status_code" => ep.status_code as i64,
                "summary" => ep.summary.clone(),
                "tags" => Value::Array(tags),
            })
        })
        .collect();

    let mw_list: Vec<Value> = bp.middlewares.iter().map(|m| m.to_value()).collect();

    let executed_count = nodes
        .iter()
        .filter(|n| {
            if let Some(d) = n.get("data").and_then(|v| v.as_object()) {
                let is_exec = d.get("executed").and_then(|v| v.as_bool()).unwrap_or(false);
                let node_type = d.get("type").and_then(|v| v.as_str()).unwrap_or("");
                is_exec && node_type != "start" && node_type != "end" && node_type != "end_failed"
            } else {
                false
            }
        })
        .count() as i64;

    let mut result = vmap! {
        "workflow_name" => bp.workflow_name.clone(),
        "version" => bp.version.clone(),
        "description" => bp.description.clone(),
        "namespace" => bp.namespace.clone(),
        "mode" => bp.mode.clone(),
        "job_id" => job_id.unwrap_or("").to_string(),
        "job_exists" => job_exists,
        "status" => status_str,
        "overall_status" => overall_status_str,
        "executed_count" => executed_count,
        "nodes" => Value::Array(nodes),
        "edges" => Value::Array(edges),
        "api" => Value::Array(api_list.clone()),
        "api_endpoints" => Value::Array(api_list),
        "routes" => Value::Array(routes_list),
        "middlewares" => Value::Array(mw_list),
        "persistence" => bp.persistence.as_ref().map(|p| Value::Object(p.clone())).unwrap_or(Value::Null),
    };

    if !selected_route.is_empty() {
        result.insert("selected_route".to_string(), Value::String(selected_route));
    }
    if !selected_method.is_empty() {
        result.insert(
            "selected_method".to_string(),
            Value::String(selected_method),
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::WorkflowStep;

    fn blueprint() -> WorkflowBlueprint {
        WorkflowBlueprint {
            workflow_name: "pure".to_string(),
            steps: vec![
                WorkflowStep {
                    id: "a".to_string(),
                    activity: "A".to_string(),
                    ..Default::default()
                },
                WorkflowStep {
                    id: "b".to_string(),
                    activity: "B".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn compile_diagram_needs_no_repository_or_store() {
        let graph = compile_diagram(&blueprint(), None, None, None, None).unwrap();
        assert_eq!(
            graph.get("workflow_name").and_then(|v| v.as_str()),
            Some("pure")
        );
        let nodes = graph.get("nodes").and_then(|v| v.as_array()).unwrap();
        assert!(nodes.len() >= 2, "nodes: {:?}", nodes);
        assert_eq!(
            graph.get("job_exists").and_then(|v| v.as_bool()),
            Some(true)
        );

        // A job that was asked for but has no state is reported as missing.
        let missing = compile_diagram(&blueprint(), Some("job-1"), None, None, None).unwrap();
        assert_eq!(
            missing.get("job_exists").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn a_step_node_says_whether_it_is_the_output_step() {
        let mut bp = blueprint();
        bp.steps[1].is_output = true;
        let graph = compile_diagram(&bp, None, None, None, None).unwrap();
        let is_output = |id: &str| {
            graph
                .get("nodes")
                .and_then(|v| v.as_array())
                .unwrap()
                .iter()
                .find(|n| n.get("id").and_then(|v| v.as_str()) == Some(id))
                .and_then(|n| n.get("data")?.get("is_output")?.as_bool())
        };
        assert_eq!(is_output("node__a"), Some(false));
        assert_eq!(is_output("node__b"), Some(true));
    }
}
