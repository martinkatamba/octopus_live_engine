package io.octopus.live.domain.services.middleware;

import java.util.HashMap;
import java.util.Map;

public class OctopusMiddlewareException extends RuntimeException {
    private final String errorName;
    private final int statusCode;
    private final String detail;
    private final Map<String, Object> details;
    private final Map<String, String> headers;

    public OctopusMiddlewareException(String errorName, int statusCode, String detail, Map<String, Object> details, Map<String, String> headers) {
        super(detail);
        this.errorName = errorName;
        this.statusCode = statusCode;
        this.detail = detail;
        this.details = details != null ? details : new HashMap<>();
        this.headers = headers != null ? headers : new HashMap<>();
    }

    public OctopusMiddlewareException(String errorName, int statusCode, String detail) {
        this(errorName, statusCode, detail, new HashMap<>(), new HashMap<>());
    }

    public String getErrorName() { return errorName; }
    public int getStatusCode() { return statusCode; }
    public String getDetail() { return detail; }
    public Map<String, Object> getDetails() { return details; }
    public Map<String, String> getHeaders() { return headers; }
}
