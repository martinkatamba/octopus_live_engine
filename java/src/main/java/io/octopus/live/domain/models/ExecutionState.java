package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonProperty;
import java.time.Instant;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class ExecutionState {
    @JsonProperty("job_id")
    private String jobId;
    @JsonProperty("workflow_name")
    private String workflowName;
    private ExecutionStatus status = ExecutionStatus.IDLE;
    private String route;
    private String method;
    @JsonProperty("current_step_id")
    private String currentStepId;
    @JsonProperty("current_index")
    private int currentIndex;
    @JsonProperty("total_steps")
    private int totalSteps;
    private Map<String, Object> data = new HashMap<>();
    @JsonProperty("output_data")
    private Map<String, Object> outputData;
    private List<StepExecutionTrace> traces = new ArrayList<>();
    @JsonProperty("started_at")
    private String startedAt = Instant.now().toString();
    @JsonProperty("finished_at")
    private String finishedAt;
    @JsonProperty("client_host")
    private String clientHost;
    @JsonProperty("duration_ms")
    private Double durationMs;
    private String error;
    @JsonProperty("error_code")
    private Integer errorCode;
    @JsonProperty("error_type")
    private String errorType;

    public String getJobId() { return jobId; }
    public void setJobId(String jobId) { this.jobId = jobId; }
    public String getWorkflowName() { return workflowName; }
    public void setWorkflowName(String workflowName) { this.workflowName = workflowName; }
    public ExecutionStatus getStatus() { return status; }
    public void setStatus(ExecutionStatus status) { this.status = status; }
    public String getRoute() { return route; }
    public void setRoute(String route) { this.route = route; }
    public String getMethod() { return method; }
    public void setMethod(String method) { this.method = method; }
    public String getCurrentStepId() { return currentStepId; }
    public void setCurrentStepId(String currentStepId) { this.currentStepId = currentStepId; }
    public int getCurrentIndex() { return currentIndex; }
    public void setCurrentIndex(int currentIndex) { this.currentIndex = currentIndex; }
    public int getTotalSteps() { return totalSteps; }
    public void setTotalSteps(int totalSteps) { this.totalSteps = totalSteps; }
    public Map<String, Object> getData() { return data; }
    public void setData(Map<String, Object> data) { this.data = data; }
    public Map<String, Object> getOutputData() { return outputData; }
    public void setOutputData(Map<String, Object> outputData) { this.outputData = outputData; }
    public List<StepExecutionTrace> getTraces() { return traces; }
    public void setTraces(List<StepExecutionTrace> traces) { this.traces = traces; }
    public String getStartedAt() { return startedAt; }
    public void setStartedAt(String startedAt) { this.startedAt = startedAt; }
    public String getFinishedAt() { return finishedAt; }
    public void setFinishedAt(String finishedAt) { this.finishedAt = finishedAt; }
    public String getClientHost() { return clientHost; }
    public void setClientHost(String clientHost) { this.clientHost = clientHost; }
    public Double getDurationMs() { return durationMs; }
    public void setDurationMs(Double durationMs) { this.durationMs = durationMs; }
    public String getError() { return error; }
    public void setError(String error) { this.error = error; }
    public Integer getErrorCode() { return errorCode; }
    public void setErrorCode(Integer errorCode) { this.errorCode = errorCode; }
    public String getErrorType() { return errorType; }
    public void setErrorType(String errorType) { this.errorType = errorType; }
}
