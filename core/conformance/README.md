# Conformance suite

Language-neutral cases that define how a live engine behaves. Every engine
runs the same blueprints with the same scripted activities, and one checker
compares the results with the expectations. The core (`../crates/octopus-core`)
must pass every case before it replaces an engine.

```
./run.sh            # Go, Rust, Python and the core; writes results/report.md
./run.sh --strict   # exit 1 if any documented case fails (for CI)
```

Running it needs Go, Cargo and Python 3.11+. The Python runner uses the
repository's `.venv` when there is one; set `PYTHON=...` to pick another
interpreter.

## Layout

```
cases/NNN_name/blueprint.yaml   an ordinary live-engine blueprint
cases/NNN_name/case.json        activity scripts, inputs and expectations
runners/{go,rust,python}/       one small program per engine
runners/core/                   the core directly, with no HTTP host
check.py                        compares the runners' output with the cases
run.sh                          runs everything
```

## case.json

```json
{
  "description": "What the case shows.",
  "decision_needed": "Optional. Set when the behaviour is undocumented; see below.",
  "workflow": "workflow_name from blueprint.yaml",
  "activities": {
    "Flaky": [{"fail": "transient"}, {"return": {"ok": true}}],
    "Echo":  [{"echo": {"saw_amount": "amount"}, "return": {"done": true}}]
  },
  "runs": [
    {"input": {"amount": 5}, "expect": {"status": "COMPLETED", "data": {"ok": true}}}
  ]
}
```

**Activity scripts.** Each activity has a list of entries. Call *n* uses
entry *n*, and the last entry repeats once the list runs out. An entry
either fails or produces output:

- `fail`: the activity fails with this message.
- `echo`: `{output_key: input_field}` copies fields of the activity's input
  into its output. A missing field becomes `null`.
- `return`: fixed output. It is merged after `echo`.
- `delay_ms`: wait this long before failing or returning (to show that
  async steps are not awaited, for example).

**Runs.** Each run uses a fresh engine, unless the case sets
`"shared_engine": true`. Then all runs share one engine, and call counts
accumulate across runs. That is how cases test what happens when the same
`job_id` is sent twice.

For each run, the runner loads `blueprint.yaml`, registers the scripted
activities, and calls the engine's sync execute with `input`. It then reads
the job back from the state store, which gives the *stored* state that
history and the visualizer show. With `"wait_async": true` on a run, it waits
(up to 5 s) until no step is still `ASYNC_DISPATCHED` before reading.

**Expectations.** Every key is optional:

| Key | Passes when |
|---|---|
| `status` | the final state's status equals it |
| `data` | every listed key is in the job data with that value (nested objects are compared as subsets; `1` equals `1.0`) |
| `data_absent` | none of these keys are in the job data |
| `lengths` | each key is a list of exactly this length |
| `traces` | the state has exactly this many traces, and each listed field matches, in order (`attempts` defaults to 1) |
| `calls` | each activity was called exactly this many times |
| `fields` | top-level state fields match (e.g. `{"error_code": 402}`) |
| `sequence` | the activities were called in exactly this order (all of them) |
| `error_contains` | the execute error, the state's `error`, or a trace's `error_message` contains this text |
| `text_absent` | none of these strings appear anywhere in the state (for masking) |
| `load_fails` | the blueprint is refused when it loads (no other key is checked) |
| `stored` | the same keys, checked against the stored state instead of the state execute returned |

In `traces`, an object value such as `input_payload` is compared as a subset.

## Runner contract

A runner takes the cases directory as its only argument and prints one JSON
object on stdout. It decides nothing; `check.py` does all the comparing.

```json
{"engine": "go", "cases": {"001_sequential_merge": [
  {"state": {"status": "COMPLETED", "data": {}, "traces": []},
   "stored": {"status": "COMPLETED", "data": {}, "traces": []},
   "error": null, "calls": {"StepA": 1}, "sequence": ["StepA"]}
]}}
```

The list has one entry per run. `state` is the engine's execution state,
serialized the way its REST API would serialize it. `error` is the execute
call's error, if there was one. `stored` is the job read back from the state
store by its `job_id` (`null` if it isn't there). `calls` counts the calls of
each activity, and `sequence` lists every call in order. If the blueprint fails to load, the entry is
`{"load_error": "..."}` instead.

A runner for another language (Java, .NET, the core's FFI) only needs to
produce this output.

## Undocumented behaviour

A case with `decision_needed` covers behaviour that the docs
(`octopus_workflow_docs/docs/09-live-engine/overview.md`) don't define, where
the engines disagree. The same applies to a row of the docs' "Differences
between the SDKs" table: the core has to pick one behaviour. Its expectations are a proposal, not a rule. A mismatch
shows as ⚠️, is not counted as a failure, and is listed under "Needs a
decision" in the report.

Once the behaviour is decided:

1. Set the expectations to match the decision.
2. Delete `decision_needed`.
3. Document the behaviour.

From then on, a mismatch is a ❌.

## Adding a case

1. Create `cases/NNN_short_name/`, using the next free number.
2. Write `blueprint.yaml` with a `workflow_name` no other case uses.
3. Write `case.json`. Assert what the docs promise and nothing incidental,
   such as timings or generated IDs.
4. Run `./run.sh`. If the engines disagree and the docs are silent, add
   `decision_needed`.
