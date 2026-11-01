# To be done

Open work, in order. Phases are described in [plan.md](plan.md). Move each item
to [done.md](done.md) when it is finished.

## Carried over from the Rust engine (`../rust`)

- [ ] Verify the SQL Server store against a live server. The container crashes
      with Docker's ~2 GB memory limit; give Docker ≥ 4 GB and run
      `OCTOPUS_TEST_SQLSERVER_DSN=... cargo test --test state_store_integration_test sqlserver`.
- [ ] Decide on the 15 clippy style lints in older code: large `Err` variants,
      `field_reassign_with_default`, complex types, and a module with the same
      name as its parent. All 15 are in `octopus-core` now; `../rust` has
      none.
- [ ] Add a Rust column to the SDK-differences table in
      `octopus_workflow_docs/docs/09-live-engine/overview.md`.
- [ ] Parity gap with Python: there is no equivalent of
      `temporal_replicator.py`. Decide whether the core needs one. (The
      DuckDB store was added on 2026-10-30.)
- [ ] Postgres queries have no client-side timeout (the sync `postgres`
      crate has none). Consider `statement_timeout` in the session, so a
      stalled server fails a save instead of blocking it.
- [ ] The conformance runner contract can't express declared routes, so the
      D5 route rule is only covered by each engine's own test. Add it once
      the runner contract covers HTTP.

## Phase 0: conformance suite

Parts 1–3 are done; see [done.md](done.md). What is left:

- [ ] **Make the engines conform** to [conformance/decisions.md](conformance/decisions.md)
      (D1–D9 decided 2026-10-26, D10 on 2026-10-28). Failing cases as of 2026-10-26:
  - [x] Rust: 29/29 (see done.md).
  - [x] Go: 29/29 (see done.md).
  - [x] Python: 29/29 (see done.md).
  - [ ] .NET and Java: unknown until they have runners.
- [ ] Python's `relational_store.py` (used for `store: sqlite`) has no column
      migrations, unlike `sqlite_store.py`. A database created before
      `client_host` was added fails every save. That is why
      `examples/python/hybrid_payment_api` fails with its old
      `data/hybrid_payments.sqlite` (gitignored); deleting the file works
      around it.
- [ ] Python runs plain (non-async) activities on the event-loop thread, so a
      slow one blocks every other request. Consider `asyncio.to_thread`.
- [ ] Open detail of D4: with `compensation_mode: async`, compensations run
      after the run has returned, so their output can't be merged into the
      returned state. Should it be merged into the stored job when they
      finish?
      (Same in Rust and Go.)
- [ ] Open detail of D3: an unknown operator name is false in the live
      engines. The Temporal engines fall back to `equals` instead. Pick one,
      or refuse unknown operators at load, like D2.
- [ ] Move each rule into the public docs (`octopus_workflow_docs/docs/09-live-engine/`)
      once engines conform, and update the "Differences between the SDKs"
      table: the masking and same-`job_id` rows, and `retry_policy` once
      Python applies it. D14–D21 are new since: exception base classes,
      the event payloads, `job_<uuid>` IDs, `timeout_seconds`, the
      visualizer's default route, loop traces, condition traces and
      the order of a step's work and its branch.
- [ ] More cases:
  - [ ] step-level `mask_fields` and `masking` with a custom `mask_value`;
  - [ ] middleware and global `exceptions`, and the global exception handler;
  - [ ] `delay_seconds`, `timeout_seconds`;
  - [ ] `rewind_to_step_index`, `jump_to_step_index`;
  - [ ] `compensation_mode: async`;
  - [ ] `is_output`;
  - [ ] `await_signal` / `human_step` with signals;
  - [ ] declared `api:` routes (needs an HTTP-level runner contract);
  - [ ] each middleware. The core's `RequestMeta` / `Decision` is the shape
        to add to the runner contract; the core runner can call
        `Engine::middleware_chain()` directly.
- [ ] Store round trips checked against the stored rows and keys. Needs live
      databases; reuse `../rust/tests/state_store_integration_test.rs`.
- [ ] Golden cases for the visualizer graph JSON (`compile_x6_diagram`).
- [ ] Runners for Java and .NET.
- [ ] Run `conformance/run.sh --strict` in CI.

## Phase 1: extract the core

The extraction is done; see done.md. What is left:

- [x] Move the middleware decisions into the core (2026-10-29, see done.md).
- [x] Add the engine wrapper the FFI needs: `octopus_core::Engine`
      (2026-10-29, see done.md).
- [ ] Decide whether `regex` should stay the core's only required dependency,
      or move behind a feature.

## Phase 2: FFI and Python spike (go/no-go)

Done on 2026-10-29; results and recommendation in
[phase-2-results.md](phase-2-results.md).

- [x] Implement the contract from `plan.md` in `octopus-ffi`: `catch_unwind`
      on every export, `octopus_last_error`, the activity callback trampoline.
- [x] Generate `include/octopus.h` with cbindgen. `scripts/check-ffi.sh`
      verifies it and runs a C program against it.
  - [ ] Run `scripts/check-ffi.sh` in CI. The repository has no CI yet.
- [x] PyO3 binding that releases the GIL during runs and takes it back for
      each activity call (`bindings/python`).
- [x] Run `order_fulfillment_api` on the core.
- [x] Benchmark against pure Python and write up the numbers.
- [x] **Decide go/no-go.** Go, decided 2026-10-29.

## Before phase 3: the core's per-step cost

Done on 2026-10-29 (done.md). Measured in pure Rust
(`cargo run --release -p octopus-core --example bench_order`), a 100-item
order went from 508 ms to 1.85 ms. Through Python the core is now faster
than the Python engine on every workload.

- [x] Stop copying the whole job: `ValueMap` is copy-on-write, so traces,
      snapshots and stored states share the job's data.
- [x] Build a live event only when someone can receive it.
- [x] Reuse threads for async steps (`workers.rs`, a cached pool).
- [x] Open `/dev/urandom` once per process for job IDs.
- [x] Rerun `bindings/python/bench/bench.py` with 50- and 100-item orders,
      8 orders in flight, and SQLite against SQLite.

What is left grows with the payload at the boundary, not in the core:

- [x] Convert each shared map once at the PyO3 boundary (2026-10-29,
      done.md). A 100-item order went from 73 ms to 42 ms (3.7× faster than
      the Python engine).
- [ ] What remains is copying each activity's whole payload into Python and
      back. The Python engine passes its own dicts, which lets an activity
      change the engine's state; the copy is what keeps the core safe.
      Going further changes the API, so it needs a decision:
      - a read-only mapping instead of a dict;
      - or reusing unchanged dicts across calls, watched with
        `PyDict_AddWatcher`. That needs Python ≥ 3.12, and lists can't be
        watched.
- [ ] The C ABI writes a shared map out again each time it appears in the
      JSON, so at 100 items it is no faster than the Python engine (154 ms).
      The .NET and Java hosts will see this. One option is a call that
      returns the state without trace payloads.
- [ ] Repeat on a quiet machine. These numbers were taken at load 8 → 5.
- [ ] Arrays are still copied element by element (`Value::Array(Vec)`).
      Make them copy-on-write too if a profile shows it.

## Phase 3: Python SDK on the core

Started 2026-10-29. Since 2026-10-30 `octopus_live` runs on the core by
default whenever `octopus_core` is installed; `engine="python"` or
`OCTOPUS_LIVE_ENGINE=python` picks the Python engine. Done so far is in
done.md.

- [x] **Step 1: the engine.** The core runs the steps, keeps the jobs and
      publishes the events (`octopus_live/core_backend.py`). The same tests
      pass on both engines:
      - the SDK's 60;
      - the example tests;
      - the robot suites;
      - the two outer examples that use `octopus_live`.
- [x] **Step 2: hand the rest to the core** (D11–D13; see done.md).
      Middlewares, the visualizer and hybrid mode run on the core.
- [x] **Fallbacks to Python** removed (2026-10-30, see done.md): a Python
      `IStateStore` object, TLS DSNs and customised built-in middlewares
      run on the core, and DuckDB does in a build with `--features duckdb`.
      What is left:
      - [ ] MySQL `ssl-mode=PREFERRED` (the mysql client's default)
            connects without TLS. The `mysql` crate can't try TLS and fall
            back; libpq-style `sslmode=prefer` on Postgres does.
      - [ ] A Python `IStateStore` called on the event loop's own thread
            runs its coroutines on a private loop. An `asyncio.Lock` (or
            similar) shared with the server loop could then bind to the
            wrong loop. The SDK moves the core calls that save off the loop,
            so this needs a store used from a plain (non-async) activity.
      - [ ] DuckDB is opt-in (`--features duckdb`), so the default wheel
            still runs `store: duckdb` on the Python engine. Revisit
            before phase 7 deletes that engine: a separate wheel with
            DuckDB, or a lighter DuckDB build.
      - [ ] Verify TLS against real servers. The tests use small fake
            Postgres and MySQL servers; the Docker databases have no TLS
            set up.
- [x] **Differences decided** (2026-10-31, D14–D18 in
      `conformance/decisions.md`, cases 031–035; see done.md). Exception
      base classes, live event payloads, job IDs, `timeout_seconds` and the
      visualizer's default route now behave the same on every engine;
      hybrid replication stays dropped (D13, confirmed).
- [ ] **Wheels:** `.github/workflows/python-wheels.yml` builds abi3 wheels
      (manylinux and musllinux x64/arm64, macOS x64/arm64, Windows x64) and
      tests them on both engines. Written, not yet run in CI. A local macOS
      x64 wheel is built and tested (see done.md). Publishing to PyPI is not
      set up.
- [ ] **Dev setup:** both venvs still import `octopus_core` through a
      `_octopus_core_dev.pth` file pointing at `core/bindings/python/python`.
      Replace it with an installed wheel (`maturin develop`) once CI
      publishes one.
- [x] Make the core the default when `octopus_core` is installed.
- [ ] Make `octopus-core` a required dependency once wheels are on PyPI;
      until then the SDK installs without it and runs on Python.
- [ ] Python engine only: `sqlite_path=` (or `redis_url=`) without
      `state_store=` keeps jobs in memory; the core infers the store.
- [ ] Delete the Python engine internals (phase 7).

**Failing on both engines before this work** (not caused by the core):
- `hybrid_payment_api`: one test needs a Temporal server.
  `test_zero_import_store_factory` expects a `SqliteStateStore`, but gets
  `RelationalSqlStateStore`.
- `cross_border_payment_clearing`: all 6 tests expect a server already
  running on a port.

## Phase 4: .NET SDK on the core

- [ ] Bind through P/Invoke with `csbindgen`, keeping callback delegates alive.
- [ ] Ship a NuGet package with `runtimes/{rid}/native`.
- [ ] Make the .NET examples pass.

## Phase 5: Java SDK on the core

- [ ] Choose Panama FFM (Java 22+) or JNI.
- [ ] Attach core threads to the JVM for callbacks.
- [ ] Ship Maven classifier jars for each platform.
- [ ] Make the Java examples pass.

## Phase 6: Go, decide

- [ ] Benchmark native Go against a `purego` binding.
- [ ] Record the decision. If Go stays native, gate it on the conformance suite.

## Phase 7: retire the duplicates

- [ ] Run each SDK on the core behind a flag for one release.
- [ ] Delete the old engine internals in each SDK.

## Shared infrastructure

- [ ] A CI matrix that builds `octopus-ffi` once per target and publishes
      artifacts for every binding.
- [ ] Cross-language Temporal tests: Rust dispatches and Go executes, and the
      other way round.
- [ ] A release process: core version, `CONTRACT_VERSION` bumps, and binding
      version pins.
