package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.time.Instant;
import java.util.HashMap;
import java.util.Map;

public class StepExecutionTrace {
    @JsonProperty("step_id")
    private String stepId;
    @JsonProperty("activity_name")
    private String activityName;
    private String status = "COMPLETED";
    @JsonProperty("is_async")
    private boolean isAsync;
    @JsonProperty("input_payload")
    private Map<String, Object> inputPayload = new HashMap<>();
    @JsonProperty("output_payload")
    private Map<String, Object> outputPayload = new HashMap<>();
    @JsonProperty("error_message")
    private String errorMessage;
    @JsonProperty("started_at")
    private String startedAt = Instant.now().toString();
    @JsonProperty("finished_at")
    private String finishedAt = Instant.now().toString();
    @JsonProperty("duration_ms")
    private double durationMs;

    public boolean isAsync() { return isAsync; }
    public void setAsync(boolean isAsync) { this.isAsync = isAsync; }
    public String getStepId() { return stepId; }
    public void setStepId(String stepId) { this.stepId = stepId; }
    public String getActivityName() { return activityName; }
    public void setActivityName(String activityName) { this.activityName = activityName; }
    public String getStatus() { return status; }
    public void setStatus(String status) { this.status = status; }
    public Map<String, Object> getInputPayload() { return inputPayload; }
    public void setInputPayload(Map<String, Object> inputPayload) { this.inputPayload = inputPayload; }
    public Map<String, Object> getOutputPayload() { return outputPayload; }
    public void setOutputPayload(Map<String, Object> outputPayload) { this.outputPayload = outputPayload; }
    public String getErrorMessage() { return errorMessage; }
    public void setErrorMessage(String errorMessage) { this.errorMessage = errorMessage; }
    public String getStartedAt() { return startedAt; }
    public void setStartedAt(String startedAt) { this.startedAt = startedAt; }
    public String getFinishedAt() { return finishedAt; }
    public void setFinishedAt(String finishedAt) { this.finishedAt = finishedAt; }
    public double getDurationMs() { return durationMs; }
    public void setDurationMs(double durationMs) { this.durationMs = durationMs; }
}
