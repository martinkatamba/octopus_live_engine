//! The Rust live engine: an HTTP host over `octopus-core`
//! (`../core/crates/octopus-core`). The engine logic lives in the core; this
//! crate keeps the HTTP server (`http`), the REST routes and the HTTP
//! middleware, and re-exports the core at its old paths.

pub mod adapters;
pub mod domain;
pub mod engine;
pub mod http;

pub use octopus_core::{crypto, json, net, vmap, yaml};

pub use engine::OctopusLiveEngine;
pub use http::{HandlerFunc, HttpContext, HttpRequest, HttpResponse, Router};
pub use json::{Value, ValueMap};

pub fn new_live_engine(api_key: &str) -> OctopusLiveEngine {
    OctopusLiveEngine::new_live_engine(api_key)
}
