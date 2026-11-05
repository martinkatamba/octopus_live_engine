# OCTOPUS Live Engine (Rust)

Runs OCTOPUS YAML workflow blueprints inside your process: no Temporal server, worker or queue. It suits request-reply APIs that answer from memory, such as fraud scoring, tax and pricing, or login flows.

- Steps with conditions, loops, jumps, retries, timeouts and saga compensation.
- Declarative middlewares: API key, HS256 JWT, IP allow-list, rate limit, payload validation, caching and audit.
- An HTTP server with an interactive visualizer at `/visualizer`, and live events per job at `/stream/<job_id>`.
- Jobs in memory, or in Redis, Postgres, MySQL, SQL Server or SQLite.

The engine itself is [`octopus-core`](https://crates.io/crates/octopus-core), the same core the Python SDK runs on; this crate adds the HTTP routes, middlewares and visualizer around it.

## Use

```rust
use octopus_live_engine::{OctopusLiveEngine, ValueMap, vmap};

let mut engine = OctopusLiveEngine::new("");
engine.load_blueprints_from_directory("./config").unwrap();

engine.register_activity("CalculateTax", |input: &ValueMap| -> Result<ValueMap, String> {
    let amount = input.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    Ok(vmap! {
        "tax" => amount * 0.15,
        "total" => amount * 1.15,
    })
});

// Run a workflow in the request...
let state = engine.execute_sync("order_tax_pipeline", vmap! { "amount" => 200.0 }).unwrap();

// ...or serve every workflow over HTTP, with the visualizer.
engine.start_server(8001).unwrap();
```

Every workflow is then served under `/api/v1.0.0/<workflow>`: `POST /execute` runs it in the request, and `POST /initiate` returns a job ID and runs it in the background.

## Features

| Feature | Default | Adds |
|---|:---:|---|
| `redis`, `postgres`, `mysql`, `sqlite`, `mssql` | ✅ | The state stores |
| `temporal` | | Async steps on Temporal (`persistence.temporal`); building it needs `protoc` |

`--no-default-features` builds a memory-only engine with no dependencies.

The blueprint format, middlewares and visualizer are documented in the [OCTOPUS Live Engine repository](https://github.com/martinkatamba/octopus_live_engine).

## Licence

MIT; see `LICENSE`.
