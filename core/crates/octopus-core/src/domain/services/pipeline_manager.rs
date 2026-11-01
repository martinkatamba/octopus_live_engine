use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::crypto::{new_job_id, now_utc};
use crate::domain::models::{
    step_retry_policy, to_int, APIEndpointConfig, CompensationRecord, EngineOptions,
    ExecutionState, ExecutionStatus, LiveEvent, MiddlewareExceptionConfig, RetryPolicy,
    StepExecutionTrace, Suspension, WorkflowBlueprint, WorkflowStep,
};
use crate::domain::ports::{
    IActivityExecutor, IAsyncActivityDispatcher, IBlueprintRepository, IStateStore,
    ISuspensionStore, IWorkflowEngine,
};
use crate::domain::services::condition_evaluator::ConditionEvaluator;
use crate::domain::services::idempotency_manager::IdempotencyManager;
use crate::domain::services::masking_service::{copy_map_recursive, mask_payload};
use crate::domain::services::middleware::OctopusMiddlewareError;
use crate::domain::services::saga_manager::{SagaAction, SagaManager};
use crate::json::{Value, ValueMap};
use crate::vmap;

pub const ERR_NO_SUSPENSION: &str = "no workflow is awaiting a signal for this job";
pub const ERR_SIGNAL_MISMATCH: &str = "the workflow is waiting for a different signal";

#[derive(Clone)]
struct AsyncOutcome {
    result: ValueMap,
    err: Option<String>,
    at: String,
}

pub struct MemorySuspensions {
    by_id: Mutex<HashMap<String, Suspension>>,
}

impl MemorySuspensions {
    pub fn new() -> Self {
        Self {
            by_id: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for MemorySuspensions {
    fn default() -> Self {
        Self::new()
    }
}

impl ISuspensionStore for MemorySuspensions {
    fn save(&self, s: Suspension) -> Result<(), String> {
        self.by_id.lock().unwrap().insert(s.job_id.clone(), s);
        Ok(())
    }

    fn take(&self, job_id: &str) -> Result<Suspension, String> {
        self.by_id
            .lock()
            .unwrap()
            .remove(job_id)
            .ok_or_else(|| ERR_NO_SUSPENSION.to_string())
    }

    fn list(&self) -> Result<Vec<Suspension>, String> {
        Ok(self.by_id.lock().unwrap().values().cloned().collect())
    }
}

pub struct PipelineManager {
    blueprint_repo: Arc<dyn IBlueprintRepository>,
    activity_executor: Arc<dyn IActivityExecutor>,
    state_store: RwLock<Arc<dyn IStateStore>>,
    async_dispatcher: RwLock<Option<Arc<dyn IAsyncActivityDispatcher>>>,
    condition_evaluator: ConditionEvaluator,
    saga_manager: SagaManager,
    idempotency_manager: IdempotencyManager,
    options: RwLock<EngineOptions>,
    suspensions: RwLock<Arc<dyn ISuspensionStore>>,
    timers: Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    save_mu: Mutex<()>,
    outcomes_mu: RwLock<(HashMap<String, AsyncOutcome>, Vec<String>)>,
    self_weak: RwLock<std::sync::Weak<PipelineManager>>,
}

impl PipelineManager {
    pub fn new(
        repo: Arc<dyn IBlueprintRepository>,
        executor: Arc<dyn IActivityExecutor>,
        store: Arc<dyn IStateStore>,
    ) -> Arc<Self> {
        Self::new_with_dispatcher(repo, executor, store, None)
    }

    pub fn new_with_dispatcher(
        repo: Arc<dyn IBlueprintRepository>,
        executor: Arc<dyn IActivityExecutor>,
        store: Arc<dyn IStateStore>,
        dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>,
    ) -> Arc<Self> {
        let saga = SagaManager::new(Arc::clone(&executor));
        let p = Arc::new(Self {
            blueprint_repo: repo,
            activity_executor: executor,
            state_store: RwLock::new(store),
            async_dispatcher: RwLock::new(None),
            condition_evaluator: ConditionEvaluator::new(),
            saga_manager: saga,
            idempotency_manager: IdempotencyManager::new(10000, Duration::from_secs(3600)),
            options: RwLock::new(EngineOptions::default()),
            suspensions: RwLock::new(Arc::new(MemorySuspensions::new())),
            timers: Mutex::new(HashMap::new()),
            save_mu: Mutex::new(()),
            outcomes_mu: RwLock::new((HashMap::new(), Vec::new())),
            self_weak: RwLock::new(std::sync::Weak::new()),
        });
        *p.self_weak.write().unwrap() = Arc::downgrade(&p);
        if let Some(d) = dispatcher {
            p.set_async_dispatcher(Some(d));
        }
        p
    }

    fn clone_handle(&self) -> Arc<Self> {
        self.self_weak
            .read()
            .unwrap()
            .upgrade()
            .expect("PipelineManager must be constructed via PipelineManager::new*")
    }

    pub fn set_async_dispatcher(&self, dispatcher: Option<Arc<dyn IAsyncActivityDispatcher>>) {
        if let Some(ref d) = dispatcher {
            let weak = self.self_weak.read().unwrap().clone();
            d.set_completion_hook(Arc::new(move |job_id, step_id, activity, result, err| {
                if let Some(pm) = weak.upgrade() {
                    pm.on_async_done(job_id, step_id, activity, result, err);
                }
            }));
        }
        *self.async_dispatcher.write().unwrap() = dispatcher;
    }

    pub fn set_options(&self, opts: EngineOptions) {
        *self.options.write().unwrap() = opts;
    }

    pub fn set_state_store(&self, store: Arc<dyn IStateStore>) {
        *self.state_store.write().unwrap() = store;
    }

    pub fn set_suspension_store(&self, store: Arc<dyn ISuspensionStore>) {
        *self.suspensions.write().unwrap() = store;
    }

    pub fn restore_suspensions(&self) -> Result<(), String> {
        let list = self.suspensions.read().unwrap().list()?;
        for susp in list {
            self.schedule_timeout(&susp);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn run_steps(
        &self,
        bp: &WorkflowBlueprint,
        mut state: ExecutionState,
        mut steps: Vec<WorkflowStep>,
        mut step_index: usize,
        mut saga_actions: Vec<SagaAction>,
        pipeline_start_time: Instant,
        idem_key: &str,
    ) -> (ExecutionState, Option<String>) {
        let job_id = state.job_id.clone();

        while step_index < steps.len() {
            let step = steps[step_index].clone();
            state.current_index = step_index;
            state.current_step_id = step.id.clone();

            // Pause until a signal arrives (or the step's timeout passes).
            let sig = step.signal_name();
            if !sig.is_empty() {
                let rest = steps[step_index + 1..].to_vec();
                return self.suspend(bp, state, &step, rest, saga_actions, idem_key);
            }

            // Pre-execution delay
            if step.delay_seconds > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(step.delay_seconds));
            }

            // Early termination / settlement
            if step.terminate || step.settle {
                let reason = if step.reason.is_empty() {
                    "Workflow settled gracefully".to_string()
                } else {
                    step.reason.clone()
                };
                let out_payload = self.mask_step_payload(
                    &vmap! {
                        "settled" => true,
                        "reason" => reason,
                    },
                    Some(bp),
                    Some(&step),
                );
                let now = now_utc();
                let trace = StepExecutionTrace {
                    step_id: step.id.clone(),
                    activity_name: "terminate_settle".to_string(),
                    status: "COMPLETED".to_string(),
                    is_async: false,
                    input_payload: ValueMap::new(),
                    output_payload: out_payload,
                    error_message: String::new(),
                    attempts: 0,
                    started_at: now.clone(),
                    finished_at: now,
                    duration_ms: 0.0,
                };
                state.traces.push(trace);
                state.status = ExecutionStatus::Completed;
                break;
            }

            // Declarative compensation trigger
            if step.compensate || step.rollback || step.trigger_compensation {
                let reason = if step.reason.is_empty() {
                    format!("Declarative compensation triggered at step '{}'", step.id)
                } else {
                    step.reason.clone()
                };
                state.error = Some(reason.clone());
                state.status = ExecutionStatus::RolledBack;

                if bp.compensation_mode == "async" {
                    let actions_copy = saga_actions.clone();
                    let saga_mgr = SagaManager::new(Arc::clone(&self.activity_executor));
                    crate::workers::spawn(move || {
                        let _ = saga_mgr.execute_compensations(&actions_copy);
                    });
                } else {
                    let (output, _) = self.saga_manager.run_compensations(&saga_actions);
                    state.data.extend(output);
                }

                let now = now_utc();
                state.finished_at = Some(now);
                state.duration_ms = (pipeline_start_time.elapsed().as_micros() as f64) / 1000.0;
                let _ = self.save(&state, bp);
                self.emit(&job_id, "workflow_rolled_back", || {
                    self.snapshot(&state, bp).to_value()
                });
                return (state, Some(reason));
            }

            // Evaluate branching condition if present
            let has_branches = !step.get_then_steps().is_empty()
                || !step.get_else_steps().is_empty()
                || step.condition.is_some()
                || step.conditions.is_some()
                || !step.branching_step.is_empty();
            if has_branches {
                let eval_result = self.condition_evaluator.evaluate_step(&step, &state.data);
                let (branch_steps, mut branch) = if eval_result {
                    (step.get_then_steps().to_vec(), "then")
                } else {
                    (step.get_else_steps().to_vec(), "else")
                };
                if branch_steps.is_empty() {
                    branch = "skip";
                }
                let runs_work =
                    !step.activity.is_empty() || step.r#loop.is_some() || step.for_each.is_some();
                let skipped = runs_work && !eval_result;
                if !runs_work || skipped {
                    let now = now_utc();
                    let mut output_payload = vmap! {
                        "result" => eval_result,
                        "branch" => branch,
                    };
                    if skipped {
                        output_payload.insert("skipped".to_string(), Value::Bool(true));
                    }
                    state.traces.push(StepExecutionTrace {
                        step_id: step.id.clone(),
                        activity_name: "condition".to_string(),
                        status: "COMPLETED".to_string(),
                        is_async: false,
                        input_payload: self.mask_step_payload(
                            &self
                                .condition_evaluator
                                .condition_fields(&step, &state.data),
                            Some(bp),
                            Some(&step),
                        ),
                        output_payload,
                        error_message: String::new(),
                        attempts: 0,
                        started_at: now.clone(),
                        finished_at: now,
                        duration_ms: 0.0,
                    });
                }

                if !branch_steps.is_empty() {
                    let mut new_steps = steps[..=step_index].to_vec();
                    new_steps.extend(branch_steps);
                    new_steps.extend_from_slice(&steps[step_index + 1..]);
                    steps = new_steps;
                    state.total_steps = steps.len();
                }

                // A false condition gates the step's own work (decision D1).
                if !runs_work || skipped {
                    step_index += 1;
                    continue;
                }
            }

            // Loop execution
            if step.r#loop.is_some() || step.for_each.is_some() {
                if let Err(loop_err) = self.execute_loop_step(&step, &mut state, bp) {
                    return self.abort(state, bp, &saga_actions, &loop_err, pipeline_start_time);
                }
                step_index += 1;
                continue;
            }

            // Activity Execution (Sync or Async)
            if !step.activity.is_empty() {
                if step.r#async || step.is_async {
                    self.dispatch_async_activity(&step, &mut state, bp);
                } else {
                    self.emit(&job_id, "step_started", || {
                        Value::Object(vmap! {
                            "step_id" => step.id.clone(),
                            "activity" => step.activity.clone(),
                            "index" => step_index as i64,
                            "total_steps" => state.total_steps as i64,
                        })
                    });

                    let start_t = Instant::now();
                    let started_at_str = now_utc();
                    let (out_payload, act_err, attempts) =
                        self.run_activity(&step, &step.activity, &state.data);
                    let duration = (start_t.elapsed().as_micros() as f64) / 1000.0;

                    let mut trace = StepExecutionTrace {
                        step_id: step.id.clone(),
                        activity_name: step.activity.clone(),
                        status: String::new(),
                        is_async: false,
                        attempts,
                        input_payload: self.mask_step_payload(&state.data, Some(bp), Some(&step)),
                        output_payload: self.mask_step_payload(&out_payload, Some(bp), Some(&step)),
                        error_message: String::new(),
                        started_at: started_at_str,
                        finished_at: now_utc(),
                        duration_ms: duration,
                    };

                    if let Some(err_str) = act_err {
                        let mw_err = OctopusMiddlewareError::parse_from_error_str(&err_str);
                        let clean_err_msg = mw_err
                            .as_ref()
                            .map(|e| e.message.clone())
                            .unwrap_or_else(|| err_str.clone());
                        trace.status = "FAILED".to_string();
                        trace.error_message = clean_err_msg.clone();
                        state.traces.push(trace);

                        if let Some(exc_cfg) = self.find_exception_config(
                            Some(&step),
                            Some(bp),
                            &err_str,
                            mw_err.as_ref(),
                        ) {
                            let msg = if exc_cfg.message.is_empty() {
                                clean_err_msg.clone()
                            } else {
                                exc_cfg.message.clone()
                            };
                            let code = if exc_cfg.code == 0 { 400 } else { exc_cfg.code };
                            state.error = Some(msg.clone());
                            state.error_code = Some(code);
                            state.error_type = Some(exc_cfg.exception.clone());

                            if !exc_cfg.activity.is_empty() {
                                let snap_data = self.snapshot(&state, bp).data;
                                let act_payload = vmap! {
                                    "exception" => exc_cfg.exception.clone(),
                                    "error" => exc_cfg.exception.clone(),
                                    "error_type" => exc_cfg.exception.clone(),
                                    "error_message" => clean_err_msg.clone(),
                                    "message" => msg,
                                    "status_code" => code as i64,
                                    "job_id" => job_id.clone(),
                                    "workflow_name" => bp.workflow_name.clone(),
                                    "failing_step_id" => step.id.clone(),
                                    "route" => state.route.clone(),
                                    "path" => state.route.clone(),
                                    "method" => state.method.clone(),
                                    "client_host" => "127.0.0.1",
                                    "timestamp" => now_utc(),
                                    "data" => Value::Object(snap_data),
                                };

                                if exc_cfg.get_is_async() {
                                    let disp_opt = self.async_dispatcher.read().unwrap().clone();
                                    let dispatcher_type = disp_opt
                                        .as_ref()
                                        .map(|d| d.get_dispatcher_type().to_string())
                                        .unwrap_or_else(|| "IN_MEMORY".to_string());
                                    let now = now_utc();
                                    let exc_step_id =
                                        format!("exc_{}", exc_cfg.activity.to_lowercase());
                                    state.traces.push(StepExecutionTrace {
                                        step_id: exc_step_id.clone(),
                                        activity_name: exc_cfg.activity.clone(),
                                        status: "ASYNC_DISPATCHED".to_string(),
                                        is_async: true,
                                        input_payload: self.mask_step_payload(
                                            &act_payload,
                                            Some(bp),
                                            Some(&step),
                                        ),
                                        output_payload: self.mask_step_payload(
                                            &vmap! {
                                                "async_dispatched" => true,
                                                "dispatcher" => dispatcher_type,
                                            },
                                            Some(bp),
                                            Some(&step),
                                        ),
                                        error_message: String::new(),
                                        attempts: 0,
                                        started_at: now.clone(),
                                        finished_at: now,
                                        duration_ms: 0.0,
                                    });

                                    if let Some(disp) = disp_opt {
                                        disp.dispatch(
                                            &exc_cfg.activity,
                                            act_payload,
                                            &job_id,
                                            &exc_step_id,
                                            Arc::clone(&self.activity_executor),
                                        );
                                    } else {
                                        let exec_clone = Arc::clone(&self.activity_executor);
                                        let act_name = exc_cfg.activity.clone();
                                        crate::workers::spawn(move || {
                                            let _ = exec_clone
                                                .execute_activity(&act_name, &act_payload);
                                        });
                                    }
                                } else {
                                    let act_start = Instant::now();
                                    if let Ok(act_res) = self
                                        .activity_executor
                                        .execute_activity(&exc_cfg.activity, &act_payload)
                                    {
                                        let dur_ms =
                                            (act_start.elapsed().as_micros() as f64) / 1000.0;
                                        let now = now_utc();
                                        state.traces.push(StepExecutionTrace {
                                            step_id: format!(
                                                "exc_{}",
                                                exc_cfg.activity.to_lowercase()
                                            ),
                                            activity_name: exc_cfg.activity.clone(),
                                            status: "COMPLETED".to_string(),
                                            is_async: false,
                                            input_payload: self.mask_step_payload(
                                                &act_payload,
                                                Some(bp),
                                                Some(&step),
                                            ),
                                            output_payload: self.mask_step_payload(
                                                &act_res,
                                                Some(bp),
                                                Some(&step),
                                            ),
                                            error_message: String::new(),
                                            attempts: 1,
                                            duration_ms: dur_ms,
                                            started_at: now.clone(),
                                            finished_at: now,
                                        });
                                    }
                                }
                            }
                        } else {
                            state.error = Some(clean_err_msg.clone());
                            let mut default_code = 500;
                            if let Some(mwe) = mw_err {
                                default_code = mwe.code;
                                state.error_type = Some(mwe.name);
                            }
                            state.error_code = Some(default_code);
                        }

                        if !saga_actions.is_empty() {
                            if bp.compensation_mode == "async" {
                                let disp_opt = self.async_dispatcher.read().unwrap().clone();
                                for action in &saga_actions {
                                    if let Some(ref disp) = disp_opt {
                                        disp.dispatch(
                                            &action.compensation,
                                            action.payload.clone(),
                                            &job_id,
                                            &format!("saga_{}", action.step_id),
                                            Arc::clone(&self.activity_executor),
                                        );
                                    } else {
                                        let exec_clone = Arc::clone(&self.activity_executor);
                                        let comp_name = action.compensation.clone();
                                        let comp_payload = action.payload.clone();
                                        crate::workers::spawn(move || {
                                            let _ = exec_clone
                                                .execute_activity(&comp_name, &comp_payload);
                                        });
                                    }
                                }
                            } else {
                                let (output, _) =
                                    self.saga_manager.run_compensations(&saga_actions);
                                state.data.extend(output);
                            }
                            state.status = ExecutionStatus::RolledBack;
                        } else {
                            state.status = ExecutionStatus::Failed;
                        }

                        let now = now_utc();
                        state.finished_at = Some(now);
                        state.duration_ms =
                            (pipeline_start_time.elapsed().as_micros() as f64) / 1000.0;
                        let _ = self.save(&state, bp);
                        self.emit(&job_id, "workflow_failed", || {
                            self.snapshot(&state, bp).to_value()
                        });
                        return (state, Some(clean_err_msg));
                    }

                    trace.status = "COMPLETED".to_string();
                    state.traces.push(trace);

                    for (k, v) in &out_payload {
                        state.data.insert(k.clone(), v.clone());
                    }

                    if step.is_output || step.output || state.output_data.is_none() {
                        state.output_data = Some(out_payload.clone());
                    }

                    if !step.compensation.is_empty() {
                        saga_actions.push(SagaAction {
                            step_id: step.id.clone(),
                            compensation: step.compensation.clone(),
                            payload: state.data.clone(),
                        });
                    }

                    self.emit(&job_id, "step_completed", || {
                        state
                            .traces
                            .last()
                            .map(|t| t.to_value())
                            .unwrap_or_default()
                    });
                }
            }

            // Pivot: clears earlier saga compensations
            if step.pivot && !saga_actions.is_empty() {
                saga_actions.clear();
                let now = now_utc();
                state.traces.push(StepExecutionTrace {
                    step_id: format!("{}.pivot", step.id),
                    activity_name: "pivot".to_string(),
                    status: "COMPLETED".to_string(),
                    is_async: false,
                    input_payload: ValueMap::new(),
                    output_payload: vmap! { "compensations_cleared" => true },
                    error_message: String::new(),
                    attempts: 0,
                    started_at: now.clone(),
                    finished_at: now,
                    duration_ms: 0.0,
                });
            }

            // Jump by step index
            if let Some(idx) = step.jump_to_step_index {
                if idx < steps.len() {
                    step_index = idx;
                    continue;
                }
            }
            if let Some(idx) = step.rewind_to_step_index {
                if idx < steps.len() {
                    step_index = idx;
                    continue;
                }
            }

            // Named jump_to
            if !step.jump_to.is_empty() {
                if let Some(target_idx) = steps.iter().position(|s| s.id == step.jump_to) {
                    step_index = target_idx;
                    continue;
                }
            }

            step_index += 1;
        }

        if state.status == ExecutionStatus::Running {
            state.status = ExecutionStatus::Completed;
        }
        if state.output_data.is_none() {
            state.output_data = Some(state.data.clone());
        }
        let now = now_utc();
        state.finished_at = Some(now);
        state.duration_ms = (pipeline_start_time.elapsed().as_micros() as f64) / 1000.0;
        self.idempotency_manager.set(idem_key, replay_copy(&state));
        let _ = self.save(&state, bp);

        self.emit(&job_id, "workflow_completed", || {
            self.snapshot(&state, bp).to_value()
        });

        (state, None)
    }

    fn dispatch_async_activity(
        &self,
        step: &WorkflowStep,
        state: &mut ExecutionState,
        bp: &WorkflowBlueprint,
    ) {
        let disp_opt = self.async_dispatcher.read().unwrap().clone();
        let dispatcher_type = disp_opt
            .as_ref()
            .map(|d| d.get_dispatcher_type().to_string())
            .unwrap_or_else(|| "IN_MEMORY".to_string());

        let data_copy = self.async_payload(step, &state.data, bp);
        let now = now_utc();
        let trace = StepExecutionTrace {
            step_id: step.id.clone(),
            activity_name: step.activity.clone(),
            status: "ASYNC_DISPATCHED".to_string(),
            is_async: true,
            input_payload: mask_payload(&data_copy, &[], ""),
            output_payload: self.mask_step_payload(
                &vmap! {
                    "async_dispatched" => true,
                    "dispatcher" => dispatcher_type,
                },
                Some(bp),
                Some(step),
            ),
            error_message: String::new(),
            attempts: 0,
            started_at: now.clone(),
            finished_at: now,
            duration_ms: 0.0,
        };
        state.traces.push(trace);

        if let Some(disp) = disp_opt {
            disp.dispatch_step(
                step,
                data_copy,
                &state.job_id,
                Arc::clone(&self.activity_executor),
            );
        } else {
            let pm = self.clone_handle();
            let job_id = state.job_id.clone();
            let step_copy = step.clone();
            let act_name = step.activity.clone();
            crate::workers::spawn(move || {
                let (out, err, _) = pm.run_activity(&step_copy, &act_name, &data_copy);
                pm.on_async_done(&job_id, &step_copy.id, &act_name, out, err.as_deref());
            });
        }
    }

    fn execute_loop_step(
        &self,
        step: &WorkflowStep,
        state: &mut ExecutionState,
        bp: &WorkflowBlueprint,
    ) -> Result<(), String> {
        let loop_cfg = step.r#loop.as_ref().or(step.for_each.as_ref());
        let Some(loop_cfg) = loop_cfg else {
            return Ok(());
        };

        let mut items: Vec<Value> = Vec::new();
        let mut items_field = loop_cfg
            .get("items_field")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if items_field.is_empty() {
            items_field = loop_cfg
                .get("items")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
        }
        if !items_field.is_empty() {
            if let Some(Value::Array(arr)) = state.data.get(&items_field) {
                items = arr.clone();
            }
        } else if let Some(Value::Array(arr)) = loop_cfg.get("items") {
            items = arr.clone();
        }

        let mut max_iter = 100usize;
        if let Some(v) = loop_cfg.get("max_iterations") {
            let n = to_int(v);
            if n > 0 {
                max_iter = n as usize;
            }
        }
        let continue_on_error = loop_cfg
            .get("continue_on_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let item_var_name = loop_cfg
            .get("item_variable")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("item")
            .to_string();

        let mut loop_results: Vec<Value> = Vec::new();
        let mut failures: Vec<Value> = Vec::new();
        let start_t = now_utc();

        for (i, item) in items.into_iter().enumerate() {
            if i >= max_iter {
                break;
            }
            let mut item_payload = state.data.clone();
            item_payload.insert("_item".to_string(), item.clone());
            item_payload.insert("_index".to_string(), Value::Int(i as i64));
            item_payload.insert(item_var_name.clone(), item);

            if step.activity.is_empty() {
                continue;
            }
            let (out, err, _) = self.run_activity(step, &step.activity, &item_payload);
            if let Some(err_str) = err {
                failures.push(Value::String(format!("item {}: {}", i, err_str)));
                if !continue_on_error {
                    state.traces.push(StepExecutionTrace {
                        step_id: step.id.clone(),
                        activity_name: step.activity.clone(),
                        status: "FAILED".to_string(),
                        is_async: false,
                        input_payload: self.mask_step_payload(&item_payload, Some(bp), Some(step)),
                        output_payload: ValueMap::new(),
                        error_message: err_str.clone(),
                        attempts: 1,
                        started_at: start_t,
                        finished_at: now_utc(),
                        duration_ms: 0.0,
                    });
                    return Err(format!(
                        "loop step '{}' failed at item {}: {}",
                        step.id, i, err_str
                    ));
                }
                loop_results.push(Value::Object(vmap! {
                    "_error" => err_str,
                    "_index" => i as i64,
                }));
                continue;
            }
            loop_results.push(Value::Object(out.clone()));
            for (k, v) in out {
                state.data.insert(k, v);
            }
        }

        state.data.insert(
            format!("{}_results", step.id),
            Value::Array(loop_results.clone()),
        );
        let trace = StepExecutionTrace {
            step_id: step.id.clone(),
            activity_name: step.activity.clone(),
            status: "COMPLETED".to_string(),
            is_async: false,
            input_payload: self.mask_step_payload(&state.data, Some(bp), Some(step)),
            output_payload: self.mask_step_payload(
                &vmap! {
                    "iterations" => loop_results.len() as i64,
                    "results" => Value::Array(loop_results),
                    "failures" => Value::Array(failures),
                },
                Some(bp),
                Some(step),
            ),
            error_message: String::new(),
            attempts: 1,
            started_at: start_t,
            finished_at: now_utc(),
            duration_ms: 0.0,
        };
        state.traces.push(trace);
        Ok(())
    }

    fn retryable(err_str: &str) -> bool {
        if let Some(mw_err) = OctopusMiddlewareError::parse_from_error_str(err_str) {
            if mw_err.code >= 400 && mw_err.code < 500 {
                return false;
            }
        }
        true
    }

    fn run_activity(
        &self,
        step: &WorkflowStep,
        name: &str,
        input: &ValueMap,
    ) -> (ValueMap, Option<String>, usize) {
        let rp = step_retry_policy(
            Some(step),
            RetryPolicy {
                max_attempts: 1,
                initial_interval: Duration::from_millis(200),
                backoff_coefficient: 2.0,
                maximum_interval: Duration::ZERO,
            },
        );
        let mut delay = rp.initial_interval;
        let mut out = ValueMap::new();
        let mut last_err: Option<String> = None;
        let mut attempt = 0usize;

        while attempt < rp.max_attempts {
            attempt += 1;
            match self.run_once(step, name, input) {
                Ok(res) => {
                    out = res;
                    last_err = None;
                    break;
                }
                Err(e) => {
                    let is_retryable = Self::retryable(&e);
                    last_err = Some(e);
                    if !is_retryable || attempt >= rp.max_attempts {
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

        (out, last_err, attempt)
    }

    fn run_once(
        &self,
        step: &WorkflowStep,
        name: &str,
        input: &ValueMap,
    ) -> Result<ValueMap, String> {
        if step.timeout_seconds <= 0 {
            return self.activity_executor.execute_activity(name, input);
        }
        let timeout = Duration::from_secs(step.timeout_seconds as u64);
        let (tx, rx) = std::sync::mpsc::channel();
        let exec = Arc::clone(&self.activity_executor);
        let act_name = name.to_string();
        let inp = input.clone();
        crate::workers::spawn(move || {
            let res = exec.execute_activity(&act_name, &inp);
            let _ = tx.send(res);
        });
        match rx.recv_timeout(timeout) {
            Ok(res) => res,
            Err(_) => {
                let mw_err = OctopusMiddlewareError {
                    name: "ActivityTimeout".to_string(),
                    code: 504,
                    message: format!(
                        "Activity '{}' did not finish within {}s.",
                        name, step.timeout_seconds
                    ),
                    details: ValueMap::new(),
                    headers: std::collections::BTreeMap::new(),
                };
                Err(mw_err.to_string())
            }
        }
    }

    fn abort(
        &self,
        mut state: ExecutionState,
        bp: &WorkflowBlueprint,
        saga: &[SagaAction],
        err_str: &str,
        start: Instant,
    ) -> (ExecutionState, Option<String>) {
        let mw_err = OctopusMiddlewareError::parse_from_error_str(err_str);
        let clean_msg = mw_err
            .as_ref()
            .map(|e| e.message.clone())
            .unwrap_or_else(|| err_str.to_string());
        let mut code = 500;
        if let Some(mwe) = mw_err {
            code = mwe.code;
            state.error_type = Some(mwe.name);
        }
        state.error = Some(clean_msg.clone());
        state.error_code = Some(code);
        state.status = ExecutionStatus::Failed;
        if !saga.is_empty() {
            let (output, _) = self.saga_manager.run_compensations(saga);
            state.data.extend(output);
            state.status = ExecutionStatus::RolledBack;
        }
        let now = now_utc();
        state.finished_at = Some(now);
        state.duration_ms = (start.elapsed().as_micros() as f64) / 1000.0;
        let _ = self.save(&state, bp);
        self.emit(&state.job_id, "workflow_failed", || {
            self.snapshot(&state, bp).to_value()
        });
        (state, Some(clean_msg))
    }

    fn find_exception_config(
        &self,
        step: Option<&WorkflowStep>,
        blueprint: Option<&WorkflowBlueprint>,
        err_str: &str,
        mw_err: Option<&OctopusMiddlewareError>,
    ) -> Option<MiddlewareExceptionConfig> {
        // The type and its base types (D14); a bare message has none.
        let mut types = OctopusMiddlewareError::type_chain(err_str);
        if types.is_empty() {
            types.extend(mw_err.map(|e| e.name.clone()));
        }
        let is_type = |name: &str| types.iter().any(|t| t.eq_ignore_ascii_case(name));
        let err_lower = err_str.to_lowercase();

        // 1. Check step-level declared exceptions first
        if let Some(s) = step {
            for ec in &s.exceptions {
                if is_type(&ec.exception) {
                    return Some(ec.clone());
                }
                if !ec.exception.is_empty() && err_lower.contains(&ec.exception.to_lowercase()) {
                    return Some(ec.clone());
                }
            }
        }

        // 2. Check workflow-level middleware exceptions
        if let Some(bp) = blueprint {
            for mw in &bp.middlewares {
                for ec in &mw.exceptions {
                    if is_type(&ec.exception) {
                        return Some(ec.clone());
                    }
                    if !ec.exception.is_empty() && err_lower.contains(&ec.exception.to_lowercase())
                    {
                        return Some(ec.clone());
                    }
                }
            }

            // 3. Middleware Wildcard '*'
            for mw in &bp.middlewares {
                for ec in &mw.exceptions {
                    if ec.exception == "*"
                        || ec.exception.eq_ignore_ascii_case("exception")
                        || ec.exception.eq_ignore_ascii_case("error")
                    {
                        return Some(ec.clone());
                    }
                }
            }

            // 4. Global Exception Handler
            let gh = bp
                .error_handling
                .as_ref()
                .and_then(|eh| eh.global_handler.as_ref())
                .or(bp.global_exception_handler.as_ref());

            if let Some(gh) = gh {
                if !gh.activity.is_empty() {
                    let code = if gh.code == 0 { 500 } else { gh.code };
                    return Some(MiddlewareExceptionConfig {
                        exception: "*".to_string(),
                        code,
                        message: gh.message.clone(),
                        activity: gh.activity.clone(),
                        is_async: gh.get_is_async(),
                        r#async: gh.get_is_async(),
                        headers: gh.headers.clone(),
                    });
                }
            }
        }
        None
    }

    fn mask_step_payload(
        &self,
        payload: &ValueMap,
        bp: Option<&WorkflowBlueprint>,
        step: Option<&WorkflowStep>,
    ) -> ValueMap {
        let Some(bp) = bp else {
            return copy_map_recursive(payload);
        };
        let (fields, mask_val) = bp.get_mask_config(step);
        mask_payload(payload, &fields, &mask_val)
    }

    /// What execute hands back: masked like the stored job (decision D5),
    /// plus the real output for a declared route's response. Activities
    /// always receive the real values.
    fn returned(&self, state: &ExecutionState, bp: &WorkflowBlueprint) -> ExecutionState {
        let mut cp = self.snapshot(state, bp);
        cp.route_output = state.output_data.clone();
        cp
    }

    pub fn snapshot(&self, state: &ExecutionState, bp: &WorkflowBlueprint) -> ExecutionState {
        let (fields, mask_value) = bp.all_mask_fields();
        let mut cp = state.clone();
        cp.route_output = None;
        cp.data = mask_payload(&state.data, &fields, &mask_value);
        if let Some(ref od) = state.output_data {
            cp.output_data = Some(mask_payload(od, &fields, &mask_value));
        }
        cp.traces = state
            .traces
            .iter()
            .map(|t| {
                let mut tc = t.clone();
                tc.input_payload = mask_payload(&t.input_payload, &fields, &mask_value);
                tc.output_payload = mask_payload(&t.output_payload, &fields, &mask_value);
                tc
            })
            .collect();
        self.apply_async_outcomes(&mut cp, &fields, &mask_value);
        cp
    }

    /// Publishes a live event, building its data only when someone can
    /// receive it.
    fn emit(&self, job_id: &str, event: &str, data: impl FnOnce() -> Value) {
        if !self.state_store.read().unwrap().has_listeners(job_id) {
            return;
        }
        let event = LiveEvent {
            event: event.to_string(),
            data: data(),
        };
        let _ = self
            .state_store
            .read()
            .unwrap()
            .publish_event(job_id, event);
    }

    fn save(&self, state: &ExecutionState, bp: &WorkflowBlueprint) -> Result<(), String> {
        let _guard = self.save_mu.lock().unwrap();
        let snap = self.snapshot(state, bp);
        self.state_store.read().unwrap().save_state(snap)
    }

    fn apply_async_outcomes(&self, st: &mut ExecutionState, fields: &[String], mask_value: &str) {
        let (outcomes, _) = &*self.outcomes_mu.read().unwrap();
        for t in &mut st.traces {
            if t.status != "ASYNC_DISPATCHED" {
                continue;
            }
            let key = format!("{}|{}", st.job_id, t.step_id);
            let Some(o) = outcomes.get(&key) else {
                continue;
            };
            t.finished_at = o.at.clone();
            if let Some(ref e) = o.err {
                t.status = "ASYNC_FAILED".to_string();
                t.error_message = e.clone();
            } else {
                t.status = "ASYNC_COMPLETED".to_string();
                t.output_payload = mask_payload(&o.result, fields, mask_value);
            }
        }
    }

    fn on_async_done(
        &self,
        job_id: &str,
        step_id: &str,
        _activity: &str,
        result: ValueMap,
        err: Option<&str>,
    ) {
        if job_id.is_empty() {
            return;
        }
        let _save_guard = self.save_mu.lock().unwrap();
        let key = format!("{}|{}", job_id, step_id);
        {
            let mut guard = self.outcomes_mu.write().unwrap();
            if !guard.0.contains_key(&key) {
                guard.1.push(key.clone());
                if guard.1.len() > 2000 {
                    let oldest = guard.1.remove(0);
                    guard.0.remove(&oldest);
                }
            }
            guard.0.insert(
                key,
                AsyncOutcome {
                    result: copy_map_recursive(&result),
                    err: err.map(|s| s.to_string()),
                    at: now_utc(),
                },
            );
        }

        let store = self.state_store.read().unwrap().clone();
        let Ok(mut stored) = store.get_state(job_id) else {
            return;
        };
        let (fields, mask_value) = self
            .blueprint_repo
            .get_blueprint(&stored.workflow_name)
            .map(|bp| bp.all_mask_fields())
            .unwrap_or_default();
        self.apply_async_outcomes(&mut stored, &fields, &mask_value);
        let _ = store.save_state(stored);
    }

    fn async_payload(
        &self,
        step: &WorkflowStep,
        data: &ValueMap,
        bp: &WorkflowBlueprint,
    ) -> ValueMap {
        let mut out = ValueMap::new();
        if !step.input.is_empty() {
            for k in &step.input {
                if let Some(v) = data.get(k) {
                    out.insert(k.clone(), v.clone());
                }
            }
            if let Some(v) = data.get("job_id") {
                out.insert("job_id".to_string(), v.clone());
            }
            return out;
        }
        let (fields, _) = bp.all_mask_fields();
        let sensitive: std::collections::HashSet<String> =
            fields.into_iter().map(|f| f.to_lowercase()).collect();
        for (k, v) in data {
            let k_lower = k.to_lowercase();
            let stripped = k.strip_prefix("_injected_").unwrap_or(k).to_lowercase();
            if !sensitive.contains(&k_lower) && !sensitive.contains(&stripped) {
                out.insert(k.clone(), v.clone());
            }
        }
        out
    }

    fn suspend(
        &self,
        bp: &WorkflowBlueprint,
        mut state: ExecutionState,
        step: &WorkflowStep,
        rest: Vec<WorkflowStep>,
        saga: Vec<SagaAction>,
        idem_key: &str,
    ) -> (ExecutionState, Option<String>) {
        let now = now_utc();
        state.status = ExecutionStatus::AwaitingSignal;
        state.traces.push(StepExecutionTrace {
            step_id: step.id.clone(),
            activity_name: String::new(),
            status: ExecutionStatus::AwaitingSignal.as_str().to_string(),
            is_async: false,
            input_payload: ValueMap::new(),
            output_payload: vmap! {
                "signal" => step.signal_name(),
                "timeout_seconds" => step.timeout_seconds as i64,
            },
            error_message: String::new(),
            attempts: 0,
            started_at: now.clone(),
            finished_at: now,
            duration_ms: 0.0,
        });

        let mut data = state.data.clone();
        if !step.input.is_empty() {
            let mut filtered = vmap! { "job_id" => state.job_id.clone() };
            for k in &step.input {
                if let Some(v) = state.data.get(k) {
                    filtered.insert(k.clone(), v.clone());
                }
            }
            data = filtered;
        }

        let compensations: Vec<CompensationRecord> = saga
            .iter()
            .map(|a| CompensationRecord {
                step_id: a.step_id.clone(),
                compensation: a.compensation.clone(),
                payload: a.payload.clone(),
            })
            .collect();

        let deadline = if step.timeout_seconds > 0 {
            Some(Instant::now() + Duration::from_secs(step.timeout_seconds as u64))
        } else {
            None
        };

        let susp = Suspension {
            job_id: state.job_id.clone(),
            workflow_name: state.workflow_name.clone(),
            step_id: step.id.clone(),
            signal: step.signal_name().to_string(),
            route: state.route.clone(),
            method: state.method.clone(),
            idem_key: idem_key.to_string(),
            started_at: state.started_at.clone(),
            deadline,
            steps: rest,
            data,
            output_data: state.output_data.clone(),
            traces: state.traces.clone(),
            compensations,
        };

        if let Err(e) = self.suspensions.read().unwrap().save(susp.clone()) {
            return self.abort(
                state,
                bp,
                &saga,
                &format!("suspending at '{}': {}", step.id, e),
                Instant::now(),
            );
        }
        self.schedule_timeout(&susp);
        let _ = self.save(&state, bp);
        self.emit(&state.job_id, "workflow_suspended", || {
            self.snapshot(&state, bp).to_value()
        });
        (state, None)
    }

    fn schedule_timeout(&self, susp: &Suspension) {
        let Some(deadline) = susp.deadline else {
            return;
        };
        let job_id = susp.job_id.clone();
        let signal = susp.signal.clone();
        let wait = deadline.saturating_duration_since(Instant::now());

        let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut timers = self.timers.lock().unwrap();
            if let Some(prev) = timers.insert(job_id.clone(), Arc::clone(&cancel_flag)) {
                prev.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let weak = self.self_weak.read().unwrap().clone();
        std::thread::spawn(move || {
            std::thread::sleep(wait);
            if cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if let Some(pm) = weak.upgrade() {
                let _ = pm.resume(&job_id, &signal, ValueMap::new(), ValueMap::new(), true);
            }
        });
    }

    fn resume(
        &self,
        job_id: &str,
        name: &str,
        data: ValueMap,
        by: ValueMap,
        timed_out: bool,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        let susp = self
            .suspensions
            .read()
            .unwrap()
            .take(job_id)
            .map_err(|_| (None, ERR_NO_SUSPENSION.to_string()))?;

        if !name.is_empty() && name != susp.signal {
            let _ = self.suspensions.read().unwrap().save(susp);
            return Err((None, ERR_SIGNAL_MISMATCH.to_string()));
        }

        if let Some(flag) = self.timers.lock().unwrap().remove(job_id) {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        let bp = self
            .blueprint_repo
            .get_blueprint(&susp.workflow_name)
            .map_err(|e| (None, e))?;
        let now = now_utc();
        let total_steps = susp.traces.len() + susp.steps.len();
        let mut state = ExecutionState {
            job_id: susp.job_id.clone(),
            workflow_name: susp.workflow_name.clone(),
            status: ExecutionStatus::Running,
            route: susp.route.clone(),
            method: susp.method.clone(),
            current_step_id: String::new(),
            current_index: 0,
            total_steps,
            data: susp.data.clone(),
            output_data: susp.output_data.filter(|m| !m.is_empty()),
            traces: susp.traces,
            started_at: susp.started_at,
            finished_at: None,
            client_host: "127.0.0.1".to_string(),
            duration_ms: 0.0,
            error: None,
            error_code: None,
            error_type: None,
            route_output: None,
        };

        state.data.insert(
            "signal".to_string(),
            Value::Object(vmap! {
                "name" => susp.signal.clone(),
                "data" => Value::Object(data.clone()),
                "by" => Value::Object(by.clone()),
                "timed_out" => timed_out,
                "received_at" => now.clone(),
            }),
        );

        let status = if timed_out {
            "SIGNAL_TIMED_OUT"
        } else {
            "SIGNAL_RECEIVED"
        };
        state.traces.push(StepExecutionTrace {
            step_id: susp.step_id,
            activity_name: String::new(),
            status: status.to_string(),
            is_async: false,
            input_payload: vmap! {
                "signal" => susp.signal,
                "data" => Value::Object(data),
                "by" => Value::Object(by),
            },
            output_payload: ValueMap::new(),
            error_message: String::new(),
            attempts: 0,
            started_at: now.clone(),
            finished_at: now,
            duration_ms: 0.0,
        });

        let saga: Vec<SagaAction> = susp
            .compensations
            .into_iter()
            .map(|c| SagaAction {
                step_id: c.step_id,
                compensation: c.compensation,
                payload: c.payload,
            })
            .collect();

        self.emit(job_id, "workflow_resumed", || {
            self.snapshot(&state, &bp).to_value()
        });

        let (final_state, err_opt) = self.run_steps(
            &bp,
            state,
            susp.steps,
            0,
            saga,
            Instant::now(),
            &susp.idem_key,
        );
        let final_state = self.returned(&final_state, &bp);
        if let Some(err) = err_opt {
            Err((Some(final_state), err))
        } else {
            Ok(final_state)
        }
    }
}

fn replay_copy(state: &ExecutionState) -> ExecutionState {
    let mut cp = state.clone();
    cp.data = vmap! { "job_id" => state.job_id.clone() };
    cp.output_data = state.output_data.as_ref().map(copy_map_recursive);
    cp.traces = Vec::new();
    cp
}

impl IWorkflowEngine for PipelineManager {
    fn execute_sync(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        self.execute_endpoint_sync(workflow_name, None, "", "", payload)
    }

    fn execute_endpoint_sync(
        &self,
        workflow_name: &str,
        endpoint: Option<&APIEndpointConfig>,
        route: &str,
        method: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        let pipeline_start_time = Instant::now();
        let bp = self
            .blueprint_repo
            .get_blueprint(workflow_name)
            .map_err(|e| {
                (
                    None,
                    format!("workflow blueprint '{}' not found: {}", workflow_name, e),
                )
            })?;

        let job_id = payload
            .get("job_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(new_job_id);

        let idem_key = if let Some(ep) = endpoint {
            format!(
                "{}|{}|{}|{}",
                workflow_name,
                ep.method.to_uppercase(),
                ep.route,
                job_id
            )
        } else {
            format!("{}|{}|{}|{}", workflow_name, method, route, job_id)
        };

        if let Some(cached) = self.idempotency_manager.get(&idem_key) {
            return Ok(self.returned(&cached, &bp));
        }

        let steps_to_run = if let Some(ep) = endpoint {
            if !ep.steps.is_empty() {
                ep.steps.clone()
            } else if !bp.steps.is_empty() {
                bp.steps.clone()
            } else if !bp.api.is_empty() && !bp.api[0].steps.is_empty() {
                bp.api[0].steps.clone()
            } else {
                Vec::new()
            }
        } else if !bp.steps.is_empty() {
            bp.steps.clone()
        } else if !bp.api.is_empty() && !bp.api[0].steps.is_empty() {
            bp.api[0].steps.clone()
        } else {
            Vec::new()
        };

        let req_route = if !route.is_empty() {
            route.to_string()
        } else if let Some(ep) = endpoint {
            ep.route.clone()
        } else {
            String::new()
        };

        let req_method = if !method.is_empty() {
            method.to_string()
        } else if let Some(ep) = endpoint {
            ep.method.clone()
        } else {
            String::new()
        };

        let client_host = payload
            .get("client_host")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                payload
                    .get("client_ip")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("127.0.0.1")
            .to_string();

        let mut data = payload;
        data.insert("job_id".to_string(), Value::String(job_id.clone()));

        let state = ExecutionState {
            job_id: job_id.clone(),
            workflow_name: workflow_name.to_string(),
            status: ExecutionStatus::Running,
            route: req_route,
            method: req_method,
            current_step_id: String::new(),
            current_index: 0,
            total_steps: steps_to_run.len(),
            data,
            output_data: None,
            traces: Vec::new(),
            started_at: now_utc(),
            finished_at: None,
            client_host,
            duration_ms: 0.0,
            error: None,
            error_code: None,
            error_type: None,
            route_output: None,
        };

        let _ = self.save(&state, &bp);
        self.emit(&job_id, "workflow_started", || {
            self.snapshot(&state, &bp).to_value()
        });

        let (final_state, err_opt) = self.run_steps(
            &bp,
            state,
            steps_to_run,
            0,
            Vec::new(),
            pipeline_start_time,
            &idem_key,
        );
        let final_state = self.returned(&final_state, &bp);
        if let Some(err) = err_opt {
            Err((Some(final_state), err))
        } else {
            Ok(final_state)
        }
    }

    fn initiate_async(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, String> {
        let bp = self
            .blueprint_repo
            .get_blueprint(workflow_name)
            .map_err(|e| format!("workflow blueprint '{}' not found: {}", workflow_name, e))?;

        let mut data = payload;
        let job_id = data
            .get("job_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(new_job_id);
        data.insert("job_id".to_string(), Value::String(job_id.clone()));

        let state = ExecutionState {
            job_id,
            workflow_name: workflow_name.to_string(),
            status: ExecutionStatus::Running,
            route: String::new(),
            method: String::new(),
            current_step_id: String::new(),
            current_index: 0,
            total_steps: 0,
            data: data.clone(),
            output_data: None,
            traces: Vec::new(),
            started_at: now_utc(),
            finished_at: None,
            client_host: "127.0.0.1".to_string(),
            duration_ms: 0.0,
            error: None,
            error_code: None,
            error_type: None,
            route_output: None,
        };
        let _ = self.save(&state, &bp);
        let snap = self.snapshot(&state, &bp);

        let pm_clone = self.clone_handle();
        let wf = workflow_name.to_string();
        crate::workers::spawn(move || {
            let _ = pm_clone.execute_sync(&wf, data);
        });

        Ok(snap)
    }

    fn signal(
        &self,
        job_id: &str,
        name: &str,
        data: ValueMap,
        by: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)> {
        self.resume(job_id, name, data, by, false)
    }

    fn stream_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String> {
        self.state_store.read().unwrap().subscribe_events(job_id)
    }

    fn get_execution_state(&self, job_id: &str) -> Result<ExecutionState, String> {
        self.state_store.read().unwrap().get_state(job_id)
    }

    fn list_execution_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String> {
        self.state_store
            .read()
            .unwrap()
            .list_states(workflow_name, limit)
    }
}
