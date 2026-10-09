package io.octopus.live.adapters.outbound;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.dataformat.yaml.YAMLFactory;
import io.octopus.live.domain.models.WorkflowBlueprint;
import io.octopus.live.domain.ports.IBlueprintRepository;
import java.io.File;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ConcurrentHashMap;

public class YamlBlueprintRepository implements IBlueprintRepository {

    private final ConcurrentHashMap<String, WorkflowBlueprint> blueprints = new ConcurrentHashMap<>();
    private final ObjectMapper yamlMapper = new ObjectMapper(new YAMLFactory())
            .enable(com.fasterxml.jackson.databind.DeserializationFeature.ACCEPT_SINGLE_VALUE_AS_ARRAY)
            .configure(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);

    @Override
    public void saveBlueprint(WorkflowBlueprint blueprint) {
        if (blueprint != null) {
            blueprint.normalizeEndpoints();
            if ((blueprint.getSteps() == null || blueprint.getSteps().isEmpty())
                    && blueprint.getApi() != null && !blueprint.getApi().isEmpty()
                    && blueprint.getApi().get(0).getSteps() != null && !blueprint.getApi().get(0).getSteps().isEmpty()) {
                blueprint.setSteps(blueprint.getApi().get(0).getSteps());
            }
            blueprints.put(blueprint.getWorkflowName(), blueprint);
        }
    }

    @Override
    public WorkflowBlueprint getBlueprint(String workflowName) {
        return blueprints.get(workflowName);
    }

    @Override
    public List<WorkflowBlueprint> listBlueprints() {
        return new ArrayList<>(blueprints.values());
    }

    public void loadFromDirectory(String directoryPath) {
        File dir = new File(directoryPath);
        if (!dir.exists() || !dir.isDirectory()) return;

        File[] files = dir.listFiles((d, name) -> name.endsWith(".yaml") || name.endsWith(".yml"));
        if (files == null) return;

        for (File file : files) {
            try {
                WorkflowBlueprint bp = yamlMapper.readValue(file, WorkflowBlueprint.class);
                if (bp != null && bp.getWorkflowName() != null && !bp.getWorkflowName().trim().isEmpty()) {
                    saveBlueprint(bp);
                }
            } catch (Exception ignored) {}
        }
    }
}
