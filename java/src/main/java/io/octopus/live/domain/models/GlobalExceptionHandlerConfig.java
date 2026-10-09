package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.HashMap;
import java.util.Map;

@JsonIgnoreProperties(ignoreUnknown = true)
public class GlobalExceptionHandlerConfig {
    private String activity;
    private int code = 500;
    private String message;
    @JsonProperty("is_async")
    private boolean isAsync;
    private boolean async;
    @JsonProperty("include_stack_trace")
    private boolean includeStackTrace = true;
    private Map<String, String> headers = new HashMap<>();

    public String getActivity() { return activity; }
    public void setActivity(String activity) { this.activity = activity; }
    public int getCode() { return code; }
    public void setCode(int code) { this.code = code; }
    public String getMessage() { return message; }
    public void setMessage(String message) { this.message = message; }
    public boolean isAsync() { return isAsync || async; }
    public void setAsync(boolean async) { this.isAsync = async; this.async = async; }
    public boolean getIsAsync() { return isAsync(); }
    public void setIsAsync(boolean isAsync) { setAsync(isAsync); }
    public boolean isIncludeStackTrace() { return includeStackTrace; }
    public void setIncludeStackTrace(boolean includeStackTrace) { this.includeStackTrace = includeStackTrace; }
    public Map<String, String> getHeaders() { return headers != null ? headers : new HashMap<>(); }
    public void setHeaders(Map<String, String> headers) { this.headers = headers; }
}
