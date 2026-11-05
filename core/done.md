# Done

What is finished, newest first. The plan is in [plan.md](plan.md); open work is
in [to-be-done.md](to-be-done.md).

## 2026-11-04: The Linux wheel, built and tested locally

The manylinux wheel CI would build, built in Docker on this machine and
tested in clean containers. Nothing was uploaded.

- **Build:** `quay.io/pypa/manylinux_2_28_x86_64`, rustup stable
  (1.99.1), protoc 29.3, `maturin build --release --manylinux 2_28`, as
  in `.github/workflows/python-wheels.yml`. The result is
  `octopus_core-0.1.0-cp310-abi3-manylinux_2_28_x86_64.whl`: 12 MB, with a
  38 MB `_native.abi3.so`. maturin's manylinux check passed.
- **Memory:** compiling `temporalio-protos` with full optimisation and
  thin LTO needs more than 2 GB; rustc was killed at Docker's 2 GB limit,
  even with one build job. With Docker at 6 GB, the build took 30 minutes
  with two jobs (under load). GitHub's runners have enough.
- **Tests:** the wheel and the SDK installed into `python:3.X-slim`
  (glibc 2.41), with the SDK's tests run on the core and on the Python
  engine:

| Python | Core | Python engine |
|---|---|---|
| 3.11 | 85 pass, 3 skipped | 85 pass, 3 skipped |
| 3.12 | 85 pass, 3 skipped | 85 pass, 3 skipped |
| 3.13 | 85 pass, 3 skipped | 85 pass, 3 skipped |
| 3.10 | the wheel imports (the SDK needs 3.11+) | — |

The skipped tests need DuckDB, which the default wheel leaves out.

## 2026-11-01: A step's work runs before its branch (D21)

A step with an activity, a true condition and `then_steps` ran the branch
first and the activity after it on the Python engine, so the branch could
not see the activity's output. The core and Go run the activity first;
that is now the rule (D21, case 038), and the Python engine defers the
`then` branch until the step's activity or loop has run. A false condition
was already the same everywhere: the work is skipped (D1) and the `else`
branch runs.

**Results:** conformance 38/38 on Go, Rust, Python and the core; SDK tests
85 pass, 3 skipped, on each engine; binding tests 7/7; example tests as
before.

## 2026-11-01: Loop and condition traces decided (D19–D20)

The two trace differences that `test_a_large_order_returns_the_same_data_as_the_python_engine`
found are decided (`conformance/decisions.md`), with cases 036 and 037. The
core, Rust, Go and Python pass all 37 cases.

| # | Rule | Case | What changed |
|---|---|---|---|
| D19 | A loop's trace has `iterations`, `results` and `failures`, as lists; `continue_on_error` records each failed item and goes on | 036 | Go (empty lists were `null`), Python engine |
| D20 | A condition trace's input is every field the condition names; its output is `result`, `branch` and `skipped` | 037 | Core, Go, Python engine |

**D19, loop traces.**
- **Go** wrote `null` for a loop with no items or no failures; it writes
  empty lists now, as the core does.
- **The Python engine** had no `continue_on_error`: any failed item failed
  the step. It now records the failure and goes on, as the core and Go do,
  and a loop that fails leaves a `FAILED` trace with the item's input.

**D20, condition traces.**
- **The input** was empty in the core and Go, and in Python held only the
  fields evaluation reached (so `a or b` left out `b` when `a` held). All
  engines now record every field the condition names, by its path as
  written: `ConditionEvaluator::condition_fields` in the core and Go,
  `ConditionEvaluator.condition_fields` in Python.
- **When a trace is written:** the Python engine traced every condition
  with branches, and none for an activity step it skipped. It now follows
  the core: a trace when the condition decides a step with no work of its
  own, or skips one that has.
- **The output:** Python writes the core's `result`, `branch` and
  `skipped`, and the activity name `condition`. It keeps its own detail
  keys (`condition_matched`, `evaluated_rules`, ...) for its visualizer;
  the rule allows extra keys.
- **Masking:** a masked field shows as masked even when it is missing, as
  everywhere else (D5).

`test_a_large_order_returns_the_same_data_as_the_python_engine` no longer
skips condition traces or drops `failures`.

**Results:**

| Suite | Result |
|---|---|
| Conformance | 37/37 on Go, Rust, Python and the core |
| Core tests | 46/46 (clippy: the 15 known lints only) |
| Go tests | pass |
| Rust engine tests | 50/50 (`--no-default-features`) |
| SDK tests | 85 pass, 3 skipped (DuckDB) on each engine |
| Binding tests | 7/7, including the large-order comparison |
| Example tests, robot suites | as before (robot 28/28) |

## 2026-10-31: Engine differences decided (D14–D18)

The six differences phase 3 found between the core and the Python engine
are decided (`conformance/decisions.md`). Each has a conformance case, and
the core, Rust, Go and Python pass all 35 cases.

| # | Rule | Case | What changed |
|---|---|---|---|
| D14 | An `exceptions` entry also matches an error's base type | 031 | Core, Go, Python SDK |
| D15 | Live events carry the core's payloads | 032 | Python engine |
| D16 | New job IDs are `job_<uuid4>`, and opaque | 033 | Python engine |
| D17 | A declared `timeout_seconds` is enforced; there is no default | 034 | Python engine and SDK |
| D18 | With nothing named, the visualizer draws the first declared route | 035 | Core, Go; SDK workaround removed |
| D13 | Hybrid mode's after-the-run replication stays dropped | — | Confirmed, nothing to change |

**D14, exception base classes.**
- **The error form** a host sends grows a type chain:
  `[Type,Base,...:code] message`. The core reads the first name as the
  type and matches an entry against any of them
  (`OctopusMiddlewareError::type_chain`).
- **Python** sends the class's MRO (without `object`), from the SDK's
  `ActivityFailure` and from the binding's own `activity_error`.
- **Go** has no inheritance: `OctopusMiddlewareError` gains a `Cause`
  (with `Unwrap`), and the engine matches any `OctopusMiddlewareError` name
  in the error's `Unwrap` chain.

**D15, live events** (the Python engine moves to the core's and Go's
events):
- `workflow_*` events carry the job's masked state, not a small dict.
- `step_started` (`step_id`, `activity`, `index`, `total_steps`) and
  `step_completed` (the step's trace) replace `step_progress`.

**D16, job IDs:** the Python engine's `live-<12 hex>` and
`live-async-<id>` become `job_<uuid4>`, as in the core and Go.

**D17, timeouts.**
- **The Python engine enforces it now:** a coroutine is cancelled; a plain
  function runs in a worker thread while a timeout is declared, so the
  wait can end (the thread runs on).
- **No hidden default:** `WorkflowStep.timeout_seconds` defaulted to 30,
  so the SDK dropped every 30 before handing a blueprint to the core,
  including one the user declared. The default is now 0 (no limit), and a
  declared value always reaches the core.

**D18, visualizer:** the core and Go draw the first declared route that
has steps when no job, route or method is named. The SDK's workaround for
this is removed.

**Conformance suite:**
- **Runner contract**, for all four runners:
  - an activity can fail with a typed error (`error_type`, `error_bases`);
  - a run can record its live events (`"events": true`);
  - a run can ask for the visualizer graph (`"diagram": true`).
- **New expectations:** `fields_match`, `events`, `diagram_nodes` and
  `diagram_nodes_absent`.

**Results:**

| Suite | Result |
|---|---|
| Conformance | 35/35 on Go, Rust, Python and the core |
| Core tests | 46/46 |
| Go tests | pass |
| SDK tests | 85 pass, 3 skipped (DuckDB) on each engine; 12 new (each decision on both engines, the timeout for a function and a coroutine) |
| Example tests, robot suites, outer examples | as before (robot 28/28) |

## 2026-10-30: No more fallbacks to Python

Everything that used to switch `octopus_live` to the Python engine now runs
on the core. The SDK falls back only when the installed `octopus_core` was
built without the store asked for (`octopus_core.STORES` lists them). The
default wheel leaves DuckDB out (below), so `store: duckdb` is that case.

**TLS for Postgres and MySQL** (`adapters/outbound/db_tls.rs`). Rustls on
`ring`, as SQL Server; no aws-lc in the tree.
- **Postgres** reads libpq's settings, in URL or key=value form:
  - `sslmode`: `disable`; `allow` and `prefer` (the default) use TLS when
    offered, certificate unchecked; `require` (checked as `verify-ca` when
    `sslrootcert` is given); `verify-ca`; `verify-full`;
  - `sslrootcert` (a file, or `system`), else `~/.postgresql/root.crt`,
    else the webpki roots;
  - `sslcert` and `sslkey` for a client certificate.
- **MySQL** reads the mysql client's settings:
  - `ssl-mode`: `DISABLED`; `PREFERRED` connects plain (see to-be-done.md);
    `REQUIRED` (checked as `VERIFY_CA` when `ssl-ca` is given);
    `VERIFY_CA`; `VERIFY_IDENTITY`;
  - also `require_ssl=true` and Go's `tls=true|skip-verify|false`;
  - `ssl-ca`, `ssl-cert`, `ssl-key`.
- **Tested** with a real handshake for every mode, against small fake
  servers that answer whether the connection came in over TLS. The
  certificates come from rcgen; no Docker needed.
- **Python's MySQL store ignores TLS settings:** pymysql is given only the
  host, user and database. On the core they now apply.
- **Postgres errors were unreadable:** every Postgres error read just
  "db error", because tokio-postgres's `to_string` leaves out the server's
  message. They now carry the severity, message and SQLSTATE.

**DuckDB store** (`adapters/outbound/duckdb_store.rs`, feature `duckdb`).
- **Same tables as Python's `DuckDbStateStore`**, so either engine reads
  the other's file. Tested both ways and core-to-core, each writer in its
  own process: a DuckDB file belongs to one process at a time.
- **The core's extra fields** (`client_host`, `duration_ms`, `error_code`,
  `error_type`) go in a side table, `execution_state_details`, that Python
  never reads.
- **Configured as** `store: duckdb` with `path`/`duckdb_path`/`db_path`,
  else `DUCKDB_PATH`, else `data/octopus_traces.duckdb`, as Python.
- **Version pinned** to `~1.10505.0` (DuckDB 1.5.5, as the venv's
  `duckdb`). The crate's version encodes DuckDB's; 1.20000 is DuckDB 2.0,
  whose files 1.5 can't open.
- **Cost:** DuckDB is C++, built from source (`bundled`).
  - **Build time:** a cold build took 74 min (debug) and 70 min (release)
    at load averages of 45–235; later builds reuse it.
  - **Size:** the module grew from 30.4 MB to 72.6 MB (58 MB stripped,
    about 21.6 MB compressed in a wheel).
- **Opt-in, not in the default wheel** (decided 2026-10-31, for the size
  and build time). Neither `octopus-core` nor the Python binding builds it
  by default; `FEATURES=duckdb ./build.sh` (or
  `maturin build --features duckdb`) adds it. Without it, `store: duckdb`
  runs on the Python engine with a warning, as before.

**A Python `IStateStore` object.** The core calls the object for every
save and read (`Engine.set_python_state_store`, `_StoreBridge` in
`core_backend.py`), so the server and the app see the same jobs.
- **Coroutines** run on the server's loop when the core calls from another
  thread; on the loop's own thread, on a private loop in a worker thread.
- **Off the loop:** `initiate_async` and the visualizer's diagram call now
  run in an executor, as they may wait on the store.
- **Events** still go through the core's relay, so `stream_events` works
  whatever the store.

**Customised built-in middlewares** (D11's open part). The core keeps the
built-in's check and calls back into Python only for the part the user
customised (`MiddlewareHooks`, `build_middleware_with_hooks`):

| Middleware | Hooks |
|---|---|
| Bearer | `token_validator`, `claims_mapper` |
| Roles | `role_extractor` |
| Rate limit | `rate_limit_key`, `rate_limit` |
| IP allow-list | `ip_validator`, `ip_allowed` |
| Cache | `cache_key`, `cache_bypass` |

- **What counts as customised:** a callable given in code (validator,
  mapper, key extractor, IP validator), or a subclass that overrides the
  method behind a hook (`_default_validator`, `_resolve_key`,
  `is_ip_allowed`, ...).
- **Errors:** an exception in a hook is re-raised after the core's
  decision, as the same Python exception, so `exceptions:` handlers match
  as before.
- **The request:** hooks take no request argument; the SDK hands them the
  Starlette request through a context variable. Async IP hooks work.
- **Still Python:** a subclass that overrides `process` replaces the
  built-in with its own code, so it runs as any custom middleware does.

**Results** (default = core; `OCTOPUS_LIVE_ENGINE=python`):

| Suite | Result |
|---|---|
| Core tests | 50/50 with `duckdb`, 46/46 without |
| SDK tests | 76/76 on each engine |
| Example tests | core 19 pass, 7 fail; Python 18 pass, 8 fail (as before) |
| Robot suites | 28/28 |
| `checkout-tax-calculator` and `realtime-fraud-preauth` flows | pass, on the core |
| Clippy | the 15 older lints only |
| `octopus-ffi`, `../rust` | build (rust: `cargo check`) |

TLS was not tried against a real server: the Docker databases have no TLS
set up, and were unhealthy under load.

## 2026-10-30: The core is `octopus_live`'s default engine

`OctopusLiveEngine` runs on the core whenever `octopus_core` is installed,
with no setting. Examples and apps need no change.

**Choosing the engine:**
- **Default (`engine="auto"`):** the core if `octopus_core` imports, else
  Python with an INFO log.
- **`engine="core"`:** as before. If the core can't be used, falls back to
  Python with a WARNING.
- **`engine="python"` or `OCTOPUS_LIVE_ENGINE=python`:** the Python engine.
- **Configurations the core can't serve** (a Python store object, DuckDB)
  still run on Python.
- **Not a hard dependency:** `octopus-core` stays an optional extra until
  wheels are on PyPI.

**Every store in the core.** The core is meant to own Octopus's work,
database drivers included; an SDK handles HTTP and passes connection
settings.
- **Binding:** now built with the core's Postgres, MySQL and SQL Server
  stores, as well as SQLite and Redis. Before this, an app configured for
  Postgres or MySQL would have run on the core with jobs kept only in
  memory.
- **MySQL driver:** built without TLS (`minimal-rust`). Its rustls feature
  brought aws-lc, a C library, into every build.
- **TLS-only DSNs:** stay on the Python engine until the core's Postgres and
  MySQL drivers have TLS.
- **Size:** the module is 30.4 MB, from 26.6 MB.
- **Tested against the project's Docker databases:** through
  `octopus_live`'s default engine, a job was written to Postgres 16 and
  to MySQL 8.0, then read back with its trace by a second engine, and its
  row checked in each database. The test rows were deleted afterwards.
- **SQL Server not tested:** its container was crash-looping on this
  machine.

**Comparisons pin the Python engine.** With the core as the default,
everything that compares against "the Python engine" now asks for it by
name:
- the conformance suite's Python runner (otherwise its 30/30 would test the
  core);
- the binding's parity tests and the example-server loader;
- the benchmark's baseline.

**Now the default behaviour:** the differences listed in to-be-done.md,
notably that `X-Forwarded-For` counts only from `trusted_proxies` (D11).

**SQLite:** the two engines read each other's SQLite jobs, so the "schema
difference" listed in to-be-done.md was wrong and is removed. It came from
a Python engine bug:
- **The bug:** on Python 3.12+, its relational store never committed.
  `sqlite3.Connection.autocommit` is truthy there by default (-1, the
  legacy mode), and the store read that as "autocommit on". Jobs stayed
  in an open transaction and were lost when the process exited.
- **Fixed:** commit unless `autocommit is True`.
- **Tested:** both directions; the Python-to-core test fails without the fix.

**Results** (default = core; `OCTOPUS_LIVE_ENGINE=python`):

| Suite | Default (core) | Python |
|---|---:|---:|
| SDK tests | 69/69 | 69/69 |
| Example tests | 19 pass, 7 fail | 18 pass, 8 fail |
| Robot suites | 28/28 | |
| `checkout-tax-calculator` and `realtime-fraud-preauth` flows | pass | |

The example failures are the same as before (a server expected to be
running; `test_zero_import_store_factory`).

## 2026-10-29: Phase 3, step 2: middlewares, visualizer and hybrid mode on the core; wheels

On the core backend, `octopus_live` now hands three more things to the core,
as decided in D11–D13 (`conformance/decisions.md`). The Python classes stay
as the public API.

**Middlewares (D11).** Each built-in Python middleware asks the core to make
its check (`Engine.build_middleware`), then applies the result: headers,
context, or the refusal as the same Python exception.
- **Covered:** Bearer, API key, roles, correlation ID, rate limit, IP
  allow-list, audit, cache, payload validation.
- **Added to the core**, from Python:
  - `RoleAuthorizationMiddleware`: roles from the claims, else the
    `X-User-Roles` header;
  - `require_json_body`;
  - a 10 MB default body limit;
  - `query:<name>` cache keys;
  - `username` and `tenant_id` claims as rate-limit keys.
- **Client IP:** one rule, `octopus_core::net::client_ip`, used by the core
  and the Rust host. `X-Forwarded-For` / `X-Real-IP` count only from
  `trusted_proxies` (a new `OctopusLiveEngine` argument). This is a
  behaviour change: on Python, any client could set its own IP.
- **Stays in Python:** a middleware given its own token validator or claims
  mapper, or a subclass that overrides `process` or the part the core does.

**Visualizer (D12).** On the core, the diagram JSON comes from the core's
`compile_diagram`, and `/visualizer` serves the core's page.
- The core's step nodes gain `is_output`.
- With no route and no job named, the SDK asks for the first declared route,
  as Python showed. The core alone (and Go) would show the top-level steps;
  this difference is listed in to-be-done.md.

**Hybrid mode (D13).**
- **Temporal:** async steps go to Temporal through the core's adapter, using
  the blueprint's `persistence.temporal` or the engine's
  `temporal_host`/`namespace`/`task_queue`.
- **Fallback:** when Temporal is unreachable, async steps run in-process and
  `start_embedded_worker` logs a warning.
- **Store:** jobs live in the core's store.
- **Dropped:** Python's after-the-run replication and its second durable
  store.
- **Constructor stores:** a store set in the constructor (`sqlite_path=`,
  `redis_url=`, `state_store="sqlite"`) now runs on the core
  (`Engine.set_state_store`) instead of falling back to Python.
- **Build:** the binding is built with `sqlite`, `redis` and `temporal` by
  default. `build.sh` finds the vendored `protoc`.

**Wheels.**
- **Packaging:** `pyproject.toml` builds abi3 wheels (`abi3-py310`): one wheel
  per platform for Python 3.10 and later. Git-ignored files and
  `__pycache__` are left out.
- **CI:** `.github/workflows/python-wheels.yml` builds manylinux and musllinux
  (x64 and arm64), macOS x64 and arm64, and Windows x64. It then installs
  each Linux and macOS wheel with the SDK and runs the SDK tests on both
  engines. **It has not run yet.**
- **Local wheel:** a macOS x64 wheel was built with maturin 1.16 (24 min on
  this machine) and installed with the SDK in a fresh 3.11 venv, without
  the dev `.pth`. The SDK tests pass 64/64 on both engines. That wheel was
  built before the `is_output` change.
- **abi3 cost:** against the per-interpreter build on 3.12, 5 interleaved
  rounds, median µs per run: 20 noop steps 303 vs 305; 100 noop steps
  1,474 vs 1,426; 5 steps echoing 100 items 2,235 vs 2,348. Step cost is
  the same. The payload-heavy case is about 5% slower, within this
  machine's noise.

**Results, on Python and on the core:**

| Suite | Python | Core |
|---|---:|---:|
| SDK tests (64, 11 of them in `test_core_backend.py`) | 64/64 | 64/64 |
| Example tests (`examples/python`) | 18 pass, 8 fail | 19 pass, 7 fail |
| Robot suites | 23/28 | **28/28** |
| `checkout-tax-calculator` and `realtime-fraud-preauth` flows | pass | pass |
| Conformance `--strict` | 30/30 (go, rust, python, core) | |

- **Core passes more:** `hybrid_payment_api`'s durable-store test and the
  5 hybrid visualizer robot tests pass on the core only. The robot tests
  failed on Python because its visualizer never showed a hybrid job as
  `COMPLETED`.
- **Still failing on both:** `cross_border_payment_clearing` expects a
  server already running; `test_zero_import_store_factory` expects a
  `SqliteStateStore`.
- **Rust and binding checks:**
  - core 39 tests and Rust host 51/50/53 on each feature set;
  - binding 7;
  - clippy at the same 15 old lints;
  - fmt clean;
  - `check-ffi.sh` passes.

## 2026-10-29: Phase 3, step 1: `octopus_live` on the core

`octopus_live` runs on the core when asked:
`OctopusLiveEngine(engine="core")` or `OCTOPUS_LIVE_ENGINE=core`. The default
is still the Python engine.

**What runs where on the core backend:**
- **The core** runs the steps, keeps the jobs and publishes live events.
- **Still in Python, unchanged:** the FastAPI router, Pydantic models,
  middlewares, visualizer, and hybrid mode's Temporal replication and
  durable store.

**SDK (`python/src/octopus_live/core_backend.py`):**
- **`CorePipelineManager`** replaces `PipelineManager` with the same
  `execute()`. It returns failures as states, never raises. In hybrid mode it
  hands the finished state to the replication manager, as before.
- **`CoreStateStore`** is the `IStateStore` over the core's jobs:
  - `save_state` covers the jobs middlewares record;
  - `subscribe` streams live events until the final event.
- **Blueprints:**
  - **Conversion:** the dataclasses are sent as the YAML-shaped dict the core
    parses (`blueprint_to_raw`). Multi-rule conditions become the step's
    `conditions` group. The dataclass default `timeout_seconds: 30`, which
    Python never enforces, is left out.
  - **Roles:** `allowed_initiator_roles` is left out because the router
    enforces it.
  - **Re-registration:** a blueprint is registered again when the repository
    holds a new object, or after `set_global_exception_handler`.
- **Activities** stay in the engine's `activity_executor`, and the core calls
  back into it for each one:
  - plain functions run on the core's thread;
  - coroutines run on the caller's event loop;
  - the snake_case alias and `{"result": ...}` wrapping are unchanged.
- **Errors:** an activity's exception reaches the core as `[Type:code]
  message` (`ActivityFailure`). The error text, type, code and trace message
  then match the Python engine's, for plain, coded and middleware
  exceptions.
- **Fallback:** a configuration the core can't serve yet stays on Python,
  with a warning. These are a Python store object, DuckDB, or a store set
  in the constructor.
- **Packaging:** `pyproject.toml` has a `core` extra. For development, both
  venvs import `octopus_core` through a `.pth` file.

**Core:**
- `ActivityResolver`: the executor asks the host for any activity that has
  no handler of its own. Exposed as `Engine::set_activity_resolver`.
- `Engine::save_state`.

**Binding:**
- `register_blueprint` takes a dict.
- `set_activity_resolver(call, has)`.
- `execute(..., endpoint_index, route, method)` runs a declared route's
  steps.
- `save_state`.
- `subscribe(job_id)` returns a `Subscription` with `next(timeout)` and
  `close()`.
- A run's state includes `route_output` (D5).
- Activity errors keep a pre-formatted message, and carry an exception's
  `code`/`status_code`.
- `build.sh` copies, then renames, so a running process keeps its loaded
  library.
- Built for Python 3.12 (the SDK venv) and 3.10 (the project venv, which the
  robot suites use).

**Results, on Python and on the core:**

| Suite | Python | Core |
|---|---:|---:|
| SDK tests (53, plus 7 new in `test_core_backend.py`) | 60/60 | 60/60 |
| Example tests (`examples/python`) | 18 pass, 8 fail | the same |
| Robot suites | 23/28 | 23/28, the same tests |
| `checkout-tax-calculator` and `realtime-fraud-preauth` flows | pass | pass |

The failures predate this work and are listed in to-be-done.md (Temporal,
servers expected to be running, the hybrid visualizer).

**New tests:** `tests/test_core_backend.py` compares failure reports with
the Python engine, and checks async activities on the caller's loop, live
events, blueprint conversion, and the fallback. The core has 1 new test,
for the resolver.

## 2026-10-29: Converting payloads at the PyO3 boundary

**Profile:** a 100-item order through PyO3 spent its time like this:

| Share | What |
|---:|---|
| ~42% | converting the returned state, whose traces share most of their data in Rust |
| ~25% | converting each activity's input |
| ~16% | converting each activity's result |

**Changes in `bindings/python`:**
- **Shared maps:** `ToPy` converts each shared map once per call, keyed by
  `ValueMap::as_ptr`, and reuses the dict wherever the map appears. Maps
  nothing else holds are converted directly. A returned state is therefore
  read-only in spirit: changing one place may change another.
- **Exact types first:** `from_py` checks for the exact built-in types
  before the subclass and `model_dump` fallbacks.
- **Tried and dropped:** interned key strings, which were slower (45–50 ms
  against 40–44 ms).

**Changes in `octopus-core`:** `ValueMap::is_shared` and `ValueMap::as_ptr`
were added.

**New test:** `test_a_large_order_returns_the_same_data_as_the_python_engine`
runs a 30-item order on both engines and compares the data and every
activity trace. It turned up two older differences between the engines,
logged in to-be-done.md.

**Results** (median per run, load 14 → 6):

| Order | Before | After | Python engine |
|---|---:|---:|---:|
| 10 items | 1,540 µs | 1,184 µs | 10,184 µs |
| 50 items | 15.8 ms | 11.2 ms | 59.1 ms |
| 100 items | 73 ms | 42 ms | 154 ms |

**Verified:**
- the Python binding: 7 tests pass, clippy and fmt are clean;
- core: 32 tests pass.

## 2026-10-29: Go decision, and the core's per-step cost

**Decision: go.** The condition was that the core's per-step cost must stop
growing with the job's size. That cost is now fixed.

**What was wrong:** the core deep-copied the whole job several times per run:
- at the start;
- in each trace's input;
- in the final save;
- in the event JSON;
- in the returned state;
- in the idempotency copy.

In `order_fulfillment_api` each loop item returns the growing
`reserved_items` list, so the loop's results add up to n² entries. A
pure-Rust run of 100 items took 508 ms.

**Changes in `octopus-core`:**
- **`ValueMap` is copy-on-write:** a newtype over
  `Arc<BTreeMap<String, Value>>`. Reads and writes keep the `BTreeMap` API
  through `Deref`/`DerefMut`, and `Arc::make_mut` copies a shared map's top
  level on the first write. A clone is one pointer copy, so traces,
  snapshots and stored states share the job's data.
- **Masking:** `mask_payload` shares every map and array that has nothing to
  mask, and returns the input itself when there are no mask fields.
- **Blueprints:** `IBlueprintRepository::get_blueprint` returns
  `Arc<WorkflowBlueprint>`, so a run no longer copies its blueprint. A
  blueprint caches its `all_mask_fields`; a clone starts with an empty cache.
- **Events:** `IStateStore::has_listeners` was added. The default is true;
  the in-memory store checks for subscribers, and Redis says true while
  connected. `PipelineManager::emit` builds an event only when someone can
  receive it.
- **Threads:** `workers.rs` is a cached thread pool. A task goes to an idle
  thread, or starts one, so it never waits behind a blocked activity. Idle
  threads exit after 60 s. Async steps, timeouts, async compensations,
  `initiate_async` and async middleware activities use it. Signal timeouts
  keep their own thread.
- **Job IDs:** `/dev/urandom` is opened once per process behind a buffer. It
  is reopened when the process ID changes, so forked workers never share
  IDs.
- **Benchmark:** `examples/bench_order.rs` runs the order workflow with Rust
  activities.
- **Tests:** 4 new ones: copy-on-write, masking that shares, and the worker
  pool reusing threads without making tasks wait.

**`../rust`:** one test edits a fetched blueprint, so it now copies it first.

**Pure-Rust order, median per run** (`bench_order`):

| Items | Before | After |
|---:|---:|---:|
| 1 | 1,449 µs | 201 µs |
| 10 | 9,798 µs | 288 µs |
| 50 | 132 ms | 836 µs |
| 100 | 508 ms | 1.85 ms |

**Through Python:** the core is faster than the Python engine on every
workload. Full numbers are in phase-2-results.md.

**Verified:**
- core: 32 tests, with default, no-default and temporal features;
- `../rust`: 51 / 50 / 53;
- Rust examples, the C smoke test and the 6 Python binding tests pass;
- conformance: 30/30 for each engine, and `--strict` exits 0;
- fmt is clean, and clippy shows the same 15 older lints.

## 2026-10-29: Phase 2, the FFI layer and the Python spike

Results, numbers and the recommendation (go, with one condition) are in
[phase-2-results.md](phase-2-results.md).

**C ABI** (`crates/octopus-ffi`):
- **Lifecycle and errors:** `octopus_engine_new`/`_free`, status codes,
  `octopus_last_error` (per thread, JSON `{code, type, message}`), and
  `catch_unwind` on every export.
- **Blueprints and activities:**
  - blueprints as YAML or JSON text;
  - activities as C callbacks that write their result with
    `octopus_out_write`. They may run on any core thread.
- **Jobs:** execute, async start, signal, get and list.
- **Declared routes:** list them, and call one through `octopus_call_route`,
  which returns `{status, headers, body}`.
- **Middleware:** the chain, `before` and `after`.
- **Access:** the tooling decision and the API key check.
- **Live events:** subscribe and unsubscribe.
- **Visualizer and worker:** the diagram, the visualizer page, and
  starting/stopping the worker.
- **Header:** `include/octopus.h`, generated by cbindgen (`cbindgen.toml`).
- **Checks:** `scripts/check-ffi.sh` verifies the header, then builds and runs
  `tests/c/smoke.c` against the library.
- **Tests:** 6 Rust tests call the exports the way a C host does.

**Route rules in the core** (`domain/services/routes.rs`):
- `choose_job_id`, `apply_injections` and `public_data`;
- `failure_body`, which hides unexpected errors unless the options expose
  them;
- `DeclaredRoute` and `run_declared_route`.

They were private helpers in `../rust`'s `rest_handler.rs`. `../rust` now
calls the core's versions, and `Engine::call_route` exposes them to the other
hosts. `EngineOptions::from_value` reads options from JSON. 3 unit tests.

**Python binding** (`bindings/python`, PyO3 0.29.2):
- **Build:**
  - its own Cargo workspace, sharing `../../target`;
  - built by `./build.sh [--release]` for `../python/.venv` (Python 3.12);
  - `pyproject.toml` is ready for maturin wheels, but not yet tried.
- **Package `octopus_core`:**
  - `Engine` exposes the whole API, and `from_yaml` matches `octopus_live`'s
    use of `activity_instance`;
  - values convert directly, with no JSON text in between;
  - runs release the GIL, and activities take it back;
  - a Python exception becomes `"Type: message"` on the step.
- **`octopus_core.fastapi.create_app`** serves declared routes and
  `/diagram/x6`.
- **Tests:** 6 pass, with `order_fulfillment_api` on the core:
  - the example's 3 checks;
  - the same answers as the pure-Python engine;
  - the async step runs.

  The core used the blueprint's SQLite store.
- **`bench/bench.py`** compares python, core through PyO3, and core through
  the C ABI (ctypes). The raw results are in `bench/results.json`.

**Tools:** cbindgen 0.29.4 was installed with `cargo install`. maturin was
not installed.

**Results:**
- The core passes 28 tests with default features, no default features and
  `temporal`.
- `scripts/check-ffi.sh` passes.
- The binding's 6 Python tests pass, and the binding has no clippy warnings.
- `../rust` passes 51 / 50 / 53, and the Rust examples pass.
- Conformance: Go, Rust, Python and the core all pass 30/30;
  `run.sh --strict` passes.
- fmt is clean, and clippy shows the same 15 older lints.

## 2026-10-29: Middleware decisions and the engine wrapper in the core

Phase 1's two leftovers. `../rust` now does only HTTP: it reads requests,
applies the core's decisions, and serves routes.

**Middleware** (`domain/services/middleware/` in the core):
- `RequestMeta`: what a host knows about a request:
  - method and path;
  - headers and query;
  - the client IP, after its trusted proxies;
  - the body;
  - the context values earlier middleware set.
- `Decision`: what the host does next:
  - response headers to set;
  - context values to store;
  - optionally, a `Response` that ends the chain.
- `Middleware::before(req) -> Decision`, and `after(req, resp)` for work done
  once the response exists (the audit and the cache store).
- All 8 built-ins moved, with the same names, defaults and config keys:
  - bearer/JWT;
  - API key;
  - correlation ID;
  - rate limit;
  - IP allow/block lists;
  - payload validation;
  - response audit;
  - Redis cache.

  `build_middleware(cfg, ...)` is the factory.
- `ErrorResponder` turns a middleware error into its response. It applies the
  blueprint's `exceptions:` overrides and runs their activities, sync or
  async. This was `respond_middleware_error`.
- `IpNet` moved to a new `net` module.
- `../rust/src/domain/services/middleware.rs` went from 1,506 lines to about
  110:
  - `into_handler`, which turns a core middleware into a handler;
  - the custom middleware registry;
  - `respond_middleware_error` and `build_middleware_from_config`, kept as
    wrappers.

**Engine wrapper** (`octopus_core::Engine`, in `engine.rs`):
- Wires the blueprint repository, activity executor, state store, async
  dispatcher, pipeline and visualizer.
- Sets up persistence and Temporal from a blueprint's `persistence:`.
- Blueprints: `register_blueprint`, `register_blueprint_yaml(text)`, and
  loading from a file or a directory.
- Activities: `register_activity`.
- Jobs: execute, signal, read, list, `subscribe_events`.
- `compile_diagram`.
- The embedded Temporal worker.
- `tooling_decision(has_authorizer, loopback)`.
- `api_key_allows(header, query)`, with the same rule as the routes'
  `check_auth`.
- `middleware_chain()`:
  - one link per middleware name, in blueprint order;
  - built once and kept, so rate-limit counters and caches last as long as
    the engine;
  - registering a blueprint rebuilds it.
- `../rust`'s `OctopusLiveEngine` is now the core `Engine` plus routes, OpenAPI
  models, host middleware and the tooling authorizer. It derefs to `Engine`,
  so tests and examples didn't change.
- The core conformance runner uses `Engine` instead of its own wiring.
- `plan.md`'s FFI contract gains `octopus_middleware_chain`, `_before` and
  `_after`.

**Changed behaviour (small):**
- If one middleware name appears twice and the first entry doesn't build, the
  second is no longer tried; the first entry for a name always wins.
- `get_router()` and `mount_routes()` on the same engine now share one set of
  middleware instances, so their counters and caches are shared too.

**Results:**
- The core passes 20 tests with default features, no default features and
  `temporal`. 12 are new:
  - 9 middleware decision tests;
  - 3 engine tests.
- `../rust` passes 51 / 50 / 53, as before. The Rust examples pass.
- Conformance: Go, Rust, Python and the core all pass 30/30;
  `run.sh --strict` passes.
- fmt is clean. Clippy shows the same 15 older lints, now all in the core;
  the 3 from `../rust` moved with the engine methods.

## 2026-10-28: Phase 1, the core extracted from the Rust engine

`crates/octopus-core` now holds the engine. `../rust` is an HTTP host over it,
and all four runners pass 30/30.

**Moved from `../rust/src` into `crates/octopus-core/src`:**
- `domain/`: models, ports, and every service except the HTTP middleware;
- `adapters/outbound/`: the stores, dispatchers, `sql_backend`, `redis_kv`,
  the Temporal adapter and the activity executor;
- `crypto.rs`, `json.rs` (with `vmap!`) and `yaml.rs`;
- `visualizer_template.html`, as `visualizer_html::render_visualizer_html`.

The Cargo features (`redis`, `postgres`, `mysql`, `sqlite`, `mssql`,
`temporal`) and their dependencies moved with them; `../rust`'s features
switch the core's on. The core's `Cargo.lock` started from `../rust`'s, so
it uses the same dependency versions.

**New in the core:**
- `url::url_decode`, which the SQL backend needs. It came out of `http.rs`.
- `domain::services::tooling::decide_tooling_access`. It returns `Allow`,
  `NotFound`, `AskAuthorizer`, `RequireApiKey` or `Deny(reason)`; the host
  works out whether the client is loopback and enforces the answer.
  `../rust`'s tooling middleware now does exactly that.
- `domain::services::compile_diagram(blueprint, job_id, state, route, method)`:
  the visualizer compiler as a pure function. `VisualizerService` only looks
  up the blueprint and the job, then calls it.
- `domain::services::middleware` keeps the middleware errors, `RouteMatcher`
  and `glob_match`. (The implementations followed on 2026-10-29.)

**`../rust` keeps:**
- `http.rs`, `engine.rs` and `adapters/inbound/rest_handler.rs`;
- the HTTP half of `domain/services/middleware.rs`.

It re-exports the core at the old paths (`domain`, `adapters::outbound`,
`json`, `yaml`, `crypto`, `vmap!`, `render_visualizer_html`), so its tests and
examples didn't change.

**Conformance:** a new `conformance/runners/core` runs the cases on
`octopus-core` directly, with no HTTP host. `run.sh` runs it as a fourth
engine.

**Results:**
- Conformance: Go, Rust, Python and the core all pass 30/30;
  `run.sh --strict` passes.
- The core passes 8 tests with default features, `--no-default-features` and
  `temporal`: 5 condition-evaluator tests that moved, a tooling-policy test,
  a `compile_diagram` test, and the FFI test.
- `../rust` passes 51 / 50 / 53 tests (default, no default features,
  temporal). That's 5 fewer than before, the 5 evaluator tests that moved to
  the core.
- The Rust examples all pass.
- fmt is clean in both crates. Clippy shows only the 15 older lints: 12 in
  the core and 3 in `../rust`.

The core's `temporal` build needs `protoc`. There's none on the PATH here,
so the runs used `PROTOC=~/.cargo/registry/src/index.crates.io-*/protoc-bin-vendored-macos-x86_64-3.3.0/bin/protoc`.

## 2026-10-28: D10 decided; all three engines pass 30/30

**Decided:** when a step fails, only the steps that completed before it are
compensated. The failed step's own compensation doesn't run (case 030).

**Fixed:** Python ran the failed step's compensation too, because it records
the compensation in the trace before the activity runs.
`_pending_compensations` now keeps only `COMPLETED` traces. Go and Rust
already conformed.

**Results:**
- Conformance: Go, Rust and Python 30/30, with no cases needing a
  decision. `run.sh --strict` passes.
- Python: 53 engine tests pass, and the four example suites pass.

## 2026-10-28: Python engine conforms to D1–D9 (29/29)

Changes in `../python/src/octopus_live`:

- **`domain/services/condition_evaluator.py`** was rewritten as a port of
  the Rust and Go evaluators:
  - the shared operator list, numeric strings and indexed paths (D3);
  - the `rule:` parser (D2);
  - `validate_blueprint_conditions`.

  `evaluate_detailed` keeps the shape that the condition trace and the
  visualizer use. Booleans no longer equal numbers (`True == 1` in Python).
- **`adapters/outbound/yaml_repository.py`:**
  - `save` refuses an unparsable rule (D2);
  - steps accept step-level `then:`/`else:` (D3);
  - `pivot:` is read (D9).
- **`domain/models/models.py`:**
  - `WorkflowStep.pivot`;
  - `StepExecutionTrace.attempts`;
  - `ExecutionState.route_output`;
  - `WorkflowBlueprint.all_mask_fields()`.
- **`domain/services/pipeline_manager.py`:**
  - `retry_policy` is applied: `max_attempts`, `initial_interval_ms`,
    `backoff_coefficient` and `maximum_interval_ms`, as in Go. 4xx errors
    are not retried.
  - Every save, event and replication uses a masked snapshot, and execute
    returns one with the real `route_output` for declared routes (D5).
    Previously the in-memory store held the live, unmasked state.
  - The same `job_id` replays the first completed run (D6).
  - Async steps record `ASYNC_COMPLETED` / `ASYNC_FAILED`, applied when the
    snapshot is taken, so the final save can't overwrite them (D7).
  - `exceptions` match by type name, a base class's name, or a
    case-insensitive message substring, and the failing step's entries are
    checked first. A step-level handler leaves a trace, so it runs once,
    not twice (D8).
  - A pivot clears earlier compensations, with a `<step>.pivot` trace as
    in Go (D9).
  - Background tasks are kept referenced until they finish.
- **`adapters/inbound/rest_router.py`:** declared routes respond from
  `route_output`. `/execute` now returns masked data.
- **Tests:**
  - `tests/test_condition_evaluator.py`: the five evaluator tests, ported;
  - `test_declared_route_returns_masked_fields_to_the_caller`.
- **The conformance runner** leaves `route_output` out of the dumped state,
  as in the Go and Rust engines, which never serialize it.

**Results:**
- Conformance: Go, Rust and Python 29/29; `run.sh --strict` passes.
- Engine tests: 53 passed, up from 26.
- Examples: `auth_portal_api` 9, `checkout_tax_api` 4,
  `order_fulfillment_api` 3 and `todo_api` 1, all passing.
- Two examples fail for reasons unrelated to the engine, with or without
  this change:
  - `cross_border_payment_clearing` needs a server on port 8005;
  - `hybrid_payment_api` has a stale SQLite file (see to-be-done.md).

**Found while testing:** case 030 (`decision_needed`). Python also
compensates the step that failed.

## 2026-10-28: Go examples build and pass again

- **`examples/go/auth_portal_api`:** `BuildApp` now sets
  `AllowClientJobIDs`, as the Rust auth portal does. The test and the robot
  suite tag runs with `X-Job-ID`, which Go has ignored by default since the
  secure defaults (2026-10-09).
- **`go mod tidy`** in `examples/go/{hybrid_payment_api,order_fulfillment_api,todo_api}`
  and `example/go/{ecommerce/checkout-tax-calculator,finance/realtime-fraud-preauth}`.
  Their `go.mod` files predated the engine's database drivers and its Go 1.26
  requirement. Only the `go` line and indirect dependencies changed.
- **Results:** all seven Go example modules build and vet cleanly, and every
  test passes. The two `example/go` modules have no tests.

## 2026-10-28: D5 refined, so declared routes return real values

**Found:** the auth portal example (Go) failed. Its blueprint masks
`access_token`, and with D5 the login route returned `"********"`, so the
client could never authenticate. Rust had the same problem, but its example
test calls the activities directly and missed it.

**Decided:** a declared `api:` route responds with the real values;
everything else stays masked. See D5 in `conformance/decisions.md`.

**Fixed in both engines:**
- The state that execute returns carries the real output in a separate
  field: `RouteOutput` in Go, `route_output` in Rust. It is never
  serialized, stored or published, and `snapshot` clears it.
- The declared-route handler responds from that field.
- A new test in each engine checks that a login route returns the real
  token while the stored job doesn't contain it:
  - Go: `TestDeclaredRouteReturnsMaskedFieldsToTheCaller`;
  - Rust: `test_declared_route_returns_masked_fields_to_the_caller`.

## 2026-10-28: Go engine conforms to D1–D9 (29/29)

These are the Rust changes, ported to `../go/pkg`:

- **`domain/services/condition_evaluator.go`** was rewritten as a port of
  `condition_evaluator.rs`:
  - the same operator list, numeric strings and indexed paths (D3);
  - the same `rule:` parser, with `and`/`or`/`not`/`&&`/`||`/`!`,
    parentheses and the six comparisons (D2);
  - parsed rules and compiled regexes are cached;
  - `ValidateBlueprintConditions` checks every rule, including branches and
    `api:` routes.
- **`adapters/outbound/yaml_repository.go`:** `SaveBlueprint` refuses a
  blueprint whose rule does not parse (D2). No existing blueprint in the
  repository uses `rule:`.
- **`domain/services/pipeline_manager.go`:**
  - a false condition skips the step's activity, loop or `for_each`, and
    leaves a condition trace with `skipped: true` (D1);
  - sync compensations merge their output into the job data through
    `compensate()`, at all three sites (D4);
  - `runSteps` and the same-`job_id` replay return a masked snapshot (D5).
    The REST handler already masked its responses, so HTTP output doesn't
    change.
- **`domain/services/saga_manager.go`:** `RunCompensations` returns the merged
  output. `ExecuteCompensations` delegates to it.
- **`domain/services/condition_evaluator_test.go`:** the five Rust evaluator
  tests, ported.

**Results:**
- Conformance: Go 29/29, Rust 29/29, Python 17/29.
- `go test ./...`: all pass.
- `go vet`: clean.

## 2026-10-26: database connects can no longer hang the engine

**Found:** a server that accepts TCP and then stalls in the handshake
blocked startup for minutes. Postgres took up to 259 s and MySQL 149 s,
because the 5 s driver timeouts only cover opening the socket. This happened
here because Docker's port proxy accepts connections at once while the
containers are starved, but any overloaded database does the same.

**Fixed in `../rust/src/adapters/outbound/sql_backend.rs`:**
- `within()` runs a blocking connect on its own thread with a hard deadline.
  It is used for every Postgres pool connect and for the MySQL initial
  connect.
- MySQL now uses `try_get_conn` (`get_conn` retries forever), plus 30 s
  read/write timeouts for later queries.
- SQL Server's tokio timeout now covers TLS and login, not just TCP.

**Result:** every stalled connect falls back to memory after 5.0 s.

## 2026-10-26: Rust engine conforms to D1–D9 (29/29)

Changes in `../rust`:

- **`condition_evaluator.rs`** was rewritten:
  - **Operators.** The shared operator list (D3) is ported from the Temporal
    Go engine's `operators.go`:
    - long names and aliases;
    - numeric strings compared as numbers;
    - `in` with lists or comma-separated strings;
    - `contains` on lists;
    - `is_null` counts an empty string as null;
    - `is_true`/`is_false` accept `yes`/`no`/`1`/`0`;
    - `is_business_day` and `older_than_days`, with a small RFC 3339 date
      parser;
    - the live-only `starts_with`, `ends_with` and `not_matches`.
  - **Paths.** Field paths accept indexes (`items[0].sku`).
  - **Rules (D2).** `rule:` is now a real expression language: `and`/`&&`,
    `or`/`||`, `not`/`!`, parentheses, `==`, `!=`, `<`, `<=`, `>`, `>=`,
    strings, numbers, `true`/`false`/`null`, and a bare field read as true
    or false. `and` binds tighter than `or`. Parsed rules are cached.
  - **Load-time check.** `validate_blueprint_conditions` refuses a blueprint
    with an unparsable rule. It runs in the repository's `save_blueprint`,
    so every load path checks it.
- **`pipeline_manager.rs`:**
  - **D1.** A false condition skips a step with an activity or loop. It still
    records a `condition` trace with `skipped: true`, so the visualizer shows
    the decision.
  - **D4.** Sync compensations merge their output into the job data. This
    uses the new `SagaManager::run_compensations`.
  - **D5.** Every execute path, including idempotent replays and signal
    resumes, returns the masked snapshot (the stored view) and never the
    working state.
- **Tests:**
  - 5 new unit tests cover the rule parser, invalid rules, indexed paths,
    calendar operators, and the collection and null operators.
  - The conformance case scripts gained `delay_ms`, and case 022 uses it,
    so the "doesn't wait" check is no longer a race.
- **Results:**
  - Conformance: Rust 29/29, Go 21/29, Python 17/29.

## 2026-10-26: phase 0, part 3: open behaviour decided

- Nine open questions were decided and recorded as rules D1–D9 in
  `conformance/decisions.md`. In every case the recommended option was
  chosen:
  - a false condition skips the step;
  - `rule:` gets full expressions and is refused at load if it does not
    parse;
  - there is one condition format shared with the Temporal engines;
  - compensation output is merged into the job data;
  - masking applies everywhere;
  - the same `job_id` replays the first result;
  - async outcomes are recorded;
  - `exceptions` match by type name or message text;
  - `pivot` is kept and documented.
- All 29 cases are now binding; none is marked `decision_needed`. Case 029
  is new: an invalid rule is refused at load. The checker gained
  `load_fails`.
- Results:
  - Go: 21/29.
  - Rust: 21/29.
  - Python: 17/29.

  The failures are the work list in to-be-done.md, under "Make the engines
  conform".

## 2026-10-26: phase 0, part 2: 28 cases, stored state and async

- The runner contract now covers four more things, in all three runners:
  - the **stored** job (as history and the visualizer show it), read back
    after each run;
  - waiting for **async** steps to finish (`wait_async`);
  - **shared engines** across runs (`shared_engine`);
  - the **order** of activity calls (`sequence`).
- The checker gained `fields`, `stored`, `text_absent` and `sequence`, and
  compares object values in traces as subsets.
- New cases 019–028:
  - masking in traces;
  - masking everywhere;
  - the same `job_id` twice;
  - async success and failure;
  - `max_iterations`;
  - `for_each`;
  - the `exceptions` mapping;
  - compensations in reverse order;
  - `pivot`.
- Results. Documented behaviour (16 cases):
  - Go: 16/16.
  - Rust: 16/16.
  - Python: 14/16. Retries are not applied, as the docs say.
- 12 cases need a decision. See "Needs a decision" in
  `conformance/results/report.md` and the list in to-be-done.md.

## 2026-10-26: phase 0, part 1: conformance suite running on Go, Rust and Python

- `conformance/` holds the case format (see its README), 18 cases, a runner
  for each of Go, Rust and Python, `check.py`, and `run.sh`. One command runs
  everything and writes `results/report.md`.
- Areas covered:
  - sequential steps and data merging;
  - condition gating and branches (`field`/`operator`/`value`);
  - `conditions` groups with `all` and `any`;
  - `rule:` strings;
  - operators;
  - retries, both succeeding after retries and running out;
  - the default retry policy;
  - saga compensation;
  - `settle`, `terminate` and `jump_to`;
  - loops;
  - unknown activities.
- Cases the docs define (12):
  - Go: 12/12.
  - Rust: 12/12.
  - Python: 10/12. Retries are not applied, which matches the docs:
    `retry_policy` is listed as "Go only".
- Undocumented behaviour (6 cases, marked `decision_needed`) where the
  engines disagree:
  - Should a `condition` skip the step's own activity? Python skips it; Go
    and Rust run it.
  - `rule:` expressions with `or` and `>=`: only Python supports them.
  - Is compensation output merged into the job data? Python merges it; Go and
    Rust discard it.
  - The Temporal engines' condition format (`branching_step`, `conditions`,
    `then`/`else`, long operator names) is half-supported by Go and Rust, and
    not at all by Python.
- Bug found and fixed in the Rust engine: `matches`/`regex` used a
  hand-written matcher with no character classes, escapes or groups, so most
  real patterns never matched (`^[a-z]+@example\.com$`, for example). It now
  uses the `regex` crate (RE2 semantics, like Go's `regexp`) with a cache of
  compiled patterns. Rust tests: 49/49 (`--no-default-features`).

## 2026-10-26: core workspace scaffolded

- Created `core/` as its own Cargo workspace, with two crates:
  - `crates/octopus-core`: an empty library holding `VERSION` and
    `CONTRACT_VERSION` (0).
  - `crates/octopus-ffi`: builds as cdylib, staticlib and rlib. It exports
    `octopus_core_version`, `octopus_contract_version` and `octopus_buf_free`,
    and establishes the `OctopusBuf` ownership rule. One test covers the
    buffer round trip.
- Wrote `plan.md`, which covers:
  - the architecture;
  - the core/host split;
  - the FFI contract;
  - the decisions;
  - the phases;
  - the risks.

## 2026-10-26: decisions taken

- A Rust core with host languages at the edges. The core owns engine
  semantics; each SDK owns HTTP and activity code.
- A C ABI that passes JSON at step granularity.
- Temporal is an optional adapter behind `IAsyncActivityDispatcher`. The core
  owns the cross-language contract, and Go and Java may keep their native
  Temporal SDKs.
- The visualizer's diagram compiler and frontend go into the core. The graph
  JSON schema is versioned and covered by golden tests.
- Go's binding is decided last, from measurements.
- The conformance suite is built before any extraction.

## 2026-10-26: Rust live engine (`../rust`) brought up to the Go and Python engines

This is the engine the core will be extracted from (phase 1).

- **HTTP hardening:**
  - limits: 32 MiB body, 64 KiB request head, 100 headers, 30 s read timeout;
  - responses: 413 / 414 / 431 / 400;
  - 6 tests at the TCP level.
- **Real state stores**, using Go's tables (`execution_states`, `step_traces`)
  and Go's Redis keys:
  - Redis: pooled connections, `SCAN` + `MGET`, events relayed between
    instances over pub/sub;
  - Postgres, MySQL, SQLite (WAL) and SQL Server (tiberius);
  - each save is one transaction, connections are pooled, list queries are
    batched;
  - if the database is unreachable, the engine falls back to memory with a
    rate-limited warning.
- **A real Temporal dispatcher and worker** (`temporal` feature, needs `protoc`):
  - workflow `IOctopusWorkflow`;
  - activity names are chosen at runtime;
  - the step's retry policy is mapped to Temporal's retry policy;
  - completion hooks;
  - fallback to in-process execution.
- **A real Redis response cache** in `RedisCacheMiddleware`, backing off when
  Redis is down.
- **Verified against real servers:**
  - Redis, Postgres, MySQL and SQLite;
  - the in-memory fallback when the database is down;
  - Temporal end to end (`temporal server start-dev`).
- **Test results:**
  - default build: 50 passed, 0 failed;
  - `--features temporal`: all passed;
  - `--no-default-features`: 49 passed, 0 failed;
  - examples: all passed;
  - `cargo fmt`: clean.
