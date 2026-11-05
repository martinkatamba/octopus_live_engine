# OCTOPUS Live Engine ⚡

> **Ultra-Low Latency, Pure In-Memory Polyglot Workflow Execution Engine** with Interactive AntV X6 Visualizer and Hexagonal Architecture.

`octopus_live_engine` runs OCTOPUS YAML blueprints inside one process, with no Temporal server, worker or queue. It is meant for request-reply APIs that answer from memory, such as fraud scoring, tax and pricing, or login flows.

---

## Key Features

- ⚡ **In-process execution**: no Temporal server, Redis, Kafka or database required; steps are function calls in the same process.
- 📐 **Strict Hexagonal Architecture**: Clean separation between domain pipeline services, inbound ports (`ILiveWorkflowUseCase`, `IVisualizerUseCase`), and outbound adapters (`IActivityExecutor`, `IBlueprintRepository`, `IStateStore`).
- 🌐 **Interactive AntV X6 Visualizer**: Embedded real-time DAG graph at `/visualizer` and `/diagram/x6` with execution state drawers.
- 📡 **Server-Sent Events (SSE) Streaming**: Real-time event streaming per job at `GET /stream/{job_id}`.
- 🔄 **In-Memory Saga Rollbacks**: Automatic reverse compensation execution on step failure.
- 🛡️ **Declarative middlewares**: API key, HS256 JWT, IP allow-list, rate limit, payload validation, caching and audit, declared in the blueprint.
- 🌍 **Five SDKs**: **Python**, **Go**, **Rust**, **.NET 8** and **Java 17+** share the blueprint format, routes and visualizer. The [Live Engine overview](../octopus_workflow_docs/docs/09-live-engine/overview.md#differences-between-the-sdks) lists where they differ.

---

## Architecture (Ports & Adapters)

```
                       INBOUND ADAPTERS
       ┌──────────────────────────────────────────────┐
       │  FastAPI / Gin / Rust / ASP.NET / HttpServer │
       │  AntV X6 Interactive Web Visualizer (HTML)   │
       │  Server-Sent Events (SSE) Stream Subscriber  │
       └──────────────────────┬───────────────────────┘
                              │
                    ┌─────────▼─────────┐
                    │   INBOUND PORTS   │
                    │  - ILiveWorkflow  │
                    │  - IVisualizer    │
                    └─────────┬─────────┘
                              │
               ┌──────────────▼──────────────┐
               │        DOMAIN CORE          │
               │  - PipelineManager (DAG)    │
               │  - ConditionEvaluator       │
               │  - SagaManager (Rollbacks)  │
               │  - IdempotencyManager (LRU) │
               └──────────────┬──────────────┘
                              │
                    ┌─────────▼─────────┐
                    │  OUTBOUND PORTS   │
                    │  - IExecutor      │
                    │  - IRepository    │
                    │  - IStateStore    │
                    └─────────┬─────────┘
                              │
       ┌──────────────────────▼───────────────────────┐
       │  Callable / Function Activity Invokers       │
       │  YAML File / Memory Blueprint Repositories   │
       │  Thread-Safe Concurrent Map State Stores     │
       └──────────────────────────────────────────────┘
                       OUTBOUND ADAPTERS
```

---

## Start a service

The [OCTOPUS CLI](../octopus_workflow_cli_go) scaffolds a complete live service in any of the supported languages. Create it at the root of the OCTOPUS checkout so it finds this engine:

```bash
(cd octopus_workflow_cli_go && go install ./cmd/octopus)   # once
octopus init --name payments --lang go --engine live      # or --lang python | dotnet | java
```

| Language | Run it (port 8001) |
|---|---|
| Go | `cd payments && make run` |
| Rust | `cd octopus_live_engine/examples/rust/todo_api && cargo run` |
| Python | `cd payments && uv sync && uv run python main.py` |
| .NET | `cd payments && make run` |
| Java | `(cd octopus_live_engine/java && mvn -q -DskipTests install)` once, then `cd payments && make run` |

Every workflow can then be called two ways under `/api/v1.0.0/<workflow>`:

- **Sync**: `POST /execute` runs every step inside the request and returns the job data.
- **Async**: `POST /initiate` returns the job ID at once and runs the steps in the background of the same process; follow it with `GET /status/<job_id>`, `GET /timeline/<job_id>` or `GET /stream/<job_id>`. Async jobs live in memory unless you configure a state store.

```bash
curl -s -X POST http://localhost:8001/api/v1.0.0/payments/execute \
  -H "Content-Type: application/json" -H "X-API-Key: secret-api-key-12345" \
  -d '{"amount": 1000, "customer_id": "CUST-1"}'
```

Differences between the SDKs worth knowing:

- **Job IDs**: Python, .NET and Java use the `job_id` you send. Go and Rust generate their own unless the service enables `AllowClientJobIDs` / `allow_client_job_ids`, so use the `job_id` they return.
- **Failed sync jobs**: Go, Rust, .NET and Java answer `/execute` with HTTP 500; Python answers 200 with the data collected so far (`?raw=true` shows `"status": "FAILED"` and the traces).
- **Blueprints**: the Go and Rust engines refuse `allowed_initiator_roles`; authorize callers with a middleware instead.
- **Visualizer**: the Go and Rust engines protect it with the API key, so the browser has to send `X-API-Key`.

Step-by-step guides per language are in the docs: *Getting Started → Live Engine Quick Guides* (`octopus_workflow_docs/docs/01-getting-started/live/`).

---

## Embedding the engine

### Python

```python
from octopus_live import OctopusLiveEngine

engine = OctopusLiveEngine()

@engine.activity("CalculateTax")
def calculate_tax(payload):
    amount = payload.get("amount", 0.0)
    return {"tax": amount * 0.15, "total": amount * 1.15}

# Load blueprints from YAML
engine.load_blueprints_from_directory("./config")

# Execute synchronous in-memory workflow
result = await engine.execute_sync("order_tax_pipeline", {"amount": 200.0})
print("Result:", result.data)
```

### Go

```go
engine := pkg.NewOctopusLiveEngine("")
if err := engine.LoadBlueprintsFromDirectory("./config"); err != nil {
    log.Fatal(err)
}

engine.RegisterActivity("CalculateTax", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
    amount := input["amount"].(float64)
    return map[string]interface{}{"tax": amount * 0.15, "total": amount * 1.15}, nil
})

state, err := engine.ExecuteSync(ctx, "order_tax_pipeline", map[string]interface{}{"amount": 200.0})
```

### Rust

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

let state = engine.execute_sync("order_tax_pipeline", vmap! { "amount" => 200.0 }).unwrap();
```

### .NET (C#)

```csharp
var engine = new OctopusLiveEngine();
engine.LoadBlueprintsFromDirectory("./Config");

engine.RegisterActivity("CalculateTax", input => {
    var amount = Convert.ToDouble(input["amount"]);
    return new Dictionary<string, object> { ["tax"] = amount * 0.15, ["total"] = amount * 1.15 };
});

var state = await engine.ExecuteSyncAsync("order_tax_pipeline", new Dictionary<string, object> { ["amount"] = 200.0 });
```

### Java

```java
LiveEngine engine = new LiveEngine();
engine.loadBlueprintsFromDirectory("./config");

engine.registerActivity("CalculateTax", input -> {
    double amount = ((Number) input.get("amount")).doubleValue();
    // Return a mutable map: the engine merges step outputs into it.
    return new HashMap<>(Map.of("tax", amount * 0.15, "total", amount * 1.15));
});

ExecutionState state = engine.executeSync("order_tax_pipeline", Map.of("amount", 200.0));
```

---

## Endpoints

| Method | Route | Description |
|---|---|---|
| `GET` | `/health` | System health check & engine mode |
| `GET` | `/visualizer` | Interactive AntV X6 visualizer UI |
| `GET` | `/api/visualizer/domains` | List all loaded workflow domains |
| `POST` | `/api/v{version}/{workflow}/execute` | Synchronous request-reply execution |
| `POST` | `/api/v{version}/{workflow}/initiate` | Non-blocking async in-memory initiation |
| `GET` | `/api/v{version}/{workflow}/status/{job_id}` | Check job execution status |
| `GET` | `/api/v{version}/{workflow}/timeline/{job_id}` | Detailed trace timeline & payload data |
| `GET` | `/api/v{version}/{workflow}/stream/{job_id}` | Server-Sent Events stream (not in Java) |
| `POST` | `/api/v{version}/{workflow}/signal/{job_id}` | Resume a run paused at an `await_signal` step (Go & Rust) |
| `GET` | `/api/v{version}/{workflow}/jobs` | Recent jobs of the workflow |
| `GET` | `/api/v{version}/{workflow}/diagram/x6` | Graph node/edge metadata for AntV X6 (`?job_id=`, `?route=`, `?method=`) |
| `GET` | `/api/visualizer/jobs`, `/api/visualizer/resolve-job` | Jobs across workflows; the workflow of a job ID |
| `GET` | `/docs`, `/openapi.json` | Swagger UI and the OpenAPI document |

Routes declared in a blueprint's `api:` block are served next to these. In Go and Rust, `/visualizer`, the history routes, the diagrams and the API docs need the engine API key (local callers only when it has none); see `EngineOptions.Tooling` / `EngineOptions::tooling`.

---

## Security defaults

- **`ApiKeyAuthMiddleware`** accepts only the keys in its `valid_keys` (or `api_keys`); with no keys configured it refuses every request. There are no built-in keys.
- **`BearerAuthAndClaimsMiddleware`** verifies tokens with the validator you register in code, or as HS256 JWTs signed with `jwt_secret` from the blueprint or the `OCTOPUS_JWT_SECRET` environment variable (`exp` and `nbf` are checked). With none of these it refuses every request.
- An unknown middleware name in a blueprint is ignored, so check the spelling.

---

## Running Tests

- **Python**: `pytest octopus_live_engine/python/tests/`
- **Go**: `cd octopus_live_engine/go && go test -v ./...`
- **Rust**: `cd octopus_live_engine/rust && cargo test`
- **.NET**: `dotnet test octopus_live_engine/dotnet/Octopus.LiveEngine.Tests/`
- **Java**: `mvn clean test -f octopus_live_engine/java/pom.xml`


---

## License

MIT; see [LICENSE](LICENSE). Each published package carries its own copy.

The Python wheel `octopus-core` links Rust crates under their own permissive licences (MIT, Apache-2.0, BSD, ISC and others); their notices ship with it as `THIRD_PARTY_LICENSES.txt`. After changing the core's dependencies or features, regenerate that file with `core/scripts/third_party_licenses.py` (`--check` tells you whether it is up to date).
