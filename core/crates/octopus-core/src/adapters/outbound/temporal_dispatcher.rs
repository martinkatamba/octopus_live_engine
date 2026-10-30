//! Durable `async: true` steps on Temporal, through the official Rust SDK.
//!
//! Wire contract shared with the Go engine: each step starts an
//! `IOctopusWorkflow` with `{activity_name, payload, options}` on the task
//! queue, and the workflow runs the activity of that name. Workers register
//! every activity the engine knows, so a Go and a Rust service can serve the
//! same queue.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use temporalio_client::{
    Client, ClientOptions, Connection, ConnectionOptions, Url, WorkflowStartOptions,
};
use temporalio_common::{ActivityDefinition, RetryPolicy as TemporalRetryPolicy};
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::activities::{
    ActivityContext, ActivityDefinitions, ActivityError, ActivityImplementer, ExecutableActivity,
};
use temporalio_sdk::runtime::RuntimeOptions;
use temporalio_sdk::{
    ActivityOptions, ApplicationFailure, Runtime, Worker, WorkerOptions, WorkflowContext,
    WorkflowResult,
};

use crate::adapters::outbound::async_dispatcher::{
    run_with_retry, step_options, KEY_JOB_ID, KEY_MAX_ATTEMPTS, KEY_STEP_ID,
};
use crate::crypto::now_utc;
use crate::domain::models::WorkflowStep;
use crate::domain::ports::{AsyncCompletion, IActivityExecutor, IAsyncActivityDispatcher};
use crate::json::{self, Value, ValueMap};
use crate::vmap;

pub const WORKFLOW_TYPE: &str = "IOctopusWorkflow";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(15);

type Json = serde_json::Value;
type Hook = Arc<RwLock<Option<AsyncCompletion>>>;

fn to_json(map: &ValueMap) -> Json {
    serde_json::from_str(&json::map_to_json(map)).unwrap_or(Json::Null)
}

fn from_json(v: &Json) -> ValueMap {
    match json::parse(&v.to_string()) {
        Ok(Value::Object(m)) => m,
        _ => ValueMap::new(),
    }
}

// ============================================================================
// Workflow: run one named activity with the step's retry policy and timeout
// ============================================================================

/// An activity addressed by its type name, with JSON input and output.
pub struct NamedActivity(String);

impl ActivityDefinition for NamedActivity {
    type Input = Json;
    type Output = Json;
    fn name(&self) -> &str {
        &self.0
    }
}

#[workflow]
#[derive(Default)]
pub struct OctopusActivityWorkflow;

#[workflow_methods]
impl OctopusActivityWorkflow {
    #[run(name = "IOctopusWorkflow")]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Json) -> WorkflowResult<Json> {
        let name = input
            .get("activity_name")
            .or_else(|| input.get("activity"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let payload = input
            .get("payload")
            .cloned()
            .unwrap_or_else(|| input.clone());
        let opt = |k: &str| {
            input
                .get("options")
                .and_then(|o| o.get(k))
                .and_then(|v| v.as_f64())
        };

        let timeout = opt("timeout_seconds")
            .filter(|v| *v > 0.0)
            .map(Duration::from_secs_f64)
            .unwrap_or(Duration::from_secs(30));
        let retry = TemporalRetryPolicy::builder()
            .initial_interval(Duration::from_millis(
                opt("initial_interval_ms")
                    .filter(|v| *v > 0.0)
                    .unwrap_or(500.0) as u64,
            ))
            .backoff_coefficient(
                opt("backoff_coefficient")
                    .filter(|v| *v >= 1.0)
                    .unwrap_or(2.0),
            )
            .maybe_maximum_interval(
                opt("maximum_interval_ms")
                    .filter(|v| *v > 0.0)
                    .map(|ms| Duration::from_millis(ms as u64)),
            )
            .maximum_attempts(opt("max_attempts").filter(|v| *v > 0.0).unwrap_or(5.0) as u32)
            .build();
        let options = ActivityOptions::with_start_to_close_timeout(timeout)
            .retry_policy(retry)
            .build();

        Ok(ctx
            .execute_activity(NamedActivity(name), payload, options)
            .await?)
    }
}

// ============================================================================
// Activities: every engine activity, registered under its own name
// ============================================================================

thread_local! {
    /// Name handed to `EngineActivity::definition()` while registering; the
    /// SDK keys registrations by that name.
    static REGISTERING: RefCell<String> = const { RefCell::new(String::new()) };
}

pub struct EngineActivity {
    name: String,
}

impl ActivityDefinition for EngineActivity {
    type Input = Json;
    type Output = Json;
    fn name(&self) -> &str {
        &self.name
    }
}

pub struct EngineActivities {
    executor: Arc<dyn IActivityExecutor>,
    hook: Hook,
}

impl ActivityImplementer for EngineActivities {
    fn register_all(self: Arc<Self>, defs: &mut ActivityDefinitions) {
        for name in self.executor.list_activities() {
            REGISTERING.with(|r| *r.borrow_mut() = name);
            defs.register_activity::<EngineActivity>(Arc::clone(&self));
        }
    }
}

impl ExecutableActivity for EngineActivity {
    type Implementer = EngineActivities;
    const REQUIRES_INSTANCE: bool = true;

    fn definition() -> Self {
        EngineActivity {
            name: REGISTERING.with(|r| r.borrow().clone()),
        }
    }

    fn execute(
        receiver: Option<Arc<EngineActivities>>,
        ctx: ActivityContext,
        input: Json,
    ) -> Pin<Box<dyn Future<Output = Result<Json, ActivityError>> + Send + 'static>> {
        Box::pin(async move {
            let acts =
                receiver.ok_or_else(|| failure("activity worker has no executor".to_string()))?;
            let name = ctx.info().activity_type.clone();
            let attempt = ctx.info().attempt as i64;

            let mut payload = from_json(&input);
            let job_id = payload
                .get(KEY_JOB_ID)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            let step_id = payload
                .get(KEY_STEP_ID)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            let max_attempts = payload
                .get(KEY_MAX_ATTEMPTS)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            payload.retain(|k, _| !k.starts_with("_octopus_"));

            // Activities and the completion hook are blocking code (they may
            // hit a SQL store), so they run on a plain thread, never on tokio.
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let res = acts.executor.execute_activity(&name, &payload);
                let last_attempt = max_attempts > 0 && attempt >= max_attempts;
                if res.is_ok() || last_attempt {
                    if let Some(hook) = acts.hook.read().unwrap().clone() {
                        let (out, err) = match &res {
                            Ok(out) => (out.clone(), None),
                            Err(e) => (ValueMap::new(), Some(e.as_str())),
                        };
                        if !job_id.is_empty() {
                            hook(&job_id, &step_id, &name, out, err);
                        }
                    }
                }
                let _ = tx.send(res);
            });
            match rx.await {
                Ok(Ok(out)) => Ok(to_json(&out)),
                Ok(Err(e)) => Err(failure(e)),
                Err(_) => Err(failure(
                    "activity thread ended without a result".to_string(),
                )),
            }
        })
    }
}

fn failure(msg: String) -> ActivityError {
    ApplicationFailure::new(anyhow::anyhow!(msg)).into()
}

// ============================================================================
// Dispatcher
// ============================================================================

struct WorkerHandle {
    shutdown: Box<dyn Fn() + Send>,
    thread: JoinHandle<()>,
}

pub struct TemporalAsyncActivityDispatcher {
    pub host: String,
    pub namespace: String,
    pub task_queue: String,
    pub fallback_in_memory: RwLock<bool>,
    pub redial_after: Duration,
    pub max_history: RwLock<usize>,

    rt: tokio::runtime::Runtime,
    client: Arc<tokio::sync::Mutex<Option<Client>>>,
    unavailable_until: Arc<Mutex<Option<Instant>>>,
    hook: Hook,
    tasks_history: Arc<Mutex<Vec<ValueMap>>>,
    worker: Mutex<Option<WorkerHandle>>,
}

async fn connect(host: &str, namespace: &str) -> Result<Client, String> {
    let target = if host.contains("://") {
        host.to_string()
    } else {
        format!("http://{}", host)
    };
    let url =
        Url::from_str(&target).map_err(|e| format!("invalid Temporal host '{}': {}", host, e))?;
    let conn = tokio::time::timeout(
        CONNECT_TIMEOUT,
        Connection::connect(ConnectionOptions::new(url).build()),
    )
    .await
    .map_err(|_| format!("timed out connecting to Temporal at {}", host))?
    .map_err(|e| format!("failed to dial Temporal server at {}: {}", host, e))?;
    Client::new(conn, ClientOptions::new(namespace.to_string()).build()).map_err(|e| e.to_string())
}

impl TemporalAsyncActivityDispatcher {
    pub fn new(host: &str, namespace: &str, task_queue: &str) -> Arc<Self> {
        let or = |v: &str, d: &str| {
            if v.is_empty() {
                d.to_string()
            } else {
                v.to_string()
            }
        };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("octopus-temporal")
            .enable_all()
            .build()
            .expect("build tokio runtime for Temporal");
        Arc::new(Self {
            host: or(host, "localhost:7233"),
            namespace: or(namespace, "default"),
            task_queue: or(task_queue, "octopus-temporal-queue"),
            fallback_in_memory: RwLock::new(true),
            redial_after: Duration::from_secs(30),
            max_history: RwLock::new(500),
            rt,
            client: Arc::new(tokio::sync::Mutex::new(None)),
            unavailable_until: Arc::new(Mutex::new(None)),
            hook: Arc::new(RwLock::new(None)),
            tasks_history: Arc::new(Mutex::new(Vec::new())),
            worker: Mutex::new(None),
        })
    }

    pub fn set_fallback_in_memory(&self, val: bool) {
        *self.fallback_in_memory.write().unwrap() = val;
    }

    pub fn set_max_history(&self, max: usize) {
        *self.max_history.write().unwrap() = max;
    }

    /// Starts an embedded worker polling the task queue for this engine's
    /// activities and `IOctopusWorkflow`. Fails if Temporal is unreachable.
    pub fn start_worker(&self, executor: Arc<dyn IActivityExecutor>) -> Result<(), String> {
        let mut slot = self.worker.lock().unwrap();
        if slot.is_some() {
            return Ok(());
        }
        let (ready_tx, ready_rx) =
            std::sync::mpsc::channel::<Result<Box<dyn Fn() + Send>, String>>();
        let handle = self.rt.handle().clone();
        let (host, ns, tq) = (
            self.host.clone(),
            self.namespace.clone(),
            self.task_queue.clone(),
        );
        let hook = Arc::clone(&self.hook);

        let thread = std::thread::Builder::new()
            .name("octopus-temporal-worker".to_string())
            .spawn(move || {
                handle.block_on(async move {
                    let built = async {
                        let runtime = Runtime::from_current_tokio(RuntimeOptions::default())
                            .map_err(|e| e.to_string())?;
                        let client = connect(&host, &ns).await?;
                        let options = WorkerOptions::new(tq.clone())
                            .register_workflow::<OctopusActivityWorkflow>()
                            .map_err(|e| e.to_string())?
                            .register_activities(EngineActivities { executor, hook })
                            .build();
                        let worker =
                            Worker::new(&runtime, client, options).map_err(|e| e.to_string())?;
                        Ok::<_, String>((runtime, worker))
                    }
                    .await;
                    let (_runtime, mut worker) = match built {
                        Ok(w) => w,
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    let _ = ready_tx.send(Ok(Box::new(worker.shutdown_handle())));
                    eprintln!(
                        "👷 [TEMPORAL WORKER] polling task queue '{}' (host {}, namespace {})",
                        tq, host, ns
                    );
                    if let Err(e) = worker.run().await {
                        eprintln!("⚠️ [TEMPORAL WORKER] stopped with error: {}", e);
                    }
                })
            })
            .map_err(|e| e.to_string())?;

        match ready_rx.recv_timeout(WORKER_START_TIMEOUT) {
            Ok(Ok(shutdown)) => {
                *slot = Some(WorkerHandle { shutdown, thread });
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(format!("cannot start Temporal worker: {}", e))
            }
            Err(_) => Err("timed out starting the Temporal worker".to_string()),
        }
    }

    /// Stops polling and waits for in-flight tasks to finish.
    pub fn stop_worker(&self) {
        let Some(w) = self.worker.lock().unwrap().take() else {
            return;
        };
        {
            // Shutdown spawns a tokio task to deregister the worker.
            let _rt = self.rt.enter();
            (w.shutdown)();
        }
        let _ = w.thread.join();
        eprintln!("🛑 [TEMPORAL WORKER] stopped");
    }

    pub fn is_worker_started(&self) -> bool {
        self.worker.lock().unwrap().is_some()
    }

    fn record(&self, rec: ValueMap) {
        let max = match *self.max_history.read().unwrap() {
            0 => 500,
            m => m,
        };
        let mut hist = self.tasks_history.lock().unwrap();
        hist.push(rec);
        if hist.len() > max {
            let excess = hist.len() - max;
            hist.drain(0..excess);
        }
    }
}

fn set_status(history: &Mutex<Vec<ValueMap>>, workflow_id: &str, status: &str) {
    let mut hist = history.lock().unwrap();
    if let Some(rec) = hist
        .iter_mut()
        .rev()
        .find(|r| r.get("temporal_workflow_id").and_then(|v| v.as_str()) == Some(workflow_id))
    {
        rec.insert("status".to_string(), Value::String(status.to_string()));
    }
}

impl IAsyncActivityDispatcher for TemporalAsyncActivityDispatcher {
    fn dispatch(
        &self,
        activity_name: &str,
        payload: ValueMap,
        job_id: &str,
        step_id: &str,
        executor: Arc<dyn IActivityExecutor>,
    ) {
        let step = WorkflowStep {
            id: step_id.to_string(),
            activity: activity_name.to_string(),
            ..Default::default()
        };
        self.dispatch_step(&step, payload, job_id, executor);
    }

    fn dispatch_step(
        &self,
        step: &WorkflowStep,
        payload: ValueMap,
        job_id: &str,
        executor: Arc<dyn IActivityExecutor>,
    ) {
        if step.activity.is_empty() {
            return;
        }
        let activity_name = step.activity.clone();
        let step_id = step.id.clone();
        let (rp, timeout) = step_options(Some(step));
        let workflow_id = format!("octopus_act_{}_{}_{}", job_id, step_id, activity_name);

        self.record(vmap! {
            "temporal_workflow_id" => workflow_id.clone(),
            "task_queue" => self.task_queue.clone(),
            "activity_name" => activity_name.clone(),
            "job_id" => job_id.to_string(),
            "step_id" => step_id.clone(),
            "max_attempts" => rp.max_attempts as i64,
            "scheduled_at" => now_utc(),
            "status" => "SCHEDULED",
        });

        let mut wf_payload = payload.clone();
        wf_payload.insert(
            "temporal_workflow_id".to_string(),
            Value::String(workflow_id.clone()),
        );
        wf_payload.insert(
            "temporal_task_queue".to_string(),
            Value::String(self.task_queue.clone()),
        );
        wf_payload.insert(KEY_JOB_ID.to_string(), Value::String(job_id.to_string()));
        wf_payload.insert(KEY_STEP_ID.to_string(), Value::String(step_id.clone()));
        wf_payload.insert(
            KEY_MAX_ATTEMPTS.to_string(),
            Value::Int(rp.max_attempts as i64),
        );
        let input = serde_json::json!({
            "activity_name": activity_name,
            "payload": to_json(&wf_payload),
            "options": {
                "timeout_seconds": timeout.as_secs_f64(),
                "max_attempts": rp.max_attempts,
                "initial_interval_ms": rp.initial_interval.as_millis() as u64,
                "backoff_coefficient": rp.backoff_coefficient,
                "maximum_interval_ms": rp.maximum_interval.as_millis() as u64,
            },
        });

        let fallback = *self.fallback_in_memory.read().unwrap();
        let (client_slot, until, redial) = (
            Arc::clone(&self.client),
            Arc::clone(&self.unavailable_until),
            self.redial_after,
        );
        let (host, ns, tq) = (
            self.host.clone(),
            self.namespace.clone(),
            self.task_queue.clone(),
        );
        let (history, hook) = (Arc::clone(&self.tasks_history), Arc::clone(&self.hook));
        let jid = job_id.to_string();

        // Start the workflow off the caller's thread; the request doesn't wait on Temporal.
        self.rt.spawn(async move {
            let client = {
                let mut slot = client_slot.lock().await;
                match slot.as_ref() {
                    Some(c) => Ok(c.clone()),
                    None => {
                        let blocked = until.lock().unwrap().is_some_and(|t| Instant::now() < t);
                        if blocked {
                            Err(format!("temporal server at {} unavailable", host))
                        } else {
                            match connect(&host, &ns).await {
                                Ok(c) => {
                                    *slot = Some(c.clone());
                                    Ok(c)
                                }
                                Err(e) => {
                                    *until.lock().unwrap() = Some(Instant::now() + redial);
                                    Err(e)
                                }
                            }
                        }
                    }
                }
            };
            let started = match client {
                Ok(c) => c
                    .start_workflow(
                        OctopusActivityWorkflow::run,
                        input,
                        WorkflowStartOptions::new(tq, workflow_id.clone()).build(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            let err = match started {
                Ok(()) => {
                    set_status(&history, &workflow_id, "DISPATCHED_TO_TEMPORAL");
                    return;
                }
                Err(e) => e,
            };

            let hook = hook.read().unwrap().clone();
            if !fallback {
                eprintln!(
                    "❌ [TEMPORAL DISPATCH] activity '{}' (job={}) not scheduled: {}",
                    activity_name, jid, err
                );
                set_status(&history, &workflow_id, "FAILED");
                if let Some(h) = hook {
                    std::thread::spawn(move || {
                        h(&jid, &step_id, &activity_name, ValueMap::new(), Some(&err));
                    });
                }
                return;
            }
            eprintln!(
                "⚡ [TEMPORAL DISPATCH] activity '{}' (job={}) running in-process: {}",
                activity_name, jid, err
            );
            set_status(&history, &workflow_id, "FALLBACK_IN_MEMORY");
            std::thread::spawn(move || {
                let (res, run_err, _) =
                    run_with_retry(&executor, &activity_name, &payload, &rp, timeout);
                if let Some(h) = hook {
                    h(&jid, &step_id, &activity_name, res, run_err.as_deref());
                }
            });
        });
    }

    fn get_dispatcher_type(&self) -> &str {
        "TEMPORAL"
    }

    fn set_completion_hook(&self, hook: AsyncCompletion) {
        *self.hook.write().unwrap() = Some(hook);
    }

    fn get_dispatched_tasks_history(&self) -> Vec<ValueMap> {
        self.tasks_history.lock().unwrap().clone()
    }
}

impl Drop for TemporalAsyncActivityDispatcher {
    fn drop(&mut self) {
        self.stop_worker();
    }
}
