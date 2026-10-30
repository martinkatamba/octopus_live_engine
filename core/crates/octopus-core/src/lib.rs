//! OCTOPUS Live Engine core.
//!
//! The engine logic every SDK shares: blueprints, the step pipeline, retries,
//! middleware, state stores, async dispatch and the visualizer's diagram
//! compiler. HTTP routing and activity code stay in the host language.
//! See `plan.md` at the workspace root.

/// Version of this crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the contracts the host SDKs depend on: the FFI functions, the
/// JSON shapes that cross them, and the visualizer's graph schema. Bump it on
/// any breaking change; SDKs refuse to load a core with a different value.
pub const CONTRACT_VERSION: u32 = 0;

pub mod adapters;
pub mod crypto;
pub mod domain;
pub mod engine;
pub mod json;
pub mod net;
pub mod url;
pub mod visualizer_html;
mod workers;
pub mod yaml;

pub use engine::Engine;
pub use json::{Value, ValueMap};
