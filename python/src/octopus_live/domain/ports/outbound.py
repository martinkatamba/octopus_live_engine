from abc import ABC, abstractmethod
from typing import Any, Dict, List, Optional
from ..models.models import ExecutionState, WorkflowBlueprint


class IActivityExecutor(ABC):
    """Secondary outbound port for executing native functions / activities."""

    @abstractmethod
    async def execute_activity(self, activity_name: str, payload: Dict[str, Any]) -> Dict[str, Any]:
        pass

    @abstractmethod
    def has_activity(self, activity_name: str) -> bool:
        pass


class IBlueprintRepository(ABC):
    """Secondary outbound port for loading and querying workflow blueprints."""

    @abstractmethod
    def get(self, workflow_name: str) -> Optional[WorkflowBlueprint]:
        pass

    @abstractmethod
    def list_all(self) -> List[WorkflowBlueprint]:
        pass

    @abstractmethod
    def save(self, blueprint: WorkflowBlueprint) -> None:
        pass


class IStateStore(ABC):
    """Secondary outbound port for storing in-memory execution state and publishing SSE events."""

    @abstractmethod
    async def save_state(self, state: ExecutionState) -> None:
        pass

    @abstractmethod
    async def get_state(self, job_id: str) -> Optional[ExecutionState]:
        pass

    @abstractmethod
    async def list_states(self, workflow_name: Optional[str] = None, limit: int = 100) -> List[ExecutionState]:
        pass

    @abstractmethod
    async def publish_event(self, job_id: str, event_type: str, data: Dict[str, Any]) -> None:
        pass

