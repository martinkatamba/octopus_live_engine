use std::sync::Arc;

use crate::domain::ports::IActivityExecutor;
use crate::json::ValueMap;

#[derive(Debug, Clone)]
pub struct SagaAction {
    pub step_id: String,
    pub compensation: String,
    pub payload: ValueMap,
}

pub struct SagaManager {
    activity_executor: Arc<dyn IActivityExecutor>,
}

impl SagaManager {
    pub fn new(executor: Arc<dyn IActivityExecutor>) -> Self {
        Self {
            activity_executor: executor,
        }
    }

    pub fn execute_compensations(&self, actions: &[SagaAction]) -> Vec<String> {
        self.run_compensations(actions).1
    }

    /// Runs the compensations newest first. Returns their outputs merged in
    /// that order (so the job data shows what was undone) and the errors.
    pub fn run_compensations(&self, actions: &[SagaAction]) -> (ValueMap, Vec<String>) {
        let mut output = ValueMap::new();
        let mut errs = Vec::new();
        for action in actions.iter().rev() {
            if action.compensation.is_empty() {
                continue;
            }
            if self.activity_executor.has_activity(&action.compensation) {
                match self
                    .activity_executor
                    .execute_activity(&action.compensation, &action.payload)
                {
                    Ok(out) => output.extend(out),
                    Err(err) => errs.push(format!(
                        "compensation '{}' failed for step '{}': {}",
                        action.compensation, action.step_id, err
                    )),
                }
            }
        }
        (output, errs)
    }
}
