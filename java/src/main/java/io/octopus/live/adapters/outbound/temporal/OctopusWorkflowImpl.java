package io.octopus.live.adapters.outbound.temporal;

import io.temporal.activity.ActivityOptions;
import io.temporal.common.RetryOptions;
import io.temporal.workflow.Workflow;

import java.time.Duration;
import java.util.Collections;
import java.util.HashMap;
import java.util.Map;

public class OctopusWorkflowImpl implements IOctopusWorkflow {

    private final IOctopusActivity activityStub;

    public OctopusWorkflowImpl() {
        ActivityOptions options = ActivityOptions.newBuilder()
                .setStartToCloseTimeout(Duration.ofSeconds(60))
                .setRetryOptions(RetryOptions.newBuilder()
                        .setMaximumAttempts(5)
                        .setInitialInterval(Duration.ofMillis(500))
                        .setBackoffCoefficient(2.0)
                        .build())
                .build();
        this.activityStub = Workflow.newActivityStub(IOctopusActivity.class, options);
    }

    @Override
    public Map<String, Object> execute(Map<String, Object> payload) {
        String activityName = payload != null && payload.get("activity_name") != null
                ? payload.get("activity_name").toString()
                : "DefaultActivity";

        Map<String, Object> activityInput = payload != null ? new HashMap<>(payload) : new HashMap<>();
        Map<String, Object> result = activityStub.executeActivity(activityName, activityInput);
        return result != null ? result : Collections.emptyMap();
    }
}
