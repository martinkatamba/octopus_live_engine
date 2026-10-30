//! Who may use the built-in tooling routes (visualizer, history, live
//! streams, diagrams, API docs). The core decides; the host enforces the
//! decision in its HTTP framework.

use crate::domain::models::ToolingAccess;

/// What the host should do with a request for a tooling route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolingDecision {
    /// Serve the route.
    Allow,
    /// Answer 404, as if the route did not exist.
    NotFound,
    /// Run the host's tooling authorizer and follow its verdict.
    AskAuthorizer,
    /// Require the engine API key; the host checks it.
    RequireApiKey,
    /// Answer 403 with this detail.
    Deny(&'static str),
}

pub const DENY_NO_API_KEY: &str = "Tooling requires an API key, but none is configured.";
pub const DENY_NOT_LOCAL: &str = "The visualizer and execution history are only available from this machine. Configure an API key, a ToolingAuthorizer or EngineOptions.Tooling to allow remote access.";

/// Decides access to a tooling route. `client_is_loopback` is the host's
/// verdict on the client address, after applying its trusted proxies.
pub fn decide_tooling_access(
    tooling: &ToolingAccess,
    has_authorizer: bool,
    api_key_configured: bool,
    client_is_loopback: bool,
) -> ToolingDecision {
    if *tooling == ToolingAccess::Disabled {
        return ToolingDecision::NotFound;
    }
    if has_authorizer {
        return ToolingDecision::AskAuthorizer;
    }
    if *tooling == ToolingAccess::Open {
        return ToolingDecision::Allow;
    }
    if *tooling == ToolingAccess::APIKey || api_key_configured {
        return if api_key_configured {
            ToolingDecision::RequireApiKey
        } else {
            ToolingDecision::Deny(DENY_NO_API_KEY)
        };
    }
    if client_is_loopback {
        ToolingDecision::Allow
    } else {
        ToolingDecision::Deny(DENY_NOT_LOCAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ToolingAccess::*;
    use ToolingDecision::*;

    #[test]
    fn each_policy() {
        // (policy, authorizer, api key, loopback) -> decision
        let cases = [
            (Disabled, true, true, true, NotFound),
            (LocalOnly, true, false, false, AskAuthorizer),
            (Open, false, false, false, Allow),
            (APIKey, false, true, false, RequireApiKey),
            (APIKey, false, false, true, Deny(DENY_NO_API_KEY)),
            (LocalOnly, false, true, false, RequireApiKey),
            (LocalOnly, false, false, true, Allow),
            (LocalOnly, false, false, false, Deny(DENY_NOT_LOCAL)),
        ];
        for (policy, authz, key, local, want) in cases {
            assert_eq!(
                decide_tooling_access(&policy, authz, key, local),
                want,
                "{:?} authorizer={} key={} loopback={}",
                policy,
                authz,
                key,
                local
            );
        }
    }
}
