use crate::crypto::now_utc;
use crate::json::{Value, ValueMap};
use crate::yaml::parse_yaml;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ExecutionStatus {
    #[default]
    Idle,
    Running,
    Completed,
    Failed,
    RolledBack,
    Cancelled,
    AwaitingSignal,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionStatus::Idle => "IDLE",
            ExecutionStatus::Running => "RUNNING",
            ExecutionStatus::Completed => "COMPLETED",
            ExecutionStatus::Failed => "FAILED",
            ExecutionStatus::RolledBack => "ROLLED_BACK",
            ExecutionStatus::Cancelled => "CANCELLED",
            ExecutionStatus::AwaitingSignal => "AWAITING_SIGNAL",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "IDLE" => ExecutionStatus::Idle,
            "RUNNING" => ExecutionStatus::Running,
            "COMPLETED" => ExecutionStatus::Completed,
            "FAILED" => ExecutionStatus::Failed,
            "ROLLED_BACK" => ExecutionStatus::RolledBack,
            "CANCELLED" => ExecutionStatus::Cancelled,
            "AWAITING_SIGNAL" => ExecutionStatus::AwaitingSignal,
            _ => ExecutionStatus::Idle,
        }
    }
}

impl fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepType {
    Activity,
    Branch,
    Loop,
    Compensation,
}

impl StepType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepType::Activity => "activity",
            StepType::Branch => "branching_step",
            StepType::Loop => "loop",
            StepType::Compensation => "compensation",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Compensated,
    AwaitingSignal,
    AwaitingReview,
    SlaBreach,
    Bypassed,
    Rewound,
}

impl StepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Pending => "PENDING",
            StepStatus::Running => "RUNNING",
            StepStatus::Completed => "COMPLETED",
            StepStatus::Failed => "FAILED",
            StepStatus::Compensated => "COMPENSATED",
            StepStatus::AwaitingSignal => "AWAITING_SIGNAL",
            StepStatus::AwaitingReview => "AWAITING_APPROVAL",
            StepStatus::SlaBreach => "SLA_BREACH",
            StepStatus::Bypassed => "BYPASSED",
            StepStatus::Rewound => "REWOUND",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConditionClause {
    pub variable: String,
    pub field: String,
    pub operator: String,
    pub value: Value,
}

impl ConditionClause {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        if !self.variable.is_empty() {
            m.insert("variable".to_string(), Value::String(self.variable.clone()));
        }
        if !self.field.is_empty() {
            m.insert("field".to_string(), Value::String(self.field.clone()));
        }
        if !self.operator.is_empty() {
            m.insert("operator".to_string(), Value::String(self.operator.clone()));
        }
        if !self.value.is_null() {
            m.insert("value".to_string(), self.value.clone());
        }
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        Some(Self {
            variable: get_str(map, "variable"),
            field: get_str(map, "field"),
            operator: get_str(map, "operator"),
            value: map.get("value").cloned().unwrap_or(Value::Null),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConditionsGroup {
    pub r#match: String,
    pub conditions: Vec<ConditionClause>,
}

impl ConditionsGroup {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        if !self.r#match.is_empty() {
            m.insert("match".to_string(), Value::String(self.r#match.clone()));
        }
        let arr: Vec<Value> = self.conditions.iter().map(|c| c.to_value()).collect();
        m.insert("conditions".to_string(), Value::Array(arr));
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        let mut conditions = Vec::new();
        if let Some(arr) = map.get("conditions").and_then(|x| x.as_array()) {
            for item in arr {
                if let Some(c) = ConditionClause::from_value(item) {
                    conditions.push(c);
                }
            }
        }
        Some(Self {
            r#match: get_str(map, "match"),
            conditions,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct StepCondition {
    pub rule: String,
    pub operator: String,
    pub field: String,
    pub variable: String,
    pub value: Value,
    pub then_steps: Vec<WorkflowStep>,
    pub else_steps: Vec<WorkflowStep>,
}

impl StepCondition {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        if !self.rule.is_empty() {
            m.insert("rule".to_string(), Value::String(self.rule.clone()));
        }
        if !self.operator.is_empty() {
            m.insert("operator".to_string(), Value::String(self.operator.clone()));
        }
        if !self.field.is_empty() {
            m.insert("field".to_string(), Value::String(self.field.clone()));
        }
        if !self.variable.is_empty() {
            m.insert("variable".to_string(), Value::String(self.variable.clone()));
        }
        if !self.value.is_null() {
            m.insert("value".to_string(), self.value.clone());
        }
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        let then_steps = parse_steps_array(map.get("then_steps"));
        let else_steps = parse_steps_array(map.get("else_steps"));
        Some(Self {
            rule: get_str(map, "rule"),
            operator: get_str(map, "operator"),
            field: get_str(map, "field"),
            variable: get_str(map, "variable"),
            value: map.get("value").cloned().unwrap_or(Value::Null),
            then_steps,
            else_steps,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MiddlewareExceptionConfig {
    pub exception: String,
    pub code: i32,
    pub message: String,
    pub activity: String,
    pub is_async: bool,
    pub r#async: bool,
    pub headers: BTreeMap<String, String>,
}

impl MiddlewareExceptionConfig {
    pub fn get_is_async(&self) -> bool {
        self.is_async || self.r#async
    }

    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        m.insert(
            "exception".to_string(),
            Value::String(self.exception.clone()),
        );
        if self.code != 0 {
            m.insert("code".to_string(), Value::Int(self.code as i64));
        }
        if !self.message.is_empty() {
            m.insert("message".to_string(), Value::String(self.message.clone()));
        }
        if !self.activity.is_empty() {
            m.insert("activity".to_string(), Value::String(self.activity.clone()));
        }
        if self.get_is_async() {
            m.insert("is_async".to_string(), Value::Bool(true));
        }
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let mut exc = Self::default();
        exc.exception = get_str_or(m, "exception", &get_str(m, "name"));
        exc.code = get_i64(m, "code")
            .or_else(|| get_i64(m, "status_code"))
            .unwrap_or(0) as i32;
        exc.message = get_str_or(m, "message", &get_str(m, "detail"));
        exc.activity = get_str_or(m, "activity", &get_str(m, "handler_activity"));
        exc.is_async = get_bool(m, "async")
            .or_else(|| get_bool(m, "is_async"))
            .or_else(|| get_bool(m, "fire_and_forget"))
            .unwrap_or(false);
        exc.r#async = exc.is_async;
        if let Some(hdrs) = m.get("headers").and_then(|h| h.as_object()) {
            for (hk, hv) in hdrs {
                if let Some(s) = hv.as_str() {
                    exc.headers.insert(hk.clone(), s.to_string());
                }
            }
        }
        Some(exc)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MaskingConfig {
    pub enabled: bool,
    pub fields: Vec<String>,
    pub mask_value: String,
}

impl MaskingConfig {
    pub fn from_value(v: &Value) -> Option<Self> {
        if let Some(arr) = v.as_array() {
            let fields: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect();
            return Some(Self {
                enabled: true,
                fields,
                mask_value: "********".to_string(),
            });
        }
        let map = v.as_object()?;
        let enabled = get_bool(map, "enabled").unwrap_or(true);
        let mut fields = to_string_vec(map.get("fields"));
        if fields.is_empty() {
            fields = to_string_vec(map.get("mask_fields"));
        }
        let mut mask_value = get_str(map, "mask_value");
        if mask_value.is_empty() {
            mask_value = get_str(map, "mask");
        }
        if mask_value.is_empty() {
            mask_value = "********".to_string();
        }
        Some(Self {
            enabled,
            fields,
            mask_value,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct TelemetryConfig {
    pub mask_fields: Vec<String>,
    pub masking: Option<MaskingConfig>,
}

impl TelemetryConfig {
    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        let mask_fields = to_string_vec(map.get("mask_fields"));
        let masking = map.get("masking").and_then(MaskingConfig::from_value);
        Some(Self {
            mask_fields,
            masking,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowStep {
    pub id: String,
    pub activity: String,
    pub branching_step: String,
    pub human_step: String,
    pub await_signal: String,
    pub description: String,
    pub timeout_seconds: i32,
    pub delay_seconds: f64,
    pub compensation: String,
    pub pivot: bool,
    pub condition: Option<StepCondition>,
    pub conditions: Option<ConditionsGroup>,
    pub then_steps: Vec<WorkflowStep>,
    pub else_steps: Vec<WorkflowStep>,
    pub then: Vec<WorkflowStep>,
    pub r#else: Vec<WorkflowStep>,
    pub r#loop: Option<ValueMap>,
    pub for_each: Option<ValueMap>,
    pub retry_policy: Option<ValueMap>,
    pub jump_to: String,
    pub jump_to_step_index: Option<usize>,
    pub rewind_to_step_index: Option<usize>,
    pub r#async: bool,
    pub is_async: bool,
    pub compensate: bool,
    pub rollback: bool,
    pub trigger_compensation: bool,
    pub terminate: bool,
    pub settle: bool,
    pub reason: String,
    pub dynamic_branching: Option<ValueMap>,
    pub is_output: bool,
    pub output: bool,
    pub exceptions: Vec<MiddlewareExceptionConfig>,
    pub mask_fields: Vec<String>,
    pub input: Vec<String>,
    pub masking: Option<MaskingConfig>,
}

impl WorkflowStep {
    pub fn get_then_steps(&self) -> Vec<WorkflowStep> {
        if !self.then_steps.is_empty() {
            return self.then_steps.clone();
        }
        if !self.then.is_empty() {
            return self.then.clone();
        }
        if let Some(ref c) = self.condition {
            if !c.then_steps.is_empty() {
                return c.then_steps.clone();
            }
        }
        Vec::new()
    }

    pub fn get_else_steps(&self) -> Vec<WorkflowStep> {
        if !self.else_steps.is_empty() {
            return self.else_steps.clone();
        }
        if !self.r#else.is_empty() {
            return self.r#else.clone();
        }
        if let Some(ref c) = self.condition {
            if !c.else_steps.is_empty() {
                return c.else_steps.clone();
            }
        }
        Vec::new()
    }

    pub fn signal_name(&self) -> String {
        if !self.await_signal.is_empty() {
            self.await_signal.clone()
        } else {
            self.human_step.clone()
        }
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        let mut step = Self::default();
        step.id = get_str(map, "id");
        step.activity = get_str(map, "activity");
        step.branching_step = get_str(map, "branching_step");
        step.human_step = get_str(map, "human_step");
        step.await_signal = get_str(map, "await_signal");
        step.description = get_str(map, "description");
        step.timeout_seconds = get_i64(map, "timeout_seconds").unwrap_or(0) as i32;
        step.delay_seconds = get_f64(map, "delay_seconds").unwrap_or(0.0);
        step.compensation = get_str(map, "compensation");
        step.pivot = get_bool(map, "pivot").unwrap_or(false);
        step.condition = map.get("condition").and_then(StepCondition::from_value);
        step.conditions = map.get("conditions").and_then(ConditionsGroup::from_value);
        step.then_steps = parse_steps_array(map.get("then_steps"));
        step.else_steps = parse_steps_array(map.get("else_steps"));
        step.then = parse_steps_array(map.get("then"));
        step.r#else = parse_steps_array(map.get("else"));
        step.r#loop = map.get("loop").and_then(|x| x.as_object()).cloned();
        step.for_each = map.get("for_each").and_then(|x| x.as_object()).cloned();
        step.retry_policy = map.get("retry_policy").and_then(|x| x.as_object()).cloned();
        step.jump_to = get_str(map, "jump_to");
        step.jump_to_step_index = get_i64(map, "jump_to_step_index").and_then(|i| {
            if i >= 0 {
                Some(i as usize)
            } else {
                None
            }
        });
        step.rewind_to_step_index = get_i64(map, "rewind_to_step_index").and_then(|i| {
            if i >= 0 {
                Some(i as usize)
            } else {
                None
            }
        });
        step.r#async = get_bool(map, "async").unwrap_or(false);
        step.is_async = get_bool(map, "is_async").unwrap_or(false);
        step.compensate = get_bool(map, "compensate").unwrap_or(false);
        step.rollback = get_bool(map, "rollback").unwrap_or(false);
        step.trigger_compensation = get_bool(map, "trigger_compensation").unwrap_or(false);
        step.terminate = get_bool(map, "terminate").unwrap_or(false);
        step.settle = get_bool(map, "settle").unwrap_or(false);
        step.reason = get_str(map, "reason");
        step.dynamic_branching = map
            .get("dynamic_branching")
            .and_then(|x| x.as_object())
            .cloned();
        step.is_output = get_bool(map, "is_output").unwrap_or(false);
        step.output = get_bool(map, "output").unwrap_or(false);
        step.exceptions = parse_middleware_exceptions(map.get("exceptions"));
        step.mask_fields = to_string_vec(map.get("mask_fields"));
        step.input = to_string_vec(map.get("input"));
        step.masking = map.get("masking").and_then(MaskingConfig::from_value);
        Some(step)
    }
}

#[derive(Clone, Debug, Default)]
pub struct APIEndpointConfig {
    pub route: String,
    pub method: String,
    pub input_model: String,
    pub output_model: String,
    pub status_code: u16,
    pub summary: String,
    pub description: String,
    pub tags: Vec<String>,
    pub steps: Vec<WorkflowStep>,
}

impl APIEndpointConfig {
    pub fn from_value(v: &Value) -> Option<Self> {
        let map = v.as_object()?;
        Some(Self {
            route: get_str(map, "route"),
            method: get_str(map, "method"),
            input_model: get_str(map, "input_model"),
            output_model: get_str(map, "output_model"),
            status_code: get_i64(map, "status_code").unwrap_or(0) as u16,
            summary: get_str(map, "summary"),
            description: get_str(map, "description"),
            tags: to_string_vec(map.get("tags")),
            steps: parse_steps_array(map.get("steps")),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MiddlewareConfig {
    pub name: String,
    pub r#type: String,
    pub activity: String,
    pub is_async: bool,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub exceptions: Vec<MiddlewareExceptionConfig>,
    pub config: ValueMap,
}

impl MiddlewareConfig {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        m.insert("name".to_string(), Value::String(self.name.clone()));
        if !self.r#type.is_empty() {
            m.insert("type".to_string(), Value::String(self.r#type.clone()));
        }
        if !self.activity.is_empty() {
            m.insert("activity".to_string(), Value::String(self.activity.clone()));
        }
        if self.is_async {
            m.insert("is_async".to_string(), Value::Bool(true));
        }
        if !self.excludes.is_empty() {
            m.insert("excludes".to_string(), Value::from(self.excludes.clone()));
        }
        if !self.includes.is_empty() {
            m.insert("includes".to_string(), Value::from(self.includes.clone()));
        }
        if !self.exceptions.is_empty() {
            let excs: Vec<Value> = self.exceptions.iter().map(|e| e.to_value()).collect();
            m.insert("exceptions".to_string(), Value::Array(excs));
        }
        if !self.config.is_empty() {
            m.insert("config".to_string(), Value::Object(self.config.clone()));
        }
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        if let Some(str_name) = v.as_str() {
            return Some(Self {
                name: str_name.to_string(),
                r#type: "pre-request".to_string(),
                ..Default::default()
            });
        }
        let raw = v.as_object()?;
        let mut m = Self::default();
        m.r#type = "pre-request".to_string();
        if let Some(name) = raw.get("name").and_then(|x| x.as_str()) {
            m.name = name.to_string();
        }
        if let Some(t) = raw.get("type").and_then(|x| x.as_str()) {
            if t.eq_ignore_ascii_case("pre-request") || t.eq_ignore_ascii_case("post-request") {
                m.r#type = t.to_lowercase();
            } else if m.name.is_empty() {
                m.name = t.to_string();
            }
        }
        m.activity = get_str(raw, "activity");
        m.is_async = get_bool(raw, "async")
            .or_else(|| get_bool(raw, "is_async"))
            .unwrap_or(false);

        if let Some(exc) = raw.get("excludes").or_else(|| raw.get("exclude")) {
            m.excludes = to_string_vec(Some(exc));
        }
        if let Some(inc) = raw.get("includes").or_else(|| raw.get("include")) {
            m.includes = to_string_vec(Some(inc));
        }
        m.exceptions = parse_middleware_exceptions(raw.get("exceptions"));

        let mut cfg = ValueMap::new();
        if let Some(nested_cfg) = raw.get("config").and_then(|x| x.as_object()) {
            for (k, val) in nested_cfg {
                cfg.insert(k.clone(), val.clone());
            }
        }
        let reserved: HashSet<&str> = [
            "name",
            "type",
            "excludes",
            "exclude",
            "includes",
            "include",
            "exceptions",
            "config",
        ]
        .into_iter()
        .collect();
        for (k, val) in raw {
            if !reserved.contains(k.as_str()) {
                cfg.insert(k.clone(), val.clone());
            }
        }
        m.config = cfg;
        Some(m)
    }
}

#[derive(Clone, Debug, Default)]
pub struct GlobalExceptionHandlerConfig {
    pub activity: String,
    pub code: i32,
    pub message: String,
    pub is_async: bool,
    pub r#async: bool,
    pub include_stack_trace: bool,
    pub headers: BTreeMap<String, String>,
}

impl GlobalExceptionHandlerConfig {
    pub fn get_is_async(&self) -> bool {
        self.is_async || self.r#async
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let mut headers = BTreeMap::new();
        if let Some(hdrs) = m.get("headers").and_then(|h| h.as_object()) {
            for (k, val) in hdrs {
                if let Some(s) = val.as_str() {
                    headers.insert(k.clone(), s.to_string());
                }
            }
        }
        let is_async = get_bool(m, "is_async")
            .or_else(|| get_bool(m, "async"))
            .unwrap_or(false);
        Some(Self {
            activity: get_str(m, "activity"),
            code: get_i64(m, "code").unwrap_or(0) as i32,
            message: get_str(m, "message"),
            is_async,
            r#async: is_async,
            include_stack_trace: get_bool(m, "include_stack_trace").unwrap_or(false),
            headers,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ErrorHandlingConfig {
    pub global_handler: Option<GlobalExceptionHandlerConfig>,
}

impl ErrorHandlingConfig {
    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let global_handler = m
            .get("global_handler")
            .and_then(GlobalExceptionHandlerConfig::from_value);
        Some(Self { global_handler })
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorkflowBlueprint {
    pub workflow_name: String,
    pub version: String,
    pub namespace: String,
    pub description: String,
    pub mode: String,
    pub compensation_mode: String,
    pub allowed_initiator_roles: Vec<String>,
    pub middlewares: Vec<MiddlewareConfig>,
    pub steps: Vec<WorkflowStep>,
    pub api: Vec<APIEndpointConfig>,
    pub persistence: Option<ValueMap>,
    pub metadata: Option<ValueMap>,
    pub masking: Option<MaskingConfig>,
    pub telemetry: Option<TelemetryConfig>,
    pub redaction: Option<MaskingConfig>,
    pub error_handling: Option<ErrorHandlingConfig>,
    pub global_exception_handler: Option<GlobalExceptionHandlerConfig>,
    #[doc(hidden)]
    pub mask_fields_cache: MaskFieldsCache,
}

/// `all_mask_fields`, worked out once per blueprint. A clone starts empty,
/// so a copy that is then edited works its fields out again; a blueprint
/// must not change after its fields are first read.
#[derive(Default)]
pub struct MaskFieldsCache(std::sync::OnceLock<(Vec<String>, String)>);

impl Clone for MaskFieldsCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for MaskFieldsCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MaskFieldsCache")
    }
}

impl WorkflowBlueprint {
    pub fn from_yaml_str(content: &str) -> Result<Self, String> {
        let val = parse_yaml(content)?;
        Self::from_value(&val)
    }

    pub fn from_value(v: &Value) -> Result<Self, String> {
        let map = v
            .as_object()
            .ok_or_else(|| "Blueprint root must be a YAML/JSON mapping".to_string())?;
        let mut bp = Self::default();
        bp.workflow_name = get_str(map, "workflow_name");
        bp.version = get_str(map, "version");
        bp.namespace = get_str(map, "namespace");
        bp.description = get_str(map, "description");
        bp.mode = get_str(map, "mode");
        bp.compensation_mode = get_str(map, "compensation_mode");
        bp.allowed_initiator_roles = to_string_vec(map.get("allowed_initiator_roles"));

        if let Some(mws) = map.get("middlewares").and_then(|x| x.as_array()) {
            for item in mws {
                if let Some(mw) = MiddlewareConfig::from_value(item) {
                    bp.middlewares.push(mw);
                }
            }
        }
        bp.steps = parse_steps_array(map.get("steps"));
        bp.api = parse_api_endpoints(map.get("api"));
        bp.persistence = map.get("persistence").and_then(|x| x.as_object()).cloned();
        bp.metadata = map.get("metadata").and_then(|x| x.as_object()).cloned();
        bp.masking = map.get("masking").and_then(MaskingConfig::from_value);
        bp.telemetry = map.get("telemetry").and_then(TelemetryConfig::from_value);
        bp.redaction = map.get("redaction").and_then(MaskingConfig::from_value);
        bp.error_handling = map
            .get("error_handling")
            .and_then(ErrorHandlingConfig::from_value);
        bp.global_exception_handler = map
            .get("global_exception_handler")
            .and_then(GlobalExceptionHandlerConfig::from_value);
        Ok(bp)
    }

    /// Route prefix for the blueprint's built-in routes, e.g. "/api/v1.0.0/orders".
    /// Version "1.0.0" and "v1.0.0" give the same prefix.
    pub fn api_prefix(&self) -> String {
        let trimmed_ver = self.version.trim_matches(|c| c == '/' || c == ' ');
        let mut ver = trimmed_ver.strip_prefix('v').unwrap_or(trimmed_ver);
        if ver.is_empty() {
            ver = "1.0.0";
        }
        let wf = self.workflow_name.trim_matches(|c| c == '/' || c == ' ');
        format!("/api/v{}/{}", ver, wf)
    }

    pub fn normalize_endpoints(&mut self) {
        if self.api.is_empty() {
            return;
        }
        let prefix = self.api_prefix();
        for ep in &mut self.api {
            let r = ep.route.trim().to_string();
            if !r.is_empty() && !r.starts_with("/api/") && !r.starts_with(&prefix) {
                ep.route = format!("{}/{}", prefix, r.trim_start_matches('/'));
            }
        }
    }

    /// Returns every field declared sensitive anywhere in the blueprint
    /// (top level, telemetry, redaction, and any step or endpoint step).
    pub fn all_mask_fields(&self) -> (Vec<String>, String) {
        self.mask_fields_cache
            .0
            .get_or_init(|| self.compute_mask_fields())
            .clone()
    }

    fn compute_mask_fields(&self) -> (Vec<String>, String) {
        let (mut fields, mask_value) = self.get_mask_config(None);
        let mut seen: HashSet<String> = fields.iter().map(|f| f.to_lowercase()).collect();

        fn walk(
            bp: &WorkflowBlueprint,
            steps: &[WorkflowStep],
            fields: &mut Vec<String>,
            seen: &mut HashSet<String>,
        ) {
            for s in steps {
                let (step_fields, _) = bp.get_mask_config(Some(s));
                for f in step_fields {
                    let low = f.to_lowercase();
                    if !seen.contains(&low) {
                        seen.insert(low);
                        fields.push(f);
                    }
                }
                walk(bp, &s.get_then_steps(), fields, seen);
                walk(bp, &s.get_else_steps(), fields, seen);
            }
        }

        walk(self, &self.steps, &mut fields, &mut seen);
        for ep in &self.api {
            walk(self, &ep.steps, &mut fields, &mut seen);
        }

        (fields, mask_value)
    }

    pub fn get_mask_config(&self, step: Option<&WorkflowStep>) -> (Vec<String>, String) {
        let mut mask_value = "********".to_string();
        let mut fields = Vec::new();
        let mut seen = HashSet::new();

        let mut add_fields = |list: &[String]| {
            for f in list {
                let trimmed = f.trim();
                let low = trimmed.to_lowercase();
                if !trimmed.is_empty() && !seen.contains(&low) {
                    seen.insert(low);
                    fields.push(trimmed.to_string());
                }
            }
        };

        // 1. Blueprint Top-Level Masking
        if let Some(ref m) = self.masking {
            if m.enabled {
                if !m.mask_value.is_empty() {
                    mask_value = m.mask_value.clone();
                }
                add_fields(&m.fields);
            }
        }

        // 2. Blueprint Telemetry Masking / mask_fields
        if let Some(ref t) = self.telemetry {
            if let Some(ref tm) = t.masking {
                if tm.enabled {
                    if !tm.mask_value.is_empty() {
                        mask_value = tm.mask_value.clone();
                    }
                    add_fields(&tm.fields);
                }
            }
            if !t.mask_fields.is_empty() {
                add_fields(&t.mask_fields);
            }
        }

        // 3. Blueprint Redaction
        if let Some(ref r) = self.redaction {
            if r.enabled {
                if !r.mask_value.is_empty() {
                    mask_value = r.mask_value.clone();
                }
                add_fields(&r.fields);
            }
        }

        // 4. Step-level Masking (if provided)
        if let Some(st) = step {
            if let Some(ref sm) = st.masking {
                if sm.enabled {
                    if !sm.mask_value.is_empty() {
                        mask_value = sm.mask_value.clone();
                    }
                    add_fields(&sm.fields);
                }
            }
            if !st.mask_fields.is_empty() {
                add_fields(&st.mask_fields);
            }
        }

        (fields, mask_value)
    }

    /// Rejects blueprint features this engine does not implement, so they fail
    /// at load instead of being silently ignored.
    pub fn validate(&self) -> Result<(), String> {
        if !self.allowed_initiator_roles.is_empty() {
            return Err(format!(
                "blueprint {:?}: allowed_initiator_roles is not supported; use an auth middleware",
                self.workflow_name
            ));
        }

        fn check(wf_name: &str, steps: &[WorkflowStep]) -> Result<(), String> {
            for st in steps {
                if let Some(ref dyn_branch) = st.dynamic_branching {
                    if !dyn_branch.is_empty() {
                        return Err(format!(
                            "blueprint {:?} step {:?}: dynamic_branching is not supported; use condition with then_steps/else_steps",
                            wf_name, st.id
                        ));
                    }
                }
                if !st.signal_name().is_empty() && (st.r#async || st.is_async) {
                    return Err(format!(
                        "blueprint {:?} step {:?}: await_signal can't be async",
                        wf_name, st.id
                    ));
                }
                check(wf_name, &st.get_then_steps())?;
                check(wf_name, &st.get_else_steps())?;
            }
            Ok(())
        }

        check(&self.workflow_name, &self.steps)?;
        for ep in &self.api {
            check(&self.workflow_name, &ep.steps)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct StepExecutionTrace {
    pub step_id: String,
    pub activity_name: String,
    pub status: String,
    pub is_async: bool,
    pub input_payload: ValueMap,
    pub output_payload: ValueMap,
    pub error_message: String,
    pub attempts: usize,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: f64,
}

impl StepExecutionTrace {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        m.insert("step_id".to_string(), Value::String(self.step_id.clone()));
        if !self.activity_name.is_empty() {
            m.insert(
                "activity_name".to_string(),
                Value::String(self.activity_name.clone()),
            );
        }
        m.insert("status".to_string(), Value::String(self.status.clone()));
        if self.is_async {
            m.insert("is_async".to_string(), Value::Bool(true));
        }
        if !self.input_payload.is_empty() {
            m.insert(
                "input_payload".to_string(),
                Value::Object(self.input_payload.clone()),
            );
        }
        if !self.output_payload.is_empty() {
            m.insert(
                "output_payload".to_string(),
                Value::Object(self.output_payload.clone()),
            );
        }
        if !self.error_message.is_empty() {
            m.insert(
                "error_message".to_string(),
                Value::String(self.error_message.clone()),
            );
        }
        if self.attempts > 0 {
            m.insert("attempts".to_string(), Value::Int(self.attempts as i64));
        }
        m.insert(
            "started_at".to_string(),
            Value::String(self.started_at.clone()),
        );
        m.insert(
            "finished_at".to_string(),
            Value::String(self.finished_at.clone()),
        );
        m.insert("duration_ms".to_string(), Value::Float(self.duration_ms));
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        Some(Self {
            step_id: get_str(m, "step_id"),
            activity_name: get_str(m, "activity_name"),
            status: get_str(m, "status"),
            is_async: get_bool(m, "is_async").unwrap_or(false),
            input_payload: m
                .get("input_payload")
                .and_then(|x| x.as_object())
                .cloned()
                .unwrap_or_default(),
            output_payload: m
                .get("output_payload")
                .and_then(|x| x.as_object())
                .cloned()
                .unwrap_or_default(),
            error_message: get_str(m, "error_message"),
            attempts: get_i64(m, "attempts").unwrap_or(0) as usize,
            started_at: get_str(m, "started_at"),
            finished_at: get_str(m, "finished_at"),
            duration_ms: get_f64(m, "duration_ms").unwrap_or(0.0),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    pub job_id: String,
    pub workflow_name: String,
    pub status: ExecutionStatus,
    pub route: String,
    pub method: String,
    pub current_step_id: String,
    pub current_index: usize,
    pub total_steps: usize,
    pub data: ValueMap,
    pub output_data: Option<ValueMap>,
    pub traces: Vec<StepExecutionTrace>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub client_host: String,
    pub duration_ms: f64,
    pub error: Option<String>,
    pub error_code: Option<i32>,
    pub error_type: Option<String>,
    /// The unmasked output a declared `api:` route responds with. Everything
    /// else about a job is masked (decision D5); this is never serialized,
    /// stored or published.
    pub route_output: Option<ValueMap>,
}

impl ExecutionState {
    pub fn to_value(&self) -> Value {
        let mut m = ValueMap::new();
        m.insert("job_id".to_string(), Value::String(self.job_id.clone()));
        m.insert(
            "workflow_name".to_string(),
            Value::String(self.workflow_name.clone()),
        );
        m.insert(
            "status".to_string(),
            Value::String(self.status.as_str().to_string()),
        );
        if !self.route.is_empty() {
            m.insert("route".to_string(), Value::String(self.route.clone()));
        }
        if !self.method.is_empty() {
            m.insert("method".to_string(), Value::String(self.method.clone()));
        }
        if !self.current_step_id.is_empty() {
            m.insert(
                "current_step_id".to_string(),
                Value::String(self.current_step_id.clone()),
            );
        }
        m.insert(
            "current_index".to_string(),
            Value::Int(self.current_index as i64),
        );
        m.insert(
            "total_steps".to_string(),
            Value::Int(self.total_steps as i64),
        );
        m.insert("data".to_string(), Value::Object(self.data.clone()));
        if let Some(ref out) = self.output_data {
            m.insert("output_data".to_string(), Value::Object(out.clone()));
        }
        let traces_arr: Vec<Value> = self.traces.iter().map(|t| t.to_value()).collect();
        m.insert("traces".to_string(), Value::Array(traces_arr));
        m.insert(
            "started_at".to_string(),
            Value::String(self.started_at.clone()),
        );
        if let Some(ref fin) = self.finished_at {
            m.insert("finished_at".to_string(), Value::String(fin.clone()));
        }
        if !self.client_host.is_empty() {
            m.insert(
                "client_host".to_string(),
                Value::String(self.client_host.clone()),
            );
        }
        if self.duration_ms > 0.0 {
            m.insert("duration_ms".to_string(), Value::Float(self.duration_ms));
        }
        if let Some(ref err) = self.error {
            m.insert("error".to_string(), Value::String(err.clone()));
        }
        if let Some(code) = self.error_code {
            m.insert("error_code".to_string(), Value::Int(code as i64));
        }
        if let Some(ref et) = self.error_type {
            m.insert("error_type".to_string(), Value::String(et.clone()));
        }
        Value::Object(m)
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let m = v.as_object()?;
        let mut traces = Vec::new();
        if let Some(arr) = m.get("traces").and_then(|x| x.as_array()) {
            for item in arr {
                if let Some(t) = StepExecutionTrace::from_value(item) {
                    traces.push(t);
                }
            }
        }
        Some(Self {
            job_id: get_str(m, "job_id"),
            workflow_name: get_str(m, "workflow_name"),
            status: ExecutionStatus::from_str(&get_str(m, "status")),
            route: get_str(m, "route"),
            method: get_str(m, "method"),
            current_step_id: get_str(m, "current_step_id"),
            current_index: get_i64(m, "current_index").unwrap_or(0) as usize,
            total_steps: get_i64(m, "total_steps").unwrap_or(0) as usize,
            data: m
                .get("data")
                .and_then(|x| x.as_object())
                .cloned()
                .unwrap_or_default(),
            output_data: m.get("output_data").and_then(|x| x.as_object()).cloned(),
            route_output: None,
            traces,
            started_at: get_str(m, "started_at"),
            finished_at: m
                .get("finished_at")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            client_host: get_str(m, "client_host"),
            duration_ms: get_f64(m, "duration_ms").unwrap_or(0.0),
            error: m
                .get("error")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            error_code: get_i64(m, "error_code").map(|c| c as i32),
            error_type: m
                .get("error_type")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct LiveEvent {
    pub event: String,
    pub data: Value,
}

#[derive(Clone, Debug)]
pub struct CompensationRecord {
    pub step_id: String,
    pub compensation: String,
    pub payload: ValueMap,
}

#[derive(Clone, Debug)]
pub struct Suspension {
    pub job_id: String,
    pub workflow_name: String,
    pub step_id: String,
    pub signal: String,
    pub route: String,
    pub method: String,
    pub idem_key: String,
    pub started_at: String,
    pub deadline: Option<Instant>,
    pub steps: Vec<WorkflowStep>,
    pub data: ValueMap,
    pub output_data: Option<ValueMap>,
    pub traces: Vec<StepExecutionTrace>,
    pub compensations: Vec<CompensationRecord>,
}

pub fn now_utc_str() -> String {
    now_utc()
}

fn get_str(map: &ValueMap, key: &str) -> String {
    map.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn get_str_or(map: &ValueMap, key: &str, fallback: &str) -> String {
    let s = get_str(map, key);
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn get_bool(map: &ValueMap, key: &str) -> Option<bool> {
    map.get(key).and_then(|v| v.as_bool())
}

fn get_i64(map: &ValueMap, key: &str) -> Option<i64> {
    map.get(key).and_then(|v| v.as_i64())
}

fn get_f64(map: &ValueMap, key: &str) -> Option<f64> {
    map.get(key).and_then(|v| v.as_f64())
}

fn to_string_vec(v: Option<&Value>) -> Vec<String> {
    let Some(val) = v else { return Vec::new() };
    if let Some(s) = val.as_str() {
        return vec![s.to_string()];
    }
    if let Some(arr) = val.as_array() {
        return arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
    }
    Vec::new()
}

fn parse_steps_array(v: Option<&Value>) -> Vec<WorkflowStep> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().filter_map(WorkflowStep::from_value).collect()
}

fn parse_middleware_exceptions(v: Option<&Value>) -> Vec<MiddlewareExceptionConfig> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(MiddlewareExceptionConfig::from_value)
        .collect()
}

fn parse_api_endpoints(v: Option<&Value>) -> Vec<APIEndpointConfig> {
    let Some(val) = v else { return Vec::new() };
    if let Some(arr) = val.as_array() {
        return arr
            .iter()
            .filter_map(APIEndpointConfig::from_value)
            .collect();
    }
    if let Some(obj) = val.as_object() {
        if let Some(eps) = obj.get("endpoints").and_then(|x| x.as_array()) {
            if !eps.is_empty() {
                return eps
                    .iter()
                    .filter_map(APIEndpointConfig::from_value)
                    .collect();
            }
        }
        if let Some(routes) = obj.get("routes").and_then(|x| x.as_array()) {
            if !routes.is_empty() {
                return routes
                    .iter()
                    .filter_map(APIEndpointConfig::from_value)
                    .collect();
            }
        }
        if let Some(single) = APIEndpointConfig::from_value(val) {
            if !single.route.is_empty() {
                return vec![single];
            }
        }
    }
    Vec::new()
}
