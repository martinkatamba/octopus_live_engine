package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

@JsonIgnoreProperties(ignoreUnknown = true)
public class WorkflowStep {
    private String id;
    private String activity;
    private String description;
    @JsonProperty("timeout_seconds")
    private int timeoutSeconds = 30;
    @JsonProperty("delay_seconds")
    private double delaySeconds = 0.0;
    private String compensation;
    @JsonProperty("branching_step")
    private String branchingStep;
    @JsonProperty("human_step")
    private String humanStep;
    private StepCondition condition;
    private ConditionsBlock conditions;
    @JsonProperty("then_steps")
    private List<WorkflowStep> thenSteps = new ArrayList<>();
    @JsonProperty("else_steps")
    private List<WorkflowStep> elseSteps = new ArrayList<>();
    private Map<String, Object> loop;
    @JsonProperty("for_each")
    private Map<String, Object> forEach;
    @JsonProperty("retry_policy")
    private Map<String, Object> retryPolicy;
    @JsonProperty("jump_to")
    private String jumpTo;
    @JsonProperty("jump_to_step_index")
    private Integer jumpToStepIndex;
    @JsonProperty("rewind_to_step_index")
    private Integer rewindToStepIndex;
    @JsonProperty("async")
    private boolean asyncStep = false;
    @JsonProperty("is_async")
    private boolean isAsync = false;
    private boolean compensate = false;
    private boolean rollback = false;
    @JsonProperty("trigger_compensation")
    private boolean triggerCompensation = false;
    private boolean terminate = false;
    private boolean settle = false;
    private String reason;
    @JsonProperty("dynamic_branching")
    private Map<String, Object> dynamicBranching;
    @JsonProperty("is_output")
    private boolean isOutput = false;
    @JsonProperty("output")
    private boolean output = false;
    private List<MiddlewareExceptionConfig> exceptions = new ArrayList<>();
    @JsonProperty("mask_fields")
    private List<String> maskFields;
    @JsonProperty("mask")
    private List<String> mask;
    private MaskingConfig masking;

    public List<String> getMaskFields() { return maskFields; }
    public void setMaskFields(List<String> maskFields) { this.maskFields = maskFields; }
    public List<String> getMask() { return mask; }
    public void setMask(List<String> mask) { this.mask = mask; }
    public MaskingConfig getMasking() { return masking; }
    public void setMasking(MaskingConfig masking) { this.masking = masking; }

    public boolean isOutput() { return isOutput || output; }
    public void setOutput(boolean output) { this.isOutput = output; this.output = output; }
    public void setIsOutput(boolean isOutput) { this.isOutput = isOutput; this.output = isOutput; }

    public String getId() { return id; }
    public void setId(String id) { this.id = id; }
    public String getActivity() { return activity; }
    public void setActivity(String activity) { this.activity = activity; }
    public String getDescription() { return description; }
    public void setDescription(String description) { this.description = description; }
    public int getTimeoutSeconds() { return timeoutSeconds; }
    public void setTimeoutSeconds(int timeoutSeconds) { this.timeoutSeconds = timeoutSeconds; }
    public double getDelaySeconds() { return delaySeconds; }
    public void setDelaySeconds(double delaySeconds) { this.delaySeconds = delaySeconds; }
    public String getCompensation() { return compensation; }
    public void setCompensation(String compensation) { this.compensation = compensation; }
    public String getBranchingStep() { return branchingStep; }
    public void setBranchingStep(String branchingStep) { this.branchingStep = branchingStep; }
    public String getHumanStep() { return humanStep; }
    public void setHumanStep(String humanStep) { this.humanStep = humanStep; }
    public StepCondition getCondition() { return condition; }
    public void setCondition(StepCondition condition) { this.condition = condition; }
    public ConditionsBlock getConditions() { return conditions; }
    public void setConditions(ConditionsBlock conditions) { this.conditions = conditions; }
    public List<WorkflowStep> getThenSteps() {
        if (thenSteps != null && !thenSteps.isEmpty()) return thenSteps;
        if (condition != null && condition.getThenSteps() != null && !condition.getThenSteps().isEmpty()) return condition.getThenSteps();
        return new ArrayList<>();
    }
    public void setThenSteps(List<WorkflowStep> thenSteps) { this.thenSteps = thenSteps; }
    public List<WorkflowStep> getElseSteps() {
        if (elseSteps != null && !elseSteps.isEmpty()) return elseSteps;
        if (condition != null && condition.getElseSteps() != null && !condition.getElseSteps().isEmpty()) return condition.getElseSteps();
        return new ArrayList<>();
    }
    public void setElseSteps(List<WorkflowStep> elseSteps) { this.elseSteps = elseSteps; }
    public Map<String, Object> getLoop() { return loop; }
    public void setLoop(Map<String, Object> loop) { this.loop = loop; }
    public Map<String, Object> getForEach() { return forEach; }
    public void setForEach(Map<String, Object> forEach) { this.forEach = forEach; }
    public Map<String, Object> getRetryPolicy() { return retryPolicy; }
    public void setRetryPolicy(Map<String, Object> retryPolicy) { this.retryPolicy = retryPolicy; }
    public String getJumpTo() { return jumpTo; }
    public void setJumpTo(String jumpTo) { this.jumpTo = jumpTo; }
    public Integer getJumpToStepIndex() { return jumpToStepIndex; }
    public void setJumpToStepIndex(Integer jumpToStepIndex) { this.jumpToStepIndex = jumpToStepIndex; }
    public Integer getRewindToStepIndex() { return rewindToStepIndex; }
    public void setRewindToStepIndex(Integer rewindToStepIndex) { this.rewindToStepIndex = rewindToStepIndex; }
    public boolean isAsync() { return asyncStep || isAsync; }
    public void setAsync(boolean asyncStep) { this.asyncStep = asyncStep; }
    public void setIsAsync(boolean isAsync) { this.isAsync = isAsync; }
    public boolean isCompensate() { return compensate || rollback || triggerCompensation; }
    public void setCompensate(boolean compensate) { this.compensate = compensate; }
    public void setRollback(boolean rollback) { this.rollback = rollback; }
    public void setTriggerCompensation(boolean triggerCompensation) { this.triggerCompensation = triggerCompensation; }
    public boolean isTerminate() { return terminate || settle; }
    public void setTerminate(boolean terminate) { this.terminate = terminate; }
    public void setSettle(boolean settle) { this.settle = settle; }
    public String getReason() { return reason; }
    public void setReason(String reason) { this.reason = reason; }
    public Map<String, Object> getDynamicBranching() { return dynamicBranching; }
    public void setDynamicBranching(Map<String, Object> dynamicBranching) { this.dynamicBranching = dynamicBranching; }
    public List<MiddlewareExceptionConfig> getExceptions() { return exceptions != null ? exceptions : new ArrayList<>(); }
    public void setExceptions(List<MiddlewareExceptionConfig> exceptions) { this.exceptions = exceptions; }
}
