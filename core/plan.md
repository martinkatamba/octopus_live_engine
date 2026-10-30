# OCTOPUS Live Engine Core: plan

One engine, written once in Rust, used by the Go, Python, Java and .NET SDKs.

Progress is tracked in [done.md](done.md) and [to-be-done.md](to-be-done.md).

## Why

The live engine exists five times today (`../go`, `../python`, `../java`,
`../dotnet`, `../rust`), at 8–15k lines each. The copies drift. Comparing the
Rust engine with Go and Python turned up:

- state stores that were stubs;
- a fake Temporal dispatcher;
- a Redis cache that never used Redis;
- visualizer templates that differ by about 200 lines between Go and Python.

Every fix has to be made five times, and each copy has to be tested on its own.

The core removes that duplication. The behaviour that must be identical
everywhere is written once. Each SDK becomes a thin wrapper that talks to its
own web framework and runs its own activity code.

Precedent: Temporal's Python, TypeScript, .NET and Ruby SDKs share one Rust
`sdk-core`; pydantic-core, Polars and delta-rs work the same way. The
counter-example is Prisma, which dropped its Rust engine in 2025 because of
binary size, deployment complexity and serialization overhead. Lesson: keep the
boundary coarse. Calls cross it about once per workflow step, never once per
field.

## Architecture

```
            ┌───────────────── octopus-core (Rust) ─────────────────┐
            │ blueprints · pipeline · retries · sagas · idempotency  │
            │ middleware · masking · conditions · crypto             │
            │ state stores · event bus · async dispatch contract     │
            │ visualizer: diagram compiler + frontend asset          │
            └──────────┬───────────────────────────────────┬─────────┘
                       │ octopus-ffi (C ABI, JSON bytes)   │ optional adapters
       ┌───────────────┼───────────────┬───────────────┐   │ (Cargo features)
   Python (PyO3)  .NET (P/Invoke)  Java (FFM/JNI)  Go (decide last)
   FastAPI/Flask  ASP.NET Core     Spring          gin / net/http
       └── host owns: HTTP routes, auth middleware, SSE, activity code ──┘
```

### What lives where

| Concern | Core | Host SDK |
|---|---|---|
| Blueprint parsing and validation, step pipeline, conditions, retries, sagas, idempotency, masking | ✅ | |
| Middleware semantics (rate limiting, caching, auth decisions) | ✅ | Gives the core request metadata |
| State stores (memory, Redis, Postgres, MySQL, SQLite, SQL Server) using Go's schema and keys | ✅ | Only passes config |
| Live events (pub/sub, relayed between instances) | ✅ | Writes them out as SSE |
| Async dispatch contract (workflow name, payload, options, status transitions) | ✅ | |
| Temporal transport | Optional adapter (`temporal` feature) | Can plug in its native Temporal SDK instead |
| Visualizer diagram compiler (blueprint + state → X6 graph JSON) | ✅ | |
| Visualizer frontend (HTML/JS/CSS) | ✅ Embedded asset | Serves the bytes |
| Tooling access policy (localhost-only unless an API key is set) | ✅ Decision | Enforces it in the framework |
| HTTP routing, request parsing, OpenAPI | | ✅ |
| Activities (user code) | Calls them back | ✅ |

### Workspace layout

```
core/
  Cargo.toml                 workspace
  crates/octopus-core/       engine library: no HTTP, drivers behind features
  crates/octopus-ffi/        C ABI (cdylib + staticlib); header from cbindgen
  conformance/               (phase 0) language-neutral golden tests
  bindings/python|dotnet|java|go/   (phases 2–6)
```

`../rust` (the Rust live engine and its HTTP server) becomes the first
consumer, with a path dependency on `core/crates/octopus-core`.

## The FFI contract

Everything crosses as UTF-8 JSON in an `OctopusBuf { ptr, len, cap }`. A buffer
is freed by the side that allocated it. Errors are a status code plus a JSON
error `{code, type, message}` using the same fields as `ExecutionState.error*`.

```
octopus_contract_version() -> u32
octopus_core_version() -> buf
octopus_last_error() -> buf                          // {code, type, message}; empty after a success
octopus_buf_free(buf)
octopus_engine_new(config_json) -> *Engine           // {api_key, options}; NULL + last error
octopus_engine_free(*Engine)
octopus_register_blueprint(*Engine, yaml_or_json) -> status
octopus_register_activity(*Engine, name, callback, user_data) -> status
    callback(user_data, input, input_len, *out) -> status   // runs on any core thread
    octopus_out_write(*out, bytes, len)              // the result (JSON object), or the error text
octopus_execute(*Engine, workflow, input_json, *out_state) -> status
octopus_initiate_async(*Engine, workflow, input_json, *out_state) -> status
octopus_signal(*Engine, job_id, name, data_json, by_json, *out_state) -> status
octopus_get_state(*Engine, job_id, *out_state) -> status
octopus_list_states(*Engine, workflow_or_null, limit, *out_json) -> status
octopus_declared_routes(*Engine, *out_json) -> status
octopus_call_route(*Engine, method, route, request_json, *out_answer) -> status   // {status, headers, body}
octopus_tooling_decision(*Engine, request_json, *out_json) -> status
octopus_api_key_allows(*Engine, header, query) -> bool
octopus_middleware_chain(*Engine, *out_json) -> status      // [{name, built_in}], in order
octopus_middleware_before(*Engine, index, request_json, *out_decision_json) -> status
octopus_middleware_after(*Engine, index, request_json, response_json) -> status
octopus_subscribe_events(*Engine, job_id, callback, user_data) -> subscription_id
octopus_unsubscribe(*Engine, subscription_id)
octopus_compile_diagram(*Engine, workflow, job_id_or_null, *out_graph_json) -> status
octopus_visualizer_html(route_prefix, default_domain, *out_html) -> status
octopus_start_worker(*Engine) / octopus_stop_worker(*Engine) -> status
```

The header is `crates/octopus-ffi/include/octopus.h`, generated by cbindgen.
Python uses PyO3 instead of this ABI (`bindings/python`), but follows the same
shapes.

Rules:

- **Threading.** The core owns its threads, and callbacks may arrive on any of
  them. Each host must handle that:
  - Python re-acquires the GIL;
  - Java attaches the thread to the JVM;
  - .NET keeps its delegates alive;
  - Go uses cgo/purego callbacks.
- **No panics across the boundary.** Every exported function runs inside
  `catch_unwind` and turns a panic into an error status.
- **Versioning.** Any breaking change to the functions, the JSON shapes or the
  graph schema bumps `CONTRACT_VERSION`. SDKs check it at load time and refuse
  to run on a mismatch.

## Decisions

1. **Temporal is an adapter behind `IAsyncActivityDispatcher`; it is not
   built into the core.**
   - `sdk-core`, the layer underneath, is production-grade. The high-level Rust
     API is the newest layer.
   - Our exposure to that newer layer is small: one fixed workflow
     (`IOctopusWorkflow`) making one activity call, with no signals, timers or
     versioning.
   - The core owns the **contract**, so a Go or Java worker can pick up a
     workflow that Rust dispatched, and the other way round:
     - the workflow name;
     - the payload encoding;
     - how retry and timeout options map;
     - the status transitions;
     - the completion hook.
   - The Rust adapter is the default for the Python and .NET bindings. Go and
     Java can keep their native Temporal SDKs through the same port.
   - Versions are pinned exactly. Upgrades must pass the real-server
     integration test.
2. **The visualizer belongs in the core.**
   - The diagram compiler is a pure function, JSON in and JSON out.
   - The frontend is a single embedded asset, so every SDK serves the same
     bytes.
   - The graph JSON schema is a versioned contract covered by golden tests.
3. **The host keeps HTTP.** Users expect FastAPI, Spring, ASP.NET and gin. The
   core makes decisions; the host does the transport.
4. **Go is decided last.** Go is already fast, and cgo breaks easy
   cross-compilation and static binaries. Go may stay native and be held to the
   conformance suite instead.
5. **The conformance suite comes first.** It defines "correct" independently of
   any one implementation, and it is worth having even if the core is
   abandoned.

## Phases

| # | Phase | Work | Exit criteria | Estimate |
|---|---|---|---|---|
| 0 | Conformance suite | Language-neutral cases in `conformance/`: blueprint + input + activity script → expected traces, statuses, events, stored rows and graph JSON. A small runner in each language. | The current Go, Python and Rust engines pass, and the gaps are listed. | 1–2 wk |
| 1 | Extract the core | Move the domain services, models, ports, outbound adapters, crypto/json/yaml and the visualizer compiler and frontend from `../rust` into `octopus-core`. `../rust` keeps `http.rs` and `rest_handler.rs` and depends on the core. | `../rust` tests are green and the conformance suite passes against the core. | 1–2 wk |
| 2 | FFI and Python spike | Implement the contract in `octopus-ffi` (cbindgen header, `catch_unwind`, a callback trampoline). Build a PyO3/maturin binding and run one example (`order_fulfillment_api`). Measure per-step overhead against pure Python. | The numbers are written up. **Go/no-go decision.** | 2 wk |
| 3 | Python SDK on the core | Swap out the internals of `octopus_live` and keep its public API. CI builds manylinux, macOS and Windows wheels. | The Python examples and robot tests pass unchanged. | 2–3 wk |
| 4 | .NET SDK on the core | P/Invoke through `csbindgen`; NuGet package with a `runtimes/{rid}/native`. | The .NET examples pass. | 2–3 wk |
| 5 | Java SDK on the core | Panama FFM (Java 22+) or JNI; Maven classifier jars per platform. | The Java examples pass. | 3 wk |
| 6 | Go: decide | Either keep it native under the conformance suite, or use `purego` to load the core without cgo. | Decision recorded with benchmarks. | 1 wk |
| 7 | Retire the duplicates | Each SDK runs on the core behind a flag for one release, then its old engine internals are deleted. | Only the core and thin wrappers remain. | ongoing |

Total: about 3–4 months for one person. Python and .NET give the earliest
payoff.

## Risks

| Risk | Mitigation |
|---|---|
| Native packaging cost: 6+ targets × 4 languages | A single CI matrix builds `octopus-ffi` once per target. Every binding consumes the same artifacts. |
| Callback overhead or thread problems | Measure in phase 2. The boundary is per step, not per field. Host-specific trampolines. |
| Changes in the Temporal Rust API | Adapter behind a port, pinned versions, real-server test in CI, host-native escape hatch. |
| A core bug hits every language at once | The conformance suite gates every core release. Bindings pin a core version. |
| Debugging across the boundary | Structured errors with codes. Every log line carries the job ID. |
| Prisma-style regret (size, deployment, serialization) | Optional features keep the default binary small. JSON at step granularity. The go/no-go decision in phase 2 is based on measurements. |
