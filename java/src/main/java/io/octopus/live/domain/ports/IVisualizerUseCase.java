package io.octopus.live.domain.ports;

import io.octopus.live.domain.models.WorkflowBlueprint;
import java.util.List;
import java.util.Map;

public interface IVisualizerUseCase {
    Map<String, Object> compileX6Diagram(String workflowName, String jobId);
    Map<String, Object> compileX6Diagram(String workflowName, String jobId, String route, String method);
    List<WorkflowBlueprint> listBlueprints();
}
