use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::domain::models::{
    ExecutionState, LiveEvent, Suspension, WorkflowBlueprint, WorkflowStep,
};
use crate::json::ValueMap;

pub type ActivityFunc = Arc<dyn Fn(&ValueMap) -> Result<ValueMap, String> + Send + Sync + 'static>;

pub trait IntoActivityFunc {
    fn into_activity_func(self) -> ActivityFunc;
}

impl<F, E> IntoActivityFunc for F
where
    F: Fn(&ValueMap) -> Result<ValueMap, E> + Send + Sync + 'static,
    E: std::fmt::Display,
{
    fn into_activity_func(self) -> ActivityFunc {
        Arc::new(move |params: &ValueMap| self(params).map_err(|e| e.to_string()))
    }
}

pub trait IActivityExecutor: Send + Sync {
    fn execute_activity(&self, activity_name: &str, input: &ValueMap) -> Result<ValueMap, String>;
    fn register_activity(&self, activity_name: &str, handler: ActivityFunc);
    fn has_activity(&self, activity_name: &str) -> bool;
    fn list_activities(&self) -> Vec<String>;
}

pub trait IBlueprintRepository: Send + Sync {
    /// Blueprints are shared: a run holds the registered one, not a copy.
    fn get_blueprint(&self, workflow_name: &str) -> Result<Arc<WorkflowBlueprint>, String>;
    fn list_blueprints(&self) -> Result<Vec<WorkflowBlueprint>, String>;
    fn save_blueprint(&self, blueprint: WorkflowBlueprint) -> Result<(), String>;
}

pub trait IStateStore: Send + Sync {
    fn save_state(&self, state: ExecutionState) -> Result<(), String>;
    fn get_state(&self, job_id: &str) -> Result<ExecutionState, String>;
    fn list_states(&self, workflow_name: &str, limit: usize)
        -> Result<Vec<ExecutionState>, String>;
    fn publish_event(&self, job_id: &str, event: LiveEvent) -> Result<(), String>;
    /// Whether an event for this job can reach anyone. When it cannot, the
    /// engine skips building the event. Stores that relay events to other
    /// instances answer true.
    fn has_listeners(&self, _job_id: &str) -> bool {
        true
    }
    fn subscribe_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String>;
}

pub type AsyncCompletion = Arc<dyn Fn(&str, &str, &str, ValueMap, Option<&str>) + Send + Sync>;

pub trait IAsyncActivityDispatcher: Send + Sync {
    fn dispatch(
        &self,
        activity_name: &str,
        payload: ValueMap,
        job_id: &str,
        step_id: &str,
        executor: Arc<dyn IActivityExecutor>,
    );
    fn get_dispatcher_type(&self) -> &str;
    fn dispatch_step(
        &self,
        step: &WorkflowStep,
        payload: ValueMap,
        job_id: &str,
        executor: Arc<dyn IActivityExecutor>,
    ) {
        self.dispatch(&step.activity, payload, job_id, &step.id, executor);
    }
    fn set_completion_hook(&self, _hook: AsyncCompletion) {}
    fn is_step_aware(&self) -> bool {
        false
    }
    fn get_dispatched_tasks_history(&self) -> Vec<ValueMap> {
        Vec::new()
    }
}

pub trait ISuspensionStore: Send + Sync {
    fn save(&self, s: Suspension) -> Result<(), String>;
    fn take(&self, job_id: &str) -> Result<Suspension, String>;
    fn list(&self) -> Result<Vec<Suspension>, String>;
}
