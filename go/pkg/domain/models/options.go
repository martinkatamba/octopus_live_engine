package models

// ToolingAccess controls who may use the built-in tooling routes: the
// visualizer, execution history, live streams, diagrams and API docs.
type ToolingAccess string

const (
	// ToolingLocalOnly (default) serves tooling to loopback clients only,
	// unless an API key or a ToolingAuthorizer is configured.
	ToolingLocalOnly ToolingAccess = "local"
	// ToolingAPIKey requires the engine API key (X-API-Key header).
	ToolingAPIKey ToolingAccess = "api_key"
	// ToolingOpen serves tooling to anyone (the pre-hardening behaviour).
	ToolingOpen ToolingAccess = "open"
	// ToolingDisabled does not mount any tooling route.
	ToolingDisabled ToolingAccess = "disabled"
)

// EngineOptions are security and runtime settings. The zero value is the
// secure default; each Allow*/Expose* field opts back into older behaviour.
type EngineOptions struct {
	// AllowClientJobIDs trusts X-Job-ID / X-Request-ID / X-Correlation-ID
	// headers, ?job_id= and body job_id as the execution id. Job ids are also
	// idempotency keys, so a client could replay another caller's response;
	// by default the server generates unguessable ids.
	AllowClientJobIDs bool

	// AllowAPIKeyInQuery accepts ?api_key= in addition to the X-API-Key header.
	// Query strings end up in logs, proxies and browser history.
	AllowAPIKeyInQuery bool

	// TrustedProxies may set X-Forwarded-For / X-Real-IP. Empty (default):
	// the TCP peer address is the client, so IP rules can't be spoofed.
	TrustedProxies []string

	// Tooling decides access to visualizer/history/docs routes ("" = local).
	Tooling ToolingAccess

	// ExposeErrorDetails returns unexpected activity errors to clients
	// verbatim. By default clients get a generic message with the job id and
	// the detail is kept in the execution trace.
	ExposeErrorDetails bool

	// MaxStoredExecutions bounds the in-memory state store (default 1000).
	MaxStoredExecutions int
}
