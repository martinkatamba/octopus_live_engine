from abc import ABC, abstractmethod
from typing import Any, AsyncIterator, Dict, List, Optional
from ..models.models import ExecutionState, WorkflowBlueprint


class ILiveWorkflowUseCase(ABC):
    """Primary inbound use case for executing synchronous and live workflows."""

    @abstractmethod
    async def execute_sync(self, workflow_name: str, payload: Dict[str, Any]) -> ExecutionState:
        """Execute a workflow completely synchronously and return final state."""
        pass

    @abstractmethod
    async def initiate_async(self, workflow_name: str, payload: Dict[str, Any]) -> ExecutionState:
        """Start workflow execution asynchronously in-memory."""
        pass

    @abstractmethod
    async def get_execution_state(self, job_id: str) -> Optional[ExecutionState]:
        """Fetch current execution state."""
        pass

    @abstractmethod
    async def list_execution_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        """Fetch recent execution states."""
        pass

    @abstractmethod
    async def stream_events(self, job_id: str) -> AsyncIterator[Dict[str, Any]]:
        """Stream step execution events as Server-Sent Events."""
        pass



class IVisualizerUseCase(ABC):
    """Inbound use case for compiling DAG visualizer graphs and metadata."""

    @abstractmethod
    def get_blueprint(self, workflow_name: str) -> Optional[WorkflowBlueprint]:
        pass

    @abstractmethod
    def list_blueprints(self) -> List[WorkflowBlueprint]:
        pass

    @abstractmethod
    def compile_x6_diagram(
        self,
        workflow_name: str,
        job_id: Optional[str] = None,
        endpoint_index: Optional[int] = None,
        route: Optional[str] = None,
        method: Optional[str] = None
    ) -> Dict[str, Any]:
        """Generate AntV X6 graph JSON representing the blueprint or live execution trace."""
        pass
