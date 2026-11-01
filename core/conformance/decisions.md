# Behaviour decisions

Rules for behaviour that the live engine docs left open, or where the docs'
"Differences between the SDKs" table showed the engines disagreeing. They
were decided on 2026-10-26; D10 onwards carry their own dates.

Each rule is enforced by the cases listed beside it. The core implements these
rules, and every SDK is held to them. Once an engine conforms, the rule moves
into the public docs (`octopus_workflow_docs/docs/09-live-engine/`).

| # | Rule | Cases | Engines to change |
|---|---|---|---|
| D1 | A step with both an `activity` and a `condition` runs its activity only when the condition holds. A false condition skips the step; the steps around it still run. | 002 | — |
| D2 | `rule:` expressions support `and`, `or`, `==`, `!=`, `<`, `<=`, `>`, `>=`. A rule that does not parse is refused when the blueprint loads; it is never evaluated as true or false. | 005, 029 | — |
| D3 | Live and Temporal blueprints share one condition language: `condition` (`field`/`variable`, `operator`, `value`, `then_steps`/`else_steps`), `branching_step`, `conditions` groups with `match: all` or `any`, `then`/`else`, and the Temporal engines' operator list (long names, with the short forms such as `gt` and `eq` as aliases). | 004, 015, 016, 018 | — |
| D4 | A compensation's output is merged into the job data, so the final state shows what was undone. | 014 | — |
| D5 | A field listed under `masking` is hidden everywhere the engine records or shows a job: the state that execute returns, the stored job's data and output, step traces, history, events, the visualizer, and the generic `/execute`, signal and error responses. Two things get the real value: activities, and the response of a declared `api:` route, which the engine passes to the host separately from the state (refined 2026-10-28, so a login route can return the token it issues). | 019, 020 | .NET, Java |
| D6 | Sending the same `job_id` again replays the stored result of the first run; the steps do not run again. | 021 | — |
| D7 | The stored job records each async step's outcome: `ASYNC_COMPLETED`, or `ASYNC_FAILED` with the error. | 022, 023 | — |
| D8 | An `exceptions` entry matches an error by its type name or by a case-insensitive substring of its message. | 026 | — |
| D9 | `pivot: true` is supported and documented. Once a pivot step has run, the steps before it are no longer compensated if a later step fails. | 028 | — |
| D10 | When a step fails, only the steps that completed before it are compensated, newest first. The failed step's own compensation does not run: a step that fails part-way cleans up inside its own activity. Decided 2026-10-28. | 009, 027, 030 | — |
| D11 | The built-in middlewares behave as the core's do, in every SDK. The core gains what only Python had: `RoleAuthorizationMiddleware`, `require_json_body`, a 10 MB default body limit, `query:<name>` cache keys, and `tenant_id`/`username` claims as rate-limit keys. The client IP is the one the host derives, which honours `X-Forwarded-For` only from trusted proxies; a client can no longer pick its own IP. Decided 2026-10-29. | — (middleware has no cases yet) | Python |
| D12 | The visualizer's graph JSON and its page are the core's, in every SDK. Decided 2026-10-29. | — | Python |
| D13 | `mode: hybrid` sends async steps to Temporal while the job runs, through the core's Temporal adapter, and stores jobs in the blueprint's `persistence:` store. Python's after-the-run replication is not carried over: Temporal holds the async steps and the `persistence:` store holds the job. Decided 2026-10-29, confirmed 2026-10-31. | — (needs Temporal) | Python (its engine is retired in phase 7) |
| D14 | An `exceptions` entry also matches an error whose base type it names (D8 otherwise unchanged). A host sends the type and its base types, most specific first, as `[Type,Base,...:code] message`; Python sends the class's MRO, Go the `OctopusMiddlewareError` names in the error's `Unwrap` chain. Decided 2026-10-31. | 031 | — |
| D15 | Live events are the core's: `workflow_started`, `workflow_completed`, `workflow_failed`, `workflow_rolled_back`, `workflow_suspended` and `workflow_resumed` carry the job's masked state; `step_started` carries `step_id`, `activity`, `index` and `total_steps`; `step_completed` carries the step's trace. Decided 2026-10-31. | 032 | — |
| D16 | A job started without a `job_id` gets `job_` and a version 4 UUID. Job IDs are opaque: nothing may parse them. Decided 2026-10-31. | 033 | — |
| D17 | A step's declared `timeout_seconds` bounds each attempt of its activity; past it the step fails with `ActivityTimeout` (504). A step that declares none has no limit: there is no default. Decided 2026-10-31. | 034 | — |
| D18 | With no job, route or method named, the visualizer draws the first declared route that has steps; with no routes, the top-level steps. Decided 2026-10-31. | 035 | — |
| D19 | A loop step's trace output has `iterations`, `results` and `failures`, the lists empty rather than missing or null. An item that fails fails the step, unless the loop sets `continue_on_error`: then the item adds `{"_error", "_index"}` to the results and `item <i>: <error>` to the failures, and the loop goes on. Decided 2026-11-01. | 036 | — |
| D20 | A condition is traced when it decides a step that has no work of its own, or skips a step that has. The trace's activity is `condition`; its input holds every field the condition names, keyed by the path as written, whether or not a short-circuited `and`/`or` reached it (null when missing, masked like any payload); its output holds `result`, `branch` (`then`, `else` or `skip`) and, when the step's own work did not run, `skipped: true`. Engines may add keys of their own to the output. Decided 2026-11-01. | 037 | — |
| D21 | A step with an activity (or loop), a condition and branches runs its own work first when the condition holds, then the `then` branch, then the next step; the branch sees the work's output. When the condition is false the work is skipped (D1) and the `else` branch runs. Decided 2026-11-01. | 038 | — |

"Engines to change" lists the engines that still don't conform. Rust has
conformed to all nine rules since 2026-10-26, and Go and Python since
2026-10-28. On D14–D18 the core, Rust, Go and Python conform since
2026-10-31, and on D19–D21 since 2026-11-01. D9 still needs documenting in the public docs. The conformance report shows
the current state.
