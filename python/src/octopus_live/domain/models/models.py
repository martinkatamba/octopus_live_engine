from dataclasses import dataclass, field as dc_field
from datetime import datetime, timezone
from enum import Enum
from typing import Any, Dict, List, Optional
from pydantic import BaseModel
from pydantic import Field as PydanticField


class ExecutionStatus(str, Enum):
    IDLE = "IDLE"
    RUNNING = "RUNNING"
    COMPLETED = "COMPLETED"
    FAILED = "FAILED"
    ROLLED_BACK = "ROLLED_BACK"
    CANCELLED = "CANCELLED"


@dataclass
class StepCondition:
    rule: Optional[str] = None
    operator: Optional[str] = None
    field: Optional[str] = None
    value: Any = None
    rules: List[Dict[str, Any]] = dc_field(default_factory=list)
    match: str = "all"
    rule_type: Optional[str] = None
    then_steps: List["WorkflowStep"] = dc_field(default_factory=list)
    else_steps: List["WorkflowStep"] = dc_field(default_factory=list)


@dataclass
class WorkflowStep:
    id: str
    activity: Optional[str] = None
    description: Optional[str] = None
    timeout_seconds: int = 30
    delay_seconds: float = 0.0
    compensation: Optional[str] = None
    condition: Optional[StepCondition] = None
    loop: Optional[Dict[str, Any]] = None
    for_each: Optional[Dict[str, Any]] = None
    retry_policy: Optional[Dict[str, Any]] = None
    jump_to: Optional[str] = None
    jump_to_step_index: Optional[int] = None
    rewind_to_step_index: Optional[int] = None
    is_async: bool = False
    compensate: bool = False
    rollback: bool = False
    trigger_compensation: bool = False
    terminate: bool = False
    settle: bool = False
    reason: Optional[str] = None
    dynamic_branching: Optional[Dict[str, Any]] = None
    is_output: bool = False
    # A pivot is the point of no return: once it has run, earlier steps are
    # no longer compensated if a later step fails (decision D9).
    pivot: bool = False
    exceptions: List["MiddlewareExceptionConfig"] = dc_field(default_factory=list)
    mask_fields: List[str] = dc_field(default_factory=list)
    masking: Optional["MaskingConfig"] = None


@dataclass
class MaskingConfig:
    enabled: bool = True
    fields: List[str] = dc_field(default_factory=list)
    mask_value: str = "********"


@dataclass
class TelemetryConfig:
    mask_fields: List[str] = dc_field(default_factory=list)
    masking: Optional[MaskingConfig] = None


@dataclass
class ApiEndpointConfig:
    route: str
    method: str = "POST"
    input_model: Optional[str] = None
    output_model: Optional[str] = None
    status_code: int = 200
    summary: Optional[str] = None
    description: Optional[str] = None
    tags: List[str] = dc_field(default_factory=list)
    steps: List[WorkflowStep] = dc_field(default_factory=list)


@dataclass
class MiddlewareExceptionConfig:
    exception: str
    code: int = 400
    message: Optional[str] = None
    activity: Optional[str] = None
    is_async: bool = False
    headers: Dict[str, str] = dc_field(default_factory=dict)


@dataclass
class MiddlewareConfig:
    name: str
    type: str = "pre-request"
    activity: Optional[str] = None
    is_async: bool = False
    excludes: List[str] = dc_field(default_factory=list)
    includes: List[str] = dc_field(default_factory=list)
    exceptions: List[MiddlewareExceptionConfig] = dc_field(default_factory=list)
    config: Dict[str, Any] = dc_field(default_factory=dict)


@dataclass
class GlobalExceptionHandlerConfig:
    activity: Optional[str] = None
    code: int = 500
    message: Optional[str] = None
    is_async: bool = False
    include_stack_trace: bool = True
    headers: Dict[str, str] = dc_field(default_factory=dict)


@dataclass
class ErrorHandlingConfig:
    global_handler: Optional[GlobalExceptionHandlerConfig] = None


@dataclass
class WorkflowBlueprint:
    workflow_name: str
    version: str = "1.0.0"
    namespace: str = "default"
    description: str = ""
    mode: str = "sync"
    compensation_mode: str = "sync"
    allowed_initiator_roles: List[str] = dc_field(default_factory=list)
    middlewares: List[MiddlewareConfig] = dc_field(default_factory=list)
    steps: List[WorkflowStep] = dc_field(default_factory=list)
    api: List[ApiEndpointConfig] = dc_field(default_factory=list)
    persistence: Dict[str, Any] = dc_field(default_factory=dict)
    metadata: Dict[str, Any] = dc_field(default_factory=dict)
    masking: Optional[MaskingConfig] = None
    telemetry: Optional[TelemetryConfig] = None
    redaction: Optional[MaskingConfig] = None
    error_handling: Optional[ErrorHandlingConfig] = None
    global_exception_handler: Optional[GlobalExceptionHandlerConfig] = None

    def get_mask_config(self, step: Optional[WorkflowStep] = None) -> tuple[List[str], str]:
        mask_value = "********"
        fields = []
        seen = set()

        def add_fields(f_list):
            nonlocal mask_value
            if not f_list:
                return
            for f in f_list:
                trimmed = str(f).strip()
                if trimmed and trimmed.lower() not in seen:
                    seen.add(trimmed.lower())
                    fields.append(trimmed)

        # 1. Top-level masking
        if self.masking and getattr(self.masking, "enabled", True):
            if getattr(self.masking, "mask_value", None):
                mask_value = self.masking.mask_value
            add_fields(getattr(self.masking, "fields", []))

        # 2. Telemetry masking
        if self.telemetry:
            if self.telemetry.masking and getattr(self.telemetry.masking, "enabled", True):
                if getattr(self.telemetry.masking, "mask_value", None):
                    mask_value = self.telemetry.masking.mask_value
                add_fields(getattr(self.telemetry.masking, "fields", []))
            if getattr(self.telemetry, "mask_fields", None):
                add_fields(self.telemetry.mask_fields)

        # 3. Redaction
        if self.redaction and getattr(self.redaction, "enabled", True):
            if getattr(self.redaction, "mask_value", None):
                mask_value = self.redaction.mask_value
            add_fields(getattr(self.redaction, "fields", []))

        # 4. Step-level masking
        if step:
            if getattr(step, "masking", None) and getattr(step.masking, "enabled", True):
                if getattr(step.masking, "mask_value", None):
                    mask_value = step.masking.mask_value
                add_fields(getattr(step.masking, "fields", []))
            if getattr(step, "mask_fields", None):
                add_fields(step.mask_fields)

        return fields, mask_value

    def all_mask_fields(self) -> tuple[List[str], str]:
        """Every field masked anywhere in the blueprint: top level and every
        step, including branches and declared routes."""
        fields, mask_value = self.get_mask_config()
        seen = {f.lower() for f in fields}

        def walk(steps: List[WorkflowStep]) -> None:
            for st in steps:
                for f in self.get_mask_config(st)[0]:
                    if f.lower() not in seen:
                        seen.add(f.lower())
                        fields.append(f)
                if st.condition:
                    walk(st.condition.then_steps)
                    walk(st.condition.else_steps)

        walk(self.steps)
        for ep in self.api:
            walk(ep.steps)
        return fields, mask_value


@dataclass
class StepExecutionTrace:
    step_id: str
    activity_name: Optional[str] = None
    status: str = "COMPLETED"
    is_async: bool = False
    input_payload: Dict[str, Any] = dc_field(default_factory=dict)
    output_payload: Dict[str, Any] = dc_field(default_factory=dict)
    error_message: Optional[str] = None
    started_at: str = dc_field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    finished_at: str = dc_field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    duration_ms: float = 0.0
    attempts: int = 1


@dataclass
class ExecutionState:
    job_id: str
    workflow_name: str
    status: ExecutionStatus = ExecutionStatus.IDLE
    route: Optional[str] = None
    method: Optional[str] = None
    current_step_id: Optional[str] = None
    current_index: int = 0
    total_steps: int = 0
    data: Dict[str, Any] = dc_field(default_factory=dict)
    output_data: Optional[Dict[str, Any]] = None
    traces: List[StepExecutionTrace] = dc_field(default_factory=list)
    started_at: str = dc_field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    finished_at: Optional[str] = None
    error: Optional[str] = None
    error_code: Optional[int] = None
    error_type: Optional[str] = None
    client_host: Optional[str] = None
    duration_ms: Optional[float] = None
    # The unmasked output a declared api: route responds with. Everything
    # else about a job is masked (decision D5); stores never persist this.
    route_output: Optional[Dict[str, Any]] = dc_field(default=None, repr=False, compare=False)



class BaseWorkflowPayload(BaseModel):
    job_id: Optional[str] = PydanticField(None, description="Unique tracking and correlation identifier")
    submitted_by_id: Optional[str] = PydanticField(None, description="Initiator identity")


class BaseWorkflowResponse(BaseModel):
    job_id: str
    workflow_name: str
    status: str
    data: Dict[str, Any]
    traces: List[Dict[str, Any]] = PydanticField(default_factory=list)
