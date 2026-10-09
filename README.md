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

## Quickstart

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

engine.RegisterActivity("CalculateTax", func(ctx context.Context, input map[string]interface{}) (map[string]interface{}, error) {
    amount := input["amount"].(float64)
    return map[string]interface{}{"tax": amount * 0.15, "total": amount * 1.15}, nil
})

state, err := engine.ExecuteSync(ctx, "order_tax_pipeline", map[string]interface{}{"amount": 200.0})
```

### .NET (C#)

```csharp
var engine = new OctopusLiveEngine();

engine.RegisterActivity("CalculateTax", input => {
    var amount = Convert.ToDouble(input["amount"]);
    return new Dictionary<string, object> { ["tax"] = amount * 0.15, ["total"] = amount * 1.15 };
});

var state = await engine.ExecuteSyncAsync("order_tax_pipeline", new Dictionary<string, object> { ["amount"] = 200.0 });
```

### Java

```java
LiveEngine engine = new LiveEngine();

engine.registerActivity("CalculateTax", input -> {
    double amount = ((Number) input.get("amount")).doubleValue();
    return Map.of("tax", amount * 0.15, "total", amount * 1.15);
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
