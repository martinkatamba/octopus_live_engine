package io.octopus.live.adapters.outbound.temporal;

import io.temporal.workflow.WorkflowInterface;
import io.temporal.workflow.WorkflowMethod;
import java.util.Map;

@WorkflowInterface
public interface IOctopusWorkflow {

    @WorkflowMethod
    Map<String, Object> execute(Map<String, Object> payload);
}
