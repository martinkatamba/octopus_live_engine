# OCTOPUS Live Engine (Python)

Runs OCTOPUS YAML workflow blueprints inside your process, behind FastAPI: no Temporal server, worker or queue. It suits request-reply APIs that answer from memory, such as fraud scoring, tax and pricing, or login flows.

- Steps with conditions, loops, jumps, retries, timeouts and saga compensation.
- Declarative middlewares: API key, HS256 JWT, IP allow-list, rate limit, payload validation, caching and audit.
- An interactive visualizer at `/visualizer`, and live events per job at `/stream/<job_id>`.
- Jobs in memory, or in Redis, Postgres, MySQL, SQL Server or SQLite.

## Install

```sh
pip install octopus-live-engine
```

This also installs `octopus-core`, the engine's native core, which the SDK runs on. It has wheels for macOS (x86_64, arm64), Linux (x86_64, aarch64; glibc 2.28+ or musl) and Windows (x64); on other platforms the SDK runs its own Python engine. `engine="python"` or `OCTOPUS_LIVE_ENGINE=python` picks the Python engine anywhere. Python 3.11 or later.

## Use

```python
from octopus_live import OctopusLiveEngine

engine = OctopusLiveEngine()

@engine.activity("CalculateTax")
def calculate_tax(payload):
    amount = payload.get("amount", 0.0)
    return {"tax": amount * 0.15, "total": amount * 1.15}

engine.load_blueprints_from_directory("./config")

# Run a workflow in the request...
result = await engine.execute_sync("order_tax_pipeline", {"amount": 200.0})
print(result.data)

# ...or serve every workflow over HTTP, with the visualizer.
engine.start_server(port=8001)
```

Every workflow is then served under `/api/v1.0.0/<workflow>`: `POST /execute` runs it in the request, and `POST /initiate` returns a job ID and runs it in the background.

The blueprint format, middlewares and visualizer are documented in the [OCTOPUS Live Engine repository](https://github.com/martinkatamba/octopus_live_engine).

## Licence

MIT; see `LICENSE`.
