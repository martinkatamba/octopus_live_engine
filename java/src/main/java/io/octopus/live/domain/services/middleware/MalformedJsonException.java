package io.octopus.live.domain.services.middleware;

public class MalformedJsonException extends BadRequestException {
    public MalformedJsonException(String detail) {
        super("MalformedJsonException", detail != null ? detail : "Malformed JSON syntax in request body.");
    }
}
