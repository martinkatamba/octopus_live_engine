# OCTOPUS Live Engine: Architecture Specification

## 1. Design Principles

1. **Sub-10ms Guarantee**: In-memory state storage and local method dispatching bypass network hops, serialization overhead, and database locks.
2. **Hexagonal Purity**: The domain kernel contains zero framework dependencies (`fastapi`, `gin`, `aspnet`, `httpserver`).
3. **Decoupled Outbound Ports**: Storage, activity invocation, and blueprint sources are swappable via adapters.
4. **Visual Traceability**: Every execution trace captures microsecond-level timing and payload deltas mapped to AntV X6 nodes.

## 2. In-Memory Saga Rollbacks

When a step fails:
1. The `PipelineManager` catches the exception and marks the step trace as `FAILED`.
2. The `SagaManager` iterates backwards through the recorded saga stack.
3. Each step's registered compensation activity is invoked with the state payload snapshot.
4. The workflow status transitions from `RUNNING` -> `FAILED` -> `ROLLED_BACK`.
5. Error traces and rollback events are published to the SSE stream.

## 3. Idempotency LRU Cache

The `IdempotencyManager` stores completed execution states keyed by `job_id`:
- If an incoming synchronous request presents a previously completed `job_id`, the engine returns the cached `ExecutionState` immediately without re-executing activities.
- Expired entries are automatically evicted based on the configured TTL (default: 1 hour).
