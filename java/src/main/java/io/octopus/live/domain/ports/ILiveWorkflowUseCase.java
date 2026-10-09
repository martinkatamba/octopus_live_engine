package io.octopus.live.domain.ports;

import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.LiveEvent;
import java.util.Map;
import java.util.function.Consumer;

public interface ILiveWorkflowUseCase {
    ExecutionState executeSync(String workflowName, Map<String, Object> payload);
    ExecutionState executeEndpointSync(String workflowName, io.octopus.live.domain.models.WorkflowBlueprint.ApiEndpointConfig endpoint, String route, String method, Map<String, Object> payload);
    ExecutionState initiateAsync(String workflowName, Map<String, Object> payload);
    void streamEvents(String jobId, Consumer<LiveEvent> consumer);
    ExecutionState getExecutionState(String jobId);
    java.util.List<ExecutionState> listExecutionStates(String workflowName, int limit);
}
