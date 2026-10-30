//! Outbound adapters: state stores, async dispatchers and the activity
//! executor. Inbound adapters (HTTP routes) belong to the host.
pub mod outbound;

pub use outbound::*;
