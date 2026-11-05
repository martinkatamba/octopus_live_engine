//! Runs the core's middleware on this crate's HTTP context. The decisions
//! (rate limiting, auth, caching, ...) are made in the core; this file reads
//! each request into a `RequestMeta` and applies the `Decision`.

pub use octopus_core::domain::services::middleware::*;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::domain::models::{MiddlewareConfig, MiddlewareExceptionConfig};
use crate::domain::ports::{IActivityExecutor, IAsyncActivityDispatcher};
use crate::http::{HandlerFunc, HttpContext};

fn request_meta<'a>(c: &'a HttpContext, client_ip: &'a str) -> RequestMeta<'a> {
    RequestMeta {
        method: &c.method,
        path: &c.path,
        headers: &c.req_headers,
        query: &c.query_params,
        client_ip,
        body: &c.body,
        context: &c.store,
    }
}

/// Applies a decision; true when it answered the request.
fn apply(c: &mut HttpContext, decision: Decision) -> bool {
    for (k, v) in decision.headers {
        c.header(&k, &v);
    }
    for (k, v) in decision.context {
        c.set(&k, v);
    }
    match decision.respond {
        Some(r) => {
            c.data(r.status, &r.content_type, r.body);
            c.abort();
            true
        }
        None => false,
    }
}

/// A handler that runs a core middleware around the rest of the chain.
pub fn into_handler(mw: Arc<dyn Middleware>) -> HandlerFunc {
    Arc::new(move |c: &mut HttpContext| {
        let client_ip = c.client_ip();
        let decision = mw.before(&request_meta(c, &client_ip));
        if apply(c, decision) {
            return;
        }
        let start = Instant::now();
        c.next();
        let resp = ResponseMeta {
            status: c.status_code,
            body: &c.resp_body,
            duration_ms: (start.elapsed().as_micros() as f64) / 1000.0,
        };
        mw.after(&request_meta(c, &client_ip), &resp);
    })
}

/// `.handler()` on any core middleware.
pub trait MiddlewareHandler {
    fn handler(self) -> HandlerFunc;
}

impl<M: Middleware + 'static> MiddlewareHandler for M {
    fn handler(self) -> HandlerFunc {
        into_handler(Arc::new(self))
    }
}

pub fn respond_middleware_error(
    c: &mut HttpContext,
    err: OctopusMiddlewareError,
    exceptions: &[MiddlewareExceptionConfig],
    executor: Option<&Arc<dyn IActivityExecutor>>,
    dispatcher: Option<&Arc<dyn IAsyncActivityDispatcher>>,
) {
    let client_ip = c.client_ip();
    let responder =
        ErrorResponder::new(exceptions.to_vec(), executor.cloned(), dispatcher.cloned());
    let decision = responder.reject(&request_meta(c, &client_ip), err);
    apply(c, decision);
}

pub type CustomMiddlewareFactory = Arc<
    dyn Fn(
            &MiddlewareConfig,
            Option<Arc<dyn IActivityExecutor>>,
            Option<Arc<dyn IAsyncActivityDispatcher>>,
        ) -> HandlerFunc
        + Send
        + Sync,
>;

/// The handler for a blueprint middleware: a registered custom one first,
/// otherwise the core's built-in one.
pub fn build_middleware_from_config(
    cfg: &MiddlewareConfig,
    executor: Option<Arc<dyn IActivityExecutor>>,
    dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    custom_registry: Option<&HashMap<String, CustomMiddlewareFactory>>,
) -> Option<HandlerFunc> {
    if let Some(factory) = custom_registry.and_then(|reg| reg.get(cfg.name.trim())) {
        return Some(factory(cfg, executor, dispatcher));
    }
    build_middleware(cfg, executor, dispatcher).map(into_handler)
}
