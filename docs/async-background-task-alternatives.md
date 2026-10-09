# Architecture Guide: Background Async Task Alternatives to Temporal

This document evaluates alternatives to **Temporal** for executing background asynchronous activities in the **OCTOPUS Live Engine** across **Go**, **Python**, **.NET (C#)**, and **Java (21+)**.

---

## 1. Executive Summary & Architectural Context

In the OCTOPUS Live Engine, request execution is decoupled into two planes:
* **Synchronous Fast Path**: Ultra-low latency (< 1ms) in-memory execution for user-facing HTTP/REST endpoints.
* **Asynchronous Background Activities**: Decoupled execution for side-effects, audit logging, email/SMS dispatch, webhooks, heavy compute, and distributed sagas.

All engines implement the outbound port **`IAsyncActivityDispatcher`** (Go, Python, .NET, Java), allowing the underlying background queue/engine to be swapped via declarative YAML configuration without modifying business activity code.

```mermaid
flowchart TD
    Req([HTTP / API Request]) --> Engine[OCTOPUS Live Engine]
    Engine -->|Sub-millisecond| Resp([HTTP Response])
    
    Engine -->|IAsyncActivityDispatcher| Dispatcher{Dispatcher Strategy}
    
    Dispatcher -->|High Durability Sagas| Temporal["Temporal / Cadence / Hatchet"]
    Dispatcher -->|Ultra-Fast & Simple| Redis["Redis Queues (Asynq / Celery / Hangfire)"]
    Dispatcher -->|ACID Transactional Outbox| DB["Postgres / MySQL (River / Skip Locked)"]
    Dispatcher -->|Enterprise Event Mesh| AMQP["RabbitMQ / Kafka / NATS"]
    Dispatcher -->|Cloud-Managed Serverless| Cloud["AWS SQS / GCP Cloud Tasks"]
    Dispatcher -->|Local / Testing| Mem["In-Memory Channels"]
```

---

## 2. Categorized Alternatives to Temporal

### Category A: Modern Durable Workflow Orchestrators (Direct Temporal Competitors)

| Engine | Tech & Storage | Key Strengths | Trade-Offs / When to Use |
| :--- | :--- | :--- | :--- |
| **[Hatchet](https://hatchet.run/)** | Go + PostgreSQL | • **Zero cluster bloat**: Runs entirely on top of PostgreSQL.<br>• Extremely low latency dispatch.<br>• Real-time web UI and multi-tenant rate limits.<br>• Ideal for AI pipelines and async tasks. | Newer ecosystem than Temporal, but drastically simpler to host and operate on existing Postgres infrastructure. |
| **[Inngest](https://www.inngest.com/)** | Go / Cloud | • **Serverless-native**: Executes steps over standard HTTP.<br>• Zero persistent worker polling required.<br>• Built-in step-level retries, throttling, and debouncing. | Primarily cloud/SaaS-focused; self-hosting requires containerized Inngest dev server. |
| **[Cadence](https://cadenceworkflow.io/)** (Uber) | Go / Java / Cassandra | • Battle-tested at Uber scale across millions of executions.<br>• Temporal's predecessor with similar programming model. | Complex infrastructure footprint (requires Cassandra / MySQL / OpenSearch). |
| **[Dapr Workflows](https://dapr.io/)** | Polyglot + Pluggable State Stores | • CNCF cloud-native sidecar architecture.<br>• Uses existing state stores (Redis, Postgres, CosmosDB).<br>• Native multi-language SDKs (Go, Python, .NET, Java). | Requires Dapr sidecar runtime running alongside the engine. |

---

### Category B: Lightweight Distributed Message & Task Queues (High Throughput, Low Overhead)

| Technology | Ecosystem Libraries | Key Strengths | Best Use Case |
| :--- | :--- | :--- | :--- |
| **Redis Streams / Queues** | • **Go**: `Asynq`<br>• **Python**: `Celery`, `ARQ`, `SAQ`<br>• **.NET**: `Hangfire`, `Coravel`<br>• **Java**: `Redisson`, `JobRunr` | • **Sub-millisecond dispatch** latency.<br>• Ubiquitous: Redis is already in almost every stack.<br>• Built-in delayed tasks, exponential retries, priority queues, and cron scheduling. | High-throughput web applications needing simple, reliable background tasks without orchestrator complexity. |
| **RabbitMQ / AMQP** | • Native AMQP 0-9-1 / Stream drivers in all 4 languages | • Sophisticated routing topologies (Direct, Topic, Fanout, Headers).<br>• Dead-letter exchanges (DLX) and fine-grained acknowledgments.<br>• Message durability and clustering. | Enterprise service-oriented architectures with multi-service pub/sub routing. |
| **Apache Kafka / Redpanda** | • `confluent-kafka` / `segmentio/kafka-go` | • Massive scale (millions of events/sec).<br>• Partition ordering and event replayability.<br>• Event-sourced log retention. | High-volume stream processing and event sourcing across teams. |
| **NATS / NATS JetStream** | • Official NATS SDKs (Go, Python, C#, Java) | • Cloud-native, single lightweight binary (< 20MB).<br>• Ultra-low resource usage, high performance.<br>• Built-in JetStream persistence, key-value, and object store. | Modern Kubernetes, microservices, and edge computing architectures. |

---

### Category C: Database-Backed Transactional Queues (Transactional Outbox Pattern)

Using `FOR UPDATE SKIP LOCKED` on relational database engines (**PostgreSQL**, **MySQL 8.0+**, **Microsoft SQL Server**):

* **Go**: [River](https://riverqueue.com/) (Modern high-performance Postgres job queue)
* **Python**: [Procrastinate](https://procrastinate.readthedocs.io/)
* **.NET & Java**: Quartz.NET / JobRunr / Custom SQL Table Poller.

#### The "Killer Feature": Atomic Transactional Enqueuing
```mermaid
sequenceDiagram
    participant App as API Request
    participant DB as Relational Database
    participant Worker as Background Worker

    Note over App,DB: Single ACID Transaction
    App->>DB: 1. INSERT INTO orders (order_id, user_id, total)
    App->>DB: 2. INSERT INTO octopus_task_queue (activity, payload, status)
    App->>DB: COMMIT TRANSACTION
    
    Note over DB,Worker: Worker Polls with SKIP LOCKED
    Worker->>DB: SELECT * FROM octopus_task_queue FOR UPDATE SKIP LOCKED
    Worker->>Worker: Execute activity (SendReceiptEmail)
    Worker->>DB: UPDATE octopus_task_queue SET status = 'COMPLETED'
```
> [!TIP]
> **Zero Dual-Write Problem**: If the API transaction fails or rolls back, the background task is **never enqueued**. No phantom emails or orphaned tasks.

---

### Category D: Fully-Managed Cloud Task Services (Zero Ops / Serverless)

| Provider | Services | Strengths |
| :--- | :--- | :--- |
| **AWS** | **Amazon SQS + EventBridge + Step Functions** | Fully managed, unlimited scale, dead-letter queues, FIFO ordering, pay-per-use. |
| **Google Cloud** | **Cloud Tasks + Cloud Pub/Sub + Cloud Workflows** | Built-in HTTP target invocation, task deduplication, concurrency throttles, rate token buckets. |
| **Microsoft Azure** | **Azure Service Bus + Azure Durable Functions** | Enterprise messaging, sessions, topic subscriptions, serverless stateful functions. |

---

## 3. Comprehensive Comparison Matrix

| Dimension | **Temporal** | **Hatchet** | **Redis (Asynq/Celery)** | **Postgres (River/Outbox)** | **RabbitMQ / NATS** | **AWS SQS** |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Operational Overhead** | High (Cluster + DB + ES) | Low (Single Postgres DB) | Low (Single Redis instance) | **Lowest** (Existing DB) | Medium (RabbitMQ cluster) | **Zero** (Fully managed) |
| **Saga Rollback Support** | Native (Deterministic) | Native | Manual Code | Manual Code | Manual Code | Step Functions |
| **Dispatch Latency** | ~10–25ms | ~2–5ms | **< 1ms** | ~2–5ms | **< 1ms** | ~10–30ms |
| **Transactional Outbox** | No (Requires 2PC or poller)| Partial | No | **Native (100% ACID)** | No | No |
| **Max Task Duration** | Years / Infinite | Days / Weeks | Minutes / Hours | Hours / Days | Minutes / Hours | 15 min (Lambda) / Days |
| **Memory / CPU Footprint** | ~500MB–2GB+ | ~50MB | ~20MB | Shared with DB | ~50–100MB | 0MB (Cloud API) |
| **Multi-Language Support** | Go, Py, TS, Java, .NET | Go, Python, TS | Go, Py, .NET, Java | Go, Py, .NET, Java | All languages | All languages |

---

## 4. Unified Declarative Blueprint Specification

In the OCTOPUS Live Engine, developers can select their preferred async dispatcher directly in `workflow.yaml`:

```yaml
name: customer_auth_portal
version: "1.0.0"

persistence:
  # State Store for Traces & Visualizer:
  store: postgres
  dsn: "postgresql://postgres:secret@localhost:5432/octopus_db"

  # Async Activity Dispatcher Options:
  # -------------------------------------------------------------
  # Option 1: Temporal (Default Distributed Orchestrator)
  # async_dispatcher: temporal
  # temporal:
  #   host: "127.0.0.1:7233"
  #   namespace: "default"
  #   task_queue: "auth_portal_api"

  # Option 2: Redis Queue (Ultra-fast, low resource)
  async_dispatcher: redis
  redis:
    url: "redis://localhost:6379"
    queue: "auth_portal_tasks"
    concurrency: 20

  # Option 3: PostgreSQL Transactional Outbox Queue (Zero extra infra)
  # async_dispatcher: pg_queue
  # pg_queue:
  #   table_name: "octopus_task_queue"
  #   poll_interval_ms: 100

  # Option 4: RabbitMQ / AMQP
  # async_dispatcher: rabbitmq
  # rabbitmq:
  #   url: "amqp://guest:guest@localhost:5672/"
  #   exchange: "octopus_events"
  #   routing_key: "auth.audit"

  # Option 5: Hatchet (Modern Postgres-native orchestrator)
  # async_dispatcher: hatchet
  # hatchet:
  #   token: "hatchet_token_xyz"
```

---

## 5. Architectural Recommendation Decision Tree

```mermaid
flowchart TD
    Start{What is your primary requirement?} --> Q1[Do you already have a Postgres or MySQL DB and want ZERO extra infra?]
    Q1 -- YES --> RecDB["💡 Recommended: Database Outbox Queue (River / SKIP LOCKED)"]
    
    Q1 -- NO --> Q2[Do you need sub-millisecond dispatch and high throughput?]
    Q2 -- YES --> RecRedis["💡 Recommended: Redis Streams / Queue (Asynq / Celery / Hangfire)"]
    
    Q2 -- NO --> Q3[Do you need multi-day sagas, human approval gates, and long timeouts?]
    Q3 -- YES --> Q4[Do you want zero cluster maintenance?]
    Q4 -- YES --> RecHatchet["💡 Recommended: Hatchet (Postgres-native) or AWS Step Functions"]
    Q4 -- NO --> RecTemporal["💡 Recommended: Temporal (Self-hosted or Temporal Cloud)"]
```

---

## 6. Implementation Roadmap for OCTOPUS Live Engine

1. **`RedisAsyncActivityDispatcher`**:
   - Implement across Go (`hibiken/asynq`), Python (`redis-py` / `arq`), .NET (`StackExchange.Redis`), and Java (`Redisson`).
2. **`TransactionalSqlAsyncActivityDispatcher`**:
   - Leverage existing `RelationalSqlStateStore` database connections in Postgres, MySQL, and SQL Server to enqueue tasks transactionally.
3. **`HatchetAsyncActivityDispatcher`**:
   - Provide direct integration with Hatchet worker SDKs for teams wanting lightweight, Postgres-native durable workflows.
