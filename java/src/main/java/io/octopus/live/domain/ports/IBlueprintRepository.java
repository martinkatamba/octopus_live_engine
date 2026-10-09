package io.octopus.live.domain.ports;

import io.octopus.live.domain.models.WorkflowBlueprint;
import java.util.List;

public interface IBlueprintRepository {
    WorkflowBlueprint getBlueprint(String workflowName);
    List<WorkflowBlueprint> listBlueprints();
    void saveBlueprint(WorkflowBlueprint blueprint);
}
