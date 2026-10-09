package io.octopus.live.domain.services.middleware;

import java.util.HashMap;
import java.util.Map;

public class LiveRequestContext {
    private String path;
    private String method;
    private String clientIp;
    private String correlationId;
    private Map<String, String> headers = new HashMap<>();
    private Map<String, String> queryParams = new HashMap<>();
    private Map<String, Object> body = new HashMap<>();
    private byte[] rawBodyBytes = new byte[0];
    private String rawBody = "";
    private Map<String, Object> authClaims = new HashMap<>();
    private Map<String, Object> workflowInjections = new HashMap<>();
    private Map<String, String> responseHeaders = new HashMap<>();
    private boolean shortCircuited = false;
    private int shortCircuitStatusCode = 200;
    private Object shortCircuitResponse = null;

    public String getPath() { return path; }
    public void setPath(String path) { this.path = path; }

    public String getMethod() { return method; }
    public void setMethod(String method) { this.method = method; }

    public String getClientIp() { return clientIp; }
    public void setClientIp(String clientIp) { this.clientIp = clientIp; }

    public String getCorrelationId() { return correlationId; }
    public void setCorrelationId(String correlationId) { this.correlationId = correlationId; }

    public Map<String, String> getHeaders() { return headers; }
    public void setHeaders(Map<String, String> headers) { this.headers = headers != null ? headers : new HashMap<>(); }

    public Map<String, String> getQueryParams() { return queryParams; }
    public void setQueryParams(Map<String, String> queryParams) { this.queryParams = queryParams != null ? queryParams : new HashMap<>(); }

    public Map<String, Object> getBody() { return body; }
    public void setBody(Map<String, Object> body) { this.body = body != null ? body : new HashMap<>(); }

    public byte[] getRawBodyBytes() { return rawBodyBytes; }
    public void setRawBodyBytes(byte[] rawBodyBytes) {
        this.rawBodyBytes = rawBodyBytes != null ? rawBodyBytes : new byte[0];
        if (this.rawBody.isEmpty() && this.rawBodyBytes.length > 0) {
            this.rawBody = new String(this.rawBodyBytes, java.nio.charset.StandardCharsets.UTF_8);
        }
    }

    public String getRawBody() { return rawBody; }
    public void setRawBody(String rawBody) {
        this.rawBody = rawBody != null ? rawBody : "";
        if (this.rawBodyBytes.length == 0 && !this.rawBody.isEmpty()) {
            this.rawBodyBytes = this.rawBody.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        }
    }

    public Map<String, Object> getAuthClaims() { return authClaims; }
    public void setAuthClaims(Map<String, Object> authClaims) { this.authClaims = authClaims != null ? authClaims : new HashMap<>(); }

    public Map<String, Object> getWorkflowInjections() { return workflowInjections; }
    public void setWorkflowInjections(Map<String, Object> workflowInjections) { this.workflowInjections = workflowInjections != null ? workflowInjections : new HashMap<>(); }

    public Map<String, String> getResponseHeaders() { return responseHeaders; }
    public void setResponseHeaders(Map<String, String> responseHeaders) { this.responseHeaders = responseHeaders != null ? responseHeaders : new HashMap<>(); }

    public boolean isShortCircuited() { return shortCircuited; }
    public void setShortCircuited(boolean shortCircuited) { this.shortCircuited = shortCircuited; }

    public int getShortCircuitStatusCode() { return shortCircuitStatusCode; }
    public void setShortCircuitStatusCode(int shortCircuitStatusCode) { this.shortCircuitStatusCode = shortCircuitStatusCode; }

    public Object getShortCircuitResponse() { return shortCircuitResponse; }
    public void setShortCircuitResponse(Object shortCircuitResponse) {
        this.shortCircuitResponse = shortCircuitResponse;
        this.shortCircuited = true;
    }
}
