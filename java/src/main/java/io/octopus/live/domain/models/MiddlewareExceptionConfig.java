package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.HashMap;
import java.util.Map;

@JsonIgnoreProperties(ignoreUnknown = true)
public class MiddlewareExceptionConfig {
    private String exception;
    private int code = 400;
    private String message;
    private String activity;
    @JsonProperty("async")
    private boolean asyncMode = false;
    @JsonProperty("is_async")
    private boolean isAsync = false;
    private Map<String, String> headers = new HashMap<>();

    public String getException() { return exception; }
    public void setException(String exception) { this.exception = exception; }

    public int getCode() { return code; }
    public void setCode(int code) { this.code = code; }

    public String getMessage() { return message; }
    public void setMessage(String message) { this.message = message; }

    public String getActivity() { return activity; }
    public void setActivity(String activity) { this.activity = activity; }

    public boolean isAsync() { return isAsync || asyncMode; }
    public void setAsync(boolean asyncMode) { this.asyncMode = asyncMode; this.isAsync = asyncMode; }
    public void setIsAsync(boolean isAsync) { this.isAsync = isAsync; this.asyncMode = isAsync; }

    public Map<String, String> getHeaders() { return headers; }
    public void setHeaders(Map<String, String> headers) { this.headers = headers != null ? headers : new HashMap<>(); }
}
