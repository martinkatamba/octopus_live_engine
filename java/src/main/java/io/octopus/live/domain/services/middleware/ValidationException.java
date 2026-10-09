package io.octopus.live.domain.services.middleware;

public class ValidationException extends BadRequestException {
    public ValidationException(String detail) {
        super("ValidationException", detail != null ? detail : "Request validation rules failed.");
    }

    public ValidationException(String errorName, String detail) {
        super(errorName, detail);
    }
}
