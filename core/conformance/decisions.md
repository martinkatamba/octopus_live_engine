# Behaviour decisions

Rules for behaviour that the live engine docs left open, or where the docs'
"Differences between the SDKs" table showed the engines disagreeing. They
were decided on 2026-10-26.

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
| D13 | `mode: hybrid` sends async steps to Temporal while the job runs, through the core's Temporal adapter, and stores jobs in the blueprint's `persistence:` store. Python's after-the-run replication is not carried over. Decided 2026-10-29. | — | Python |

"Engines to change" lists the engines that still don't conform. Rust has
conformed to all nine rules since 2026-10-26, and Go and Python since
2026-10-28. D9 still needs documenting in the public docs. The conformance report shows
the current state.
