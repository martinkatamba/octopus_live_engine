package io.octopus.live.domain.services.middleware;

public class InvalidApiKeyException extends OctopusMiddlewareException {
    public InvalidApiKeyException(String detail) {
        super("InvalidApiKeyException", 401, detail != null ? detail : "Invalid or missing API key");
    }
}
