# OctopusLiveWire

The .NET SDK of the OCTOPUS Live Engine. It runs YAML workflow blueprints behind
ASP.NET Core minimal APIs, with middlewares, sagas, retries, async steps and a
live visualizer.

```
dotnet add package OctopusLiveWire
```

The namespace is `Octopus.LiveEngine`.

## A route in a few lines

`hello.yaml`:

```yaml
workflow_name: hello
version: "1.0.0"
api:
  - route: /api/v1/hello
    method: POST
    steps:
      - id: step_greet
        activity: Greet
        is_output: true
```

`Program.cs`:

```csharp
using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;

var app = WebApplication.CreateBuilder(args).Build();
var engine = new OctopusLiveEngine();

engine.RegisterActivity("Greet", input =>
    new Dictionary<string, object> { ["greeting"] = $"hello {input["name"]}" });

engine.LoadBlueprintFile("hello.yaml");
app.MapLiveEndpoints(engine);
app.Run();
```

`POST /api/v1/hello` with `{"name": "ada"}` answers `{"greeting": "hello ada"}`.
The visualizer is at `/visualizer?domain=hello`; it and the other tooling routes
answer this machine only, unless `EngineOptions` says otherwise.

## Engines

The workflows run on the OCTOPUS core, a Rust library shared with the other
SDKs. The package carries it for linux-x64, linux-arm64, osx-x64, osx-arm64 and
win-x64. On any other platform the SDK runs them on its own .NET engine.

- `new OctopusLiveEngine()` (or `engine: "auto"`): the core when it loads, else .NET.
- `engine: "core"`: the core, or an error.
- `engine: "dotnet"`: the .NET engine.

`OCTOPUS_LIVE_ENGINE` sets the default; `engine.EngineName` says which one runs.

On `"auto"`, a custom state store (`SetStateStore`) or async dispatcher
(`SetAsyncDispatcher`) moves the instance to the .NET engine, as long as it
hasn't run a workflow or mounted its routes yet. On the core, configure the
store with a blueprint's `persistence:` block instead: in memory, SQLite,
PostgreSQL, MySQL, SQL Server or Redis.

`mode: hybrid` blueprints send their async steps to Temporal
(`persistence.temporal`). The core runs the worker when the routes are mounted,
or when you call `StartWorkerAsync`.

## Licences

MIT. The core's library includes Rust crates under their own licences; their
notices are in `THIRD_PARTY_LICENSES.txt` in the package.
