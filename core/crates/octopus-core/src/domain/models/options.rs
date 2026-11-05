#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ToolingAccess {
    /// Default: serves tooling to loopback clients only, unless an API key or
    /// a ToolingAuthorizer is configured.
    #[default]
    LocalOnly,
    /// Requires the engine API key (X-API-Key header).
    APIKey,
    /// Serves tooling to anyone (the pre-hardening behaviour).
    Open,
    /// Does not mount any tooling route.
    Disabled,
}

impl ToolingAccess {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolingAccess::LocalOnly => "local",
            ToolingAccess::APIKey => "api_key",
            ToolingAccess::Open => "open",
            ToolingAccess::Disabled => "disabled",
        }
    }
}

/// Security and runtime settings. The Default value is the secure default;
/// each `allow_*`/`expose_*` field opts back into older behaviour.
#[derive(Clone, Debug, Default)]
pub struct EngineOptions {
    /// Trusts X-Job-ID / X-Request-ID / X-Correlation-ID headers, ?job_id= and
    /// body job_id as the execution id.
    pub allow_client_job_ids: bool,
    /// Accepts ?api_key= in addition to the X-API-Key header.
    pub allow_api_key_in_query: bool,
    /// Trusted proxies that may set X-Forwarded-For / X-Real-IP.
    pub trusted_proxies: Vec<String>,
    /// Decides access to visualizer/history/docs routes.
    pub tooling: ToolingAccess,
    /// Returns unexpected activity errors to clients verbatim.
    pub expose_error_details: bool,
    /// Bounds the in-memory state store (default 1000 when <= 0).
    pub max_stored_executions: usize,
}

impl ToolingAccess {
    /// Parses [`ToolingAccess::as_str`]'s names; anything else is `LocalOnly`.
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "api_key" | "apikey" => ToolingAccess::APIKey,
            "open" => ToolingAccess::Open,
            "disabled" => ToolingAccess::Disabled,
            _ => ToolingAccess::LocalOnly,
        }
    }
}

impl EngineOptions {
    /// Reads options from JSON with the field names above; missing fields keep
    /// their secure defaults. `tooling` takes the [`ToolingAccess::as_str`] names.
    pub fn from_value(v: &crate::json::Value) -> Self {
        let get = |k: &str| v.as_object().and_then(|m| m.get(k));
        let flag = |k: &str| get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        EngineOptions {
            allow_client_job_ids: flag("allow_client_job_ids"),
            allow_api_key_in_query: flag("allow_api_key_in_query"),
            trusted_proxies: get("trusted_proxies")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            tooling: get("tooling")
                .and_then(|x| x.as_str())
                .map(ToolingAccess::parse)
                .unwrap_or_default(),
            expose_error_details: flag("expose_error_details"),
            max_stored_executions: get("max_stored_executions")
                .and_then(|x| x.as_i64())
                .filter(|n| *n > 0)
                .unwrap_or(0) as usize,
        }
    }
}
