package io.octopus.live.domain.services.middleware;

import java.util.Map;

public class BadRequestException extends OctopusMiddlewareException {
    public BadRequestException(String detail) {
        super("BadRequestException", 400, detail != null ? detail : "Invalid or malformed request payload. Please verify syntax and parameters.");
    }

    public BadRequestException(String errorName, String detail) {
        super(errorName, 400, detail != null ? detail : "Invalid or malformed request payload.");
    }

    public BadRequestException(String errorName, String detail, Map<String, Object> details, Map<String, String> headers) {
        super(errorName, 400, detail, details, headers);
    }
}
