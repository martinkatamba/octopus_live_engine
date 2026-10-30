use std::sync::mpsc::Receiver;

use crate::domain::models::{APIEndpointConfig, ExecutionState, LiveEvent, WorkflowBlueprint};
use crate::json::ValueMap;

pub trait IWorkflowEngine: Send + Sync {
    fn execute_sync(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)>;

    fn execute_endpoint_sync(
        &self,
        workflow_name: &str,
        endpoint: Option<&APIEndpointConfig>,
        route: &str,
        method: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)>;

    fn initiate_async(
        &self,
        workflow_name: &str,
        payload: ValueMap,
    ) -> Result<ExecutionState, String>;

    fn signal(
        &self,
        job_id: &str,
        name: &str,
        data: ValueMap,
        by: ValueMap,
    ) -> Result<ExecutionState, (Option<ExecutionState>, String)>;

    fn stream_events(
        &self,
        job_id: &str,
    ) -> Result<(Receiver<LiveEvent>, Box<dyn FnOnce() + Send>), String>;

    fn get_execution_state(&self, job_id: &str) -> Result<ExecutionState, String>;

    fn list_execution_states(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<ExecutionState>, String>;
}

pub trait IVisualizerCompiler: Send + Sync {
    fn compile_x6_diagram(
        &self,
        workflow_name: &str,
        job_id: Option<&str>,
        route: Option<&str>,
        method: Option<&str>,
    ) -> Result<ValueMap, String>;

    fn list_blueprints(&self) -> Result<Vec<WorkflowBlueprint>, String>;
}
