//! The engine, wired: blueprints, activities, state store, async dispatcher,
//! pipeline, visualizer and middleware, behind one value a host holds. The
//! Rust host and the FFI both build on it; neither wires the parts itself.

use std::fs;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

#[cfg(feature = "temporal")]
use crate::adapters::outbound::TemporalAsyncActivityDispatcher;
use crate::adapters::outbound::{
    ActivityResolver, CallableActivityExecutor, InMemoryAsyncActivityDispatcher,
    InMemoryStateStore, StoreFactory, YamlBlueprintRepository,
};
use crate::crypto::constant_time_eq;
use crate::domain::models::{
    APIEndpointConfig, EngineOptions, ErrorHandlingConfig, ExecutionState,
    GlobalExceptionHandlerConfig, LiveEvent, MiddlewareConfig, WorkflowBlueprint,
};
use crate::domain::ports::{
    ActivityFunc, IActivityExecutor, IAsyncActivityDispatcher, IBlueprintRepository, IStateStore,
    ISuspensionStore, IVisualizerCompiler, IWorkflowEngine, IntoActivityFunc,
};
use crate::domain::services::{
    build_middleware, decide_tooling_access, declared_routes, run_declared_route, DeclaredRoute,
    Middleware, PipelineManager, RouteAnswer, RouteRequest, ToolingDecision, VisualizerService,
};
use crate::json::{Value, ValueMap};

/// One entry of the blueprints' middleware chain.
#[derive(Clone)]
pub struct ChainLink {
    pub config: MiddlewareConfig,
    /// The built-in middleware `config` names; `None` when the core does not
    /// know the name, so the host may supply its own.
    pub middleware: Option<Arc<dyn Middleware>>,
}

pub struct Engine {
    pub blueprint_repo: Arc<YamlBlueprintRepository>,
    pub activity_executor: Arc<CallableActivityExecutor>,
    pub state_store: Arc<dyn IStateStore>,
    in_memory_store: Option<Arc<InMemoryStateStore>>,
    pub pipeline_manager: Arc<PipelineManager>,
    pub visualizer_service: Arc<VisualizerService>,
    pub api_key: String,
    pub async_dispatcher: Arc<dyn IAsyncActivityDispatcher>,
    #[cfg(feature = "temporal")]
    temporal_dispatcher: Option<Arc<TemporalAsyncActivityDispatcher>>,
    options: EngineOptions,
    /// Built on first use and kept, so rate-limit counters and caches live as
    /// long as the engine; registering a blueprint rebuilds it.
    middleware_chain: Mutex<Option<Arc<Vec<ChainLink>>>>,
}

impl Engine {
    /// An engine with an in-memory state store and dispatcher. An empty
    /// `api_key` leaves the engine's own routes unprotected.
    pub fn new(api_key: &str) -> Self {
        let repo = YamlBlueprintRepository::new();
        let executor = CallableActivityExecutor::new();
        let in_mem_store = InMemoryStateStore::new();
        let store: Arc<dyn IStateStore> = in_mem_store.clone();
        let dispatcher: Arc<dyn IAsyncActivityDispatcher> = InMemoryAsyncActivityDispatcher::new();

        let pipeline = PipelineManager::new_with_dispatcher(
            repo.clone(),
            executor.clone(),
            store.clone(),
            Some(dispatcher.clone()),
        );
        let visualizer = VisualizerService::new(repo.clone(), store.clone());

        Self {
            blueprint_repo: repo,
            activity_executor: executor,
            state_store: store,
            in_memory_store: Some(in_mem_store),
            pipeline_manager: pipeline,
            visualizer_service: visualizer,
            api_key: api_key.to_string(),
            async_dispatcher: dispatcher,
            #[cfg(feature = "temporal")]
            temporal_dispatcher: None,
            options: EngineOptions::default(),
            middleware_chain: Mutex::new(None),
        }
    }

    pub fn set_options(&mut self, opts: EngineOptions) {
        self.options = opts.clone();
        self.pipeline_manager.set_options(opts.clone());
        if let Some(ref mem) = self.in_memory_store {
            mem.set_limit(opts.max_stored_executions);
        }
    }

    pub fn options(&self) -> EngineOptions {
        self.options.clone()
    }

    // --- Wiring ---

    pub fn set_state_store(&mut self, store: Arc<dyn IStateStore>) {
        self.in_memory_store = None;
        self.state_store = store.clone();
        self.pipeline_manager.set_state_store(store.clone());
        self.visualizer_service.set_state_store(store);
    }

    pub fn set_async_dispatcher(&mut self, dispatcher: Arc<dyn IAsyncActivityDispatcher>) {
        self.async_dispatcher = dispatcher.clone();
        self.pipeline_manager.set_async_dispatcher(Some(dispatcher));
    }

    pub fn set_suspension_store(&self, store: Arc<dyn ISuspensionStore>) {
        self.pipeline_manager.set_suspension_store(store);
    }

    pub fn restore_suspensions(&self) -> Result<(), String> {
        self.pipeline_manager.restore_suspensions()
    }

    pub fn register_activity<H>(&self, name: &str, handler: H)
    where
        H: IntoActivityFunc,
    {
        self.activity_executor
            .register_activity(name, handler.into_activity_func());
    }

    pub fn register_activity_func(&self, name: &str, handler: ActivityFunc) {
        self.activity_executor.register_activity(name, handler);
    }

    /// Lets the host run activities it keeps in its own registry.
    pub fn set_activity_resolver(&self, resolver: Option<ActivityResolver>) {
        self.activity_executor.set_resolver(resolver);
    }

    // --- Blueprints ---

    pub fn register_blueprint(&mut self, bp: WorkflowBlueprint) -> Result<(), String> {
        self.configure_persistence_if_present(&bp);
        self.blueprint_repo.save_blueprint(bp)?;
        self.reset_middleware_chain();
        Ok(())
    }

    /// Registers a blueprint written as YAML (or JSON, which is YAML too).
    pub fn register_blueprint_yaml(&mut self, text: &str) -> Result<(), String> {
        let val = crate::yaml::parse_yaml(text)
            .map_err(|e| format!("failed to parse YAML blueprint: {}", e))?;
        self.register_blueprint(WorkflowBlueprint::from_value(&val)?)
    }

    pub fn load_blueprint_file(&mut self, file_path: &str) -> Result<(), String> {
        let content = fs::read_to_string(file_path)
            .map_err(|e| format!("failed to read blueprint file {}: {}", file_path, e))?;
        let val = crate::yaml::parse_yaml(&content)
            .map_err(|e| format!("failed to parse YAML blueprint {}: {}", file_path, e))?;
        let bp = WorkflowBlueprint::from_value(&val)?;
        self.register_blueprint(bp)
    }

    pub fn load_blueprints_from_directory(&mut self, dir_path: &str) -> Result<(), String> {
        self.blueprint_repo.load_from_directory(dir_path)?;
        let bps = self.blueprint_repo.list_blueprints().unwrap_or_default();
        for bp in bps {
            self.configure_persistence_if_present(&bp);
        }
        self.reset_middleware_chain();
        Ok(())
    }

    pub fn list_blueprints(&self) -> Vec<WorkflowBlueprint> {
        self.blueprint_repo.list_blueprints().unwrap_or_default()
    }

    pub fn set_global_exception_handler(
        &self,
        activity: &str,
        code: i32,
        message: &str,
        is_async: bool,
    ) {
        let cfg = GlobalExceptionHandlerConfig {
            activity: activity.to_string(),
            code,
            message: message.to_string(),
            is_async,
            r#async: is_async,
            include_stack_trace: false,
            headers: Default::default(),
        };
        for mut bp in self.list_blueprints() {
            bp.error_handling = Some(ErrorHandlingConfig {
                global_handler: Some(cfg.clone()),
            });
            bp.global_exception_handler = Some(cfg.clone());
            let _ = self.blueprint_repo.save_blueprint(bp);
        }
    }

    fn configure_persistence_if_present(&mut self, bp: &WorkflowBlueprint) {
        let Some(ref persistence) = bp.persistence else {
            return;
        };
        let store_type = persistence
            .get("store")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !store_type.is_empty() && store_type != "inmemory" {
            let factory = StoreFactory::new();
            let st = factory.create(store_type, Some(persistence));
            self.set_state_store(st);
        }
        self.configure_temporal_if_present(bp);
    }

    fn configure_temporal_if_present(&mut self, bp: &WorkflowBlueprint) {
        let Some(ref persistence) = bp.persistence else {
            return;
        };
        if let Some(Value::Object(temp_map)) = persistence.get("temporal") {
            #[cfg(feature = "temporal")]
            {
                let host = temp_map.get("host").and_then(|v| v.as_str()).unwrap_or("");
                let ns = temp_map
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tq = temp_map
                    .get("task_queue")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if self.async_dispatcher.get_dispatcher_type() == "IN_MEMORY" {
                    let temp_disp = TemporalAsyncActivityDispatcher::new(host, ns, tq);
                    self.temporal_dispatcher = Some(temp_disp.clone());
                    self.set_async_dispatcher(temp_disp);
                }
            }
            #[cfg(not(feature = "temporal"))]
            {
                let _ = temp_map;
                eprintln!(
                    "⚠️ [ENGINE] blueprint '{}' sets persistence.temporal, but this build has no Temporal support \
                     (enable the `temporal` feature); async steps run in-process",
                    bp.workflow_name
                );
            }
        }
    }

    // --- Temporal worker ---

    pub fn start_embedded_worker(&self) -> Result<(), String> {
        #[cfg(feature = "temporal")]
        if let Some(ref temp_disp) = self.temporal_dispatcher {
            return temp_disp.start_worker(self.activity_executor.clone());
        }
        Ok(())
    }

    pub fn stop_embedded_worker(&self) {
        #[cfg(feature = "temporal")]
        if let Some(ref temp_disp) = self.temporal_dispatcher {
            temp_disp.stop_worker();
        }
    }

    // --- Running workflows ---

    pub fn execute_sync(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        self.pipeline_manager.execute_sync(workflow_name, payload)
    }

    pub fn execute_endpoint_sync(
        &self,
        workflow_name: &str,
        endpoint: Option<&APIEndpointConfig>,
        route: &str,
        method: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        self.pipeline_manager
            .execute_endpoint_sync(workflow_name, endpoint, route, method, payload)
    }

    pub fn initiate_async(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, String> {
        self.pipeline_manager.initiate_async(workflow_name, payload)
    }

    pub fn signal(
        &self,
        job_id: &str,
        name: &str,
        data: ValueMap,
        by: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        self.pipeline_manager.signal(job_id, name, data, by)
    }

    /// Stores a state the host built itself (a request a middleware rejected,
    /// for example), so it is listed and shown like any other job.
    pub fn save_state(&self, state: ExecutionState) -> Result<(), String> {
        self.state_store.save_state(state)
    }

    pub fn get_execution_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        self.pipeline_manager.get_execution_state(job_id)
    }

    pub fn list_execution_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        self.pipeline_manager
            .list_execution_states(workflow_name, limit)
    }

    /// Live events for one job; call the returned function to unsubscribe.
    #[allow(clippy::type_complexity)]
    pub fn subscribe_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String> {
        self.state_store.subscribe_events(job_id)
    }

    /// The visualizer graph for a workflow, with a job's progress when
    /// `job_id` names one.
    pub fn compile_diagram(
        &self,
        workflow_name: &str,
        job_id: Option<&str>,
        route: Option<&str>,
        method: Option<&str>,
    ) -> Result<ValueMap, String> {
        self.visualizer_service
            .compile_x6_diagram(workflow_name, job_id, route, method)
    }

    // --- Requests ---

    /// The declared `api:` routes of every registered blueprint.
    pub fn declared_routes(&self) -> Vec<DeclaredRoute> {
        declared_routes(&self.list_blueprints())
    }

    /// Runs a declared route and returns the answer to send.
    pub fn call_route(&self, route: &DeclaredRoute, req: RouteRequest) -> RouteAnswer {
        let Ok(bp) = self.blueprint_repo.get_blueprint(&route.workflow_name) else {
            return RouteAnswer {
                status: 404,
                headers: Default::default(),
                body: Value::Object(crate::vmap! {
                    "error" => "WorkflowNotFound",
                    "detail" => format!("no blueprint named '{}'", route.workflow_name),
                }),
            };
        };
        run_declared_route(
            self.pipeline_manager.as_ref(),
            &bp,
            route,
            &self.options,
            req,
        )
    }

    /// Access to the tooling routes, given whether the host has a tooling
    /// authorizer and whether the client address is loopback.
    pub fn tooling_decision(
        &self,
        has_authorizer: bool,
        client_is_loopback: bool,
    ) -> ToolingDecision {
        decide_tooling_access(
            &self.options.tooling,
            has_authorizer,
            !self.api_key.is_empty(),
            client_is_loopback,
        )
    }

    /// Whether a request may use the engine's protected routes: the
    /// `X-API-Key` header, or the `api_key` query parameter when the options
    /// allow it, must match the engine's key. Always true without a key.
    pub fn api_key_allows(&self, header: &str, query: &str) -> bool {
        if self.api_key.is_empty() {
            return true;
        }
        let presented = if header.is_empty() && self.options.allow_api_key_in_query {
            query
        } else {
            header
        };
        constant_time_eq(presented.as_bytes(), self.api_key.as_bytes())
    }

    /// The middleware the registered blueprints declare, in order, one link per
    /// middleware name (the first blueprint to declare a name configures it).
    pub fn middleware_chain(&self) -> Arc<Vec<ChainLink>> {
        let mut guard = self.middleware_chain.lock().unwrap();
        if let Some(chain) = guard.as_ref() {
            return chain.clone();
        }
        let executor: Arc<dyn IActivityExecutor> = self.activity_executor.clone();
        let mut links: Vec<ChainLink> = Vec::new();
        for bp in self.list_blueprints() {
            for cfg in &bp.middlewares {
                if links.iter().any(|l| l.config.name == cfg.name) {
                    continue;
                }
                links.push(ChainLink {
                    middleware: build_middleware(
                        cfg,
                        Some(executor.clone()),
                        Some(self.async_dispatcher.clone()),
                    ),
                    config: cfg.clone(),
                });
            }
        }
        let chain = Arc::new(links);
        *guard = Some(chain.clone());
        chain
    }

    fn reset_middleware_chain(&self) {
        *self.middleware_chain.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::{ExecutionStatus, ToolingAccess};
    use crate::domain::services::RequestMeta;
    use std::collections::BTreeMap;

    const BLUEPRINT: &str = r#"
workflow_name: greet
version: "1.0.0"
middlewares:
  - name: ApiKeyAuthMiddleware
    config:
      api_keys: [k1]
  - name: SomethingCustom
  - name: ApiKeyAuthMiddleware
steps:
  - id: hello
    activity: Hello
    is_output: true
"#;

    #[test]
    fn runs_a_workflow_from_yaml_text() {
        let mut engine = Engine::new("");
        engine.register_blueprint_yaml(BLUEPRINT).unwrap();
        engine.register_activity_func(
            "Hello",
            Arc::new(|input: &ValueMap| {
                let name = input.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let mut out = ValueMap::new();
                out.insert("greeting".into(), Value::String(format!("hi {}", name)));
                Ok(out)
            }),
        );
        let mut input = ValueMap::new();
        input.insert("name".into(), Value::String("Ada".into()));
        let state = engine.execute_sync("greet", input).unwrap();
        assert_eq!(state.status, ExecutionStatus::Completed);
        let stored = engine.get_execution_state(&state.job_id).unwrap();
        assert_eq!(
            stored.data.get("greeting").and_then(|v| v.as_str()),
            Some("hi Ada")
        );
        let graph = engine
            .compile_diagram("greet", Some(&state.job_id), None, None)
            .unwrap();
        assert!(graph.contains_key("nodes"));
    }

    #[test]
    fn middleware_chain_is_built_once_per_name_and_kept() {
        let mut engine = Engine::new("");
        engine.register_blueprint_yaml(BLUEPRINT).unwrap();
        let chain = engine.middleware_chain();
        let names: Vec<_> = chain.iter().map(|l| l.config.name.as_str()).collect();
        assert_eq!(names, ["ApiKeyAuthMiddleware", "SomethingCustom"]);
        assert!(chain[0].middleware.is_some());
        assert!(chain[1].middleware.is_none());
        assert!(Arc::ptr_eq(&chain, &engine.middleware_chain()));

        let (headers, query, ctx) = (BTreeMap::new(), BTreeMap::new(), ValueMap::new());
        let req = RequestMeta {
            method: "GET",
            path: "/api/greet",
            headers: &headers,
            query: &query,
            client_ip: "127.0.0.1",
            body: b"",
            context: &ctx,
        };
        let decision = chain[0].middleware.as_ref().unwrap().before(&req);
        assert_eq!(decision.respond.map(|r| r.status), Some(401));
    }

    #[test]
    fn api_key_and_tooling_follow_the_options() {
        let mut engine = Engine::new("secret");
        assert!(engine.api_key_allows("secret", ""));
        assert!(!engine.api_key_allows("", "secret"));
        engine.set_options(EngineOptions {
            allow_api_key_in_query: true,
            ..Default::default()
        });
        assert!(engine.api_key_allows("", "secret"));
        assert!(!engine.api_key_allows("wrong", "secret"));
        assert_eq!(
            engine.tooling_decision(false, false),
            ToolingDecision::RequireApiKey
        );
        assert!(Engine::new("").api_key_allows("", ""));
        let mut open = Engine::new("");
        open.set_options(EngineOptions {
            tooling: ToolingAccess::Disabled,
            ..Default::default()
        });
        assert_eq!(open.tooling_decision(true, true), ToolingDecision::NotFound);
    }
}
