package io.octopus.live.domain.services.middleware;

public class InvalidClaimException extends OctopusMiddlewareException {
    public InvalidClaimException(String detail) {
        super("InvalidClaimException", 403, detail != null ? detail : "Required claim is missing or invalid");
    }
}
