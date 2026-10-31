//! The Rust host's engine: the core `Engine` plus what only an HTTP host has,
//! namely routes, OpenAPI models, host middleware and the tooling authorizer.
//! Everything else (`execute_sync`, `register_blueprint`, the public fields,
//! ...) comes from the core through `Deref`.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::adapters::inbound::{register_live_routes_with_config, RouteConfig, ToolingAuthorizer};
use crate::domain::models::MiddlewareConfig;
use crate::domain::ports::{
    IActivityExecutor, IAsyncActivityDispatcher, IVisualizerCompiler, IWorkflowEngine,
};
use crate::domain::services::{into_handler, CustomMiddlewareFactory};
use crate::http::{HandlerFunc, HttpContext, Router};
use crate::json::Value;
use octopus_core::Engine;

pub struct OctopusLiveEngine {
    core: Engine,
    models: HashMap<String, Value>,
    middlewares: Vec<HandlerFunc>,
    custom_middleware_registry: HashMap<String, CustomMiddlewareFactory>,
    tooling_authorizer: Option<ToolingAuthorizer>,
    router: Option<Router>,
}

impl Deref for OctopusLiveEngine {
    type Target = Engine;

    fn deref(&self) -> &Engine {
        &self.core
    }
}

impl DerefMut for OctopusLiveEngine {
    fn deref_mut(&mut self) -> &mut Engine {
        &mut self.core
    }
}

impl OctopusLiveEngine {
    pub fn new(api_key: &str) -> Self {
        Self {
            core: Engine::new(api_key),
            models: HashMap::new(),
            middlewares: Vec::new(),
            custom_middleware_registry: HashMap::new(),
            tooling_authorizer: None,
            router: None,
        }
    }

    pub fn new_live_engine(api_key: &str) -> Self {
        Self::new(api_key)
    }

    pub fn set_tooling_authorizer<F>(&mut self, func: F)
    where
        F: Fn(&mut HttpContext) -> bool + Send + Sync + 'static,
    {
        self.tooling_authorizer = Some(Arc::new(func));
    }

    pub fn register_model(&mut self, name: &str, model_schema: Value) {
        self.models.insert(name.to_string(), model_schema);
    }

    pub fn register_custom_middleware<F>(&mut self, name: &str, factory: F)
    where
        F: Fn(
                &MiddlewareConfig,
                Option<Arc<dyn IActivityExecutor>>,
                Option<Arc<dyn IAsyncActivityDispatcher>>,
            ) -> HandlerFunc
            + Send
            + Sync
            + 'static,
    {
        self.custom_middleware_registry
            .insert(name.to_string(), Arc::new(factory));
    }

    pub fn use_middleware<F>(&mut self, mw: F)
    where
        F: Fn(&mut HttpContext) + Send + Sync + 'static,
    {
        self.middlewares.push(Arc::new(mw));
    }

    pub fn add_middleware(&mut self, mw: HandlerFunc) {
        self.middlewares.push(mw);
    }

    pub fn start_server(&mut self, port: u16) -> std::io::Result<()> {
        let _ = self.start_embedded_worker();
        let mut router = Router::new();
        self.mount_routes(&mut router);
        let addr = format!("0.0.0.0:{}", port);
        let res = router.run(&addr);
        self.stop_embedded_worker();
        res
    }

    pub fn mount_routes(&self, router: &mut Router) {
        for link in self.middleware_chain().iter() {
            let custom = self.custom_middleware_registry.get(link.config.name.trim());
            if let Some(factory) = custom {
                let exec: Arc<dyn IActivityExecutor> = self.activity_executor.clone();
                router.use_middleware(factory(
                    &link.config,
                    Some(exec),
                    Some(self.async_dispatcher.clone()),
                ));
            } else if let Some(mw) = &link.middleware {
                router.use_middleware(into_handler(mw.clone()));
            }
        }

        for mw in &self.middlewares {
            router.use_middleware(mw.clone());
        }

        let options = self.options();
        if router
            .set_trusted_proxies(&options.trusted_proxies)
            .is_err()
        {
            let _ = router.set_trusted_proxies(&[]);
        }

        let wf_uc: Arc<dyn IWorkflowEngine> = self.pipeline_manager.clone();
        let viz_uc: Arc<dyn IVisualizerCompiler> = self.visualizer_service.clone();

        register_live_routes_with_config(
            router,
            wf_uc,
            viz_uc,
            RouteConfig {
                api_key: self.api_key.clone(),
                options,
                tooling_authorizer: self.tooling_authorizer.clone(),
                registered_models: self.models.clone(),
            },
        );
    }

    pub fn get_router(&mut self) -> Router {
        if self.router.is_none() {
            let mut r = Router::new();
            self.mount_routes(&mut r);
            self.router = Some(r);
        }
        self.router.clone().unwrap()
    }
}
