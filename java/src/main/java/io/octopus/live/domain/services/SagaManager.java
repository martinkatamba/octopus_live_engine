package io.octopus.live.domain.services;

import io.octopus.live.domain.ports.IActivityExecutor;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public class SagaManager {

    public static class SagaAction {
        private final String stepId;
        private final String compensation;
        private final Map<String, Object> payload;

        public SagaAction(String stepId, String compensation, Map<String, Object> payload) {
            this.stepId = stepId;
            this.compensation = compensation;
            this.payload = payload;
        }

        public String getStepId() { return stepId; }
        public String getCompensation() { return compensation; }
        public Map<String, Object> getPayload() { return payload; }
    }

    private final IActivityExecutor activityExecutor;

    public SagaManager(IActivityExecutor activityExecutor) {
        this.activityExecutor = activityExecutor;
    }

    public List<Exception> executeCompensations(List<SagaAction> actions) {
        List<Exception> errors = new ArrayList<>();
        for (int i = actions.size() - 1; i >= 0; i--) {
            SagaAction action = actions.get(i);
            if (action.getCompensation() == null || action.getCompensation().trim().isEmpty()) {
                continue;
            }
            if (activityExecutor.hasActivity(action.getCompensation())) {
                try {
                    activityExecutor.executeActivity(action.getCompensation(), action.getPayload());
                } catch (Exception ex) {
                    errors.add(new RuntimeException("Compensation '" + action.getCompensation() + "' failed for step '" + action.getStepId() + "'", ex));
                }
            }
        }
        return errors;
    }
}
