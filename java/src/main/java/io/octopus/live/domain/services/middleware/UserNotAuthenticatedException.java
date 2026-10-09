package io.octopus.live.domain.services.middleware;

public class UserNotAuthenticatedException extends OctopusMiddlewareException {
    public UserNotAuthenticatedException(String detail) {
        super("UserNotAuthenticated", 401, detail != null ? detail : "User is not authenticated or token is missing/invalid");
    }
}
