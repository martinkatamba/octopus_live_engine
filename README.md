# OCTOPUS Live Engine ⚡

> **Ultra-Low Latency, Pure In-Memory Polyglot Workflow Execution Engine** with Interactive AntV X6 Visualizer and Hexagonal Architecture.

`octopus_live_engine` is a standalone, lightweight, zero-infrastructure workflow orchestrator designed for real-time applications requiring sub-10ms response times (e.g. real-time fraud scoring, tax/pricing calculation, synchronous request-reply pipelines, and API gateways).

---

## Key Features

- ⚡ **Ultra-Low Latency Execution**: Zero external dependencies (No Temporal server, Redis, Kafka, or SQL database required). Executions take **< 1ms**.
- 📐 **Strict Hexagonal Architecture**: Clean separation between domain pipeline services, inbound ports (`ILiveWorkflowUseCase`, `IVisualizerUseCase`), and outbound adapters (`IActivityExecutor`, `IBlueprintRepository`, `IStateStore`).
- 🌐 **Interactive AntV X6 Visualizer**: Embedded real-time DAG graph at `/visualizer` and `/diagram/x6` with execution state drawers.
- 📡 **Server-Sent Events (SSE) Streaming**: Real-time event streaming per job at `GET /stream/{job_id}`.
- 🔄 **In-Memory Saga Rollbacks**: Automatic reverse compensation execution on step failure.
- 🛡️ **LRU Idempotency Guard**: In-memory replay prevention with TTL expiration.
- 🌍 **Polyglot SDKs**: Full parity across **Python**, **Go**, **.NET 8.0**, and **Java 17+**.

---

## Architecture (Ports & Adapters)

```
                       INBOUND ADAPTERS
       ┌──────────────────────────────────────────────┐
       │  FastAPI / Gin / ASP.NET / HttpServer Routes │
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

The [OCTOPUS CLI](../octopus_workflow_cli_go) scaffolds a complete live service in any of the four languages. Create it at the root of the OCTOPUS checkout so it finds this engine:

```bash
(cd octopus_workflow_cli_go && go install ./cmd/octopus)   # once
octopus init --name payments --lang go --engine live      # or --lang python | dotnet | java
```

| Language | Run it (port 8001) |
|---|---|
| Go | `cd payments && make run` |
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

- **Job IDs**: Python, .NET and Java use the `job_id` you send. Go generates its own unless the service enables `AllowClientJobIDs`, so use the `job_id` it returns.
- **Failed sync jobs**: Go, .NET and Java answer `/execute` with HTTP 500; Python answers 200 with the data collected so far (`?raw=true` shows `"status": "FAILED"` and the traces).
- **Blueprints**: the Go engine refuses `allowed_initiator_roles`; authorize callers with a middleware instead.
- **Visualizer**: the Go engine protects it with the API key, so the browser has to send `X-API-Key`.

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
| `GET` | `/api/v{version}/{workflow}/stream/{job_id}` | Real-time Server-Sent Events (SSE) stream |
| `GET` | `/api/v{version}/{workflow}/diagram/x6` | Graph node/edge metadata for AntV X6 |
| `GET` | `/docs` | OpenAPI / Swagger interactive documentation |

---

## Running Tests

- **Python**: `pytest octopus_live_engine/python/tests/`
- **Go**: `cd octopus_live_engine/go && go test -v ./...`
- **.NET**: `dotnet test octopus_live_engine/dotnet/Octopus.LiveEngine.Tests/`
- **Java**: `mvn clean test -f octopus_live_engine/java/pom.xml`
