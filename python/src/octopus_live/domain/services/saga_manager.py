import logging
from typing import Any, Dict, List
from ..models.models import StepExecutionTrace
from ..ports.outbound import IActivityExecutor

logger = logging.getLogger("OctopusLive.SagaManager")


class SagaManager:
    """Handles in-memory reverse rollback of executed steps on workflow failure."""

    @classmethod
    async def rollback(
        cls,
        executed_traces: List[StepExecutionTrace],
        activity_executor: IActivityExecutor,
        current_data: Dict[str, Any]
    ) -> Dict[str, Any]:
        logger.warning("🔄 Initiating in-memory Saga rollback across %d executed steps...", len(executed_traces))
        data = dict(current_data)

        # Rollback in reverse order
        for trace in reversed(executed_traces):
            comp_activity = trace.input_payload.get("_compensation_activity")
            if comp_activity and activity_executor.has_activity(comp_activity):
                logger.info("↩️ Executing compensation activity '%s' for step '%s'", comp_activity, trace.step_id)
                try:
                    res = await activity_executor.execute_activity(comp_activity, data)
                    if isinstance(res, dict):
                        data.update(res)
                except Exception as ex:
                    logger.error("Failed compensation '%s': %s", comp_activity, ex)

        return data
