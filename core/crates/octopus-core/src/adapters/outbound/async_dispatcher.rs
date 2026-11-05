use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::domain::models::{step_retry_policy, RetryPolicy, WorkflowStep};
use crate::domain::ports::{AsyncCompletion, IActivityExecutor, IAsyncActivityDispatcher};
use crate::json::ValueMap;
use crate::vmap;

#[cfg_attr(not(feature = "temporal"), allow(dead_code))]
pub(crate) const KEY_JOB_ID: &str = "_octopus_job_id";
#[cfg_attr(not(feature = "temporal"), allow(dead_code))]
pub(crate) const KEY_STEP_ID: &str = "_octopus_step_id";
#[cfg_attr(not(feature = "temporal"), allow(dead_code))]
pub(crate) const KEY_MAX_ATTEMPTS: &str = "_octopus_max_attempts";

const DEFAULT_ASYNC_TIMEOUT: Duration = Duration::from_secs(30);

#[cfg_attr(not(feature = "temporal"), allow(dead_code))]
fn default_async_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 5,
        initial_interval: Duration::from_millis(500),
        backoff_coefficient: 2.0,
        maximum_interval: Duration::ZERO,
    }
}

pub(crate) fn step_options(step: Option<&WorkflowStep>) -> (RetryPolicy, Duration) {
    let rp = step_retry_policy(step, default_async_retry());
    let mut timeout = DEFAULT_ASYNC_TIMEOUT;
    if let Some(s) = step {
        if s.timeout_seconds > 0 {
            timeout = Duration::from_secs(s.timeout_seconds as u64);
        }
    }
    (rp, timeout)
}

pub(crate) fn run_with_retry(
    executor: &Arc<dyn IActivityExecutor>,
    name: &str,
    payload: &ValueMap,
    rp: &RetryPolicy,
    timeout: Duration,
) -> (ValueMap, Option<String>, usize) {
    let mut delay = rp.initial_interval;
    let mut res = ValueMap::new();
    let mut last_err: Option<String> = None;
    let mut attempt = 0usize;

    while attempt < rp.max_attempts {
        attempt += 1;
        let exec_res = if timeout.is_zero() {
            executor.execute_activity(name, payload)
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            let exec_clone = Arc::clone(executor);
            let act_name = name.to_string();
            let p_clone = payload.clone();
            crate::workers::spawn(move || {
                let r = exec_clone.execute_activity(&act_name, &p_clone);
                let _ = tx.send(r);
            });
            match rx.recv_timeout(timeout) {
                Ok(r) => r,
                Err(_) => Err(format!("Activity '{}' timed out after {:?}", name, timeout)),
            }
        };

        match exec_res {
            Ok(out) => {
                res = out;
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt >= rp.max_attempts {
                    break;
                }
            }
        }
        std::thread::sleep(delay);
        delay = Duration::from_secs_f64(delay.as_secs_f64() * rp.backoff_coefficient);
        if !rp.maximum_interval.is_zero() && delay > rp.maximum_interval {
            delay = rp.maximum_interval;
        }
    }

    (res, last_err, attempt)
}

pub struct InMemoryAsyncActivityDispatcher {
    hook: RwLock<Option<AsyncCompletion>>,
}

impl InMemoryAsyncActivityDispatcher {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            hook: RwLock::new(None),
        })
    }
}

impl Default for InMemoryAsyncActivityDispatcher {
    fn default() -> Self {
        Self {
            hook: RwLock::new(None),
        }
    }
}

impl IAsyncActivityDispatcher for InMemoryAsyncActivityDispatcher {
    fn dispatch(
        &self,
        activity_name: &str,
        payload: ValueMap,
        job_id: &str,
        step_id: &str,
        executor: Arc<dyn IActivityExecutor>,
    ) {
        let mut step = WorkflowStep::default();
        step.id = step_id.to_string();
        step.activity = activity_name.to_string();
        step.retry_policy = Some(vmap! { "max_attempts" => 1i64 });
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
        let rp = step_retry_policy(
            Some(step),
            RetryPolicy {
                max_attempts: 1,
                initial_interval: Duration::from_millis(500),
                backoff_coefficient: 2.0,
                maximum_interval: Duration::ZERO,
            },
        );
        let (_, timeout) = step_options(Some(step));
        let act_name = step.activity.clone();
        let step_id = step.id.clone();
        let jid = job_id.to_string();
        let hook = self.hook.read().unwrap().clone();

        crate::workers::spawn(move || {
            let (res, err, _) = run_with_retry(&executor, &act_name, &payload, &rp, timeout);
            if let Some(h) = hook {
                h(&jid, &step_id, &act_name, res, err.as_deref());
            }
        });
    }

    fn get_dispatcher_type(&self) -> &str {
        "IN_MEMORY"
    }

    fn set_completion_hook(&self, hook: AsyncCompletion) {
        *self.hook.write().unwrap() = Some(hook);
    }
}
