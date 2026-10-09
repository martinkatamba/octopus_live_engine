package io.octopus.live.domain.services.middleware;

public class UserNotAuthorizedException extends OctopusMiddlewareException {
    public UserNotAuthorizedException(String detail) {
        super("UserNotAuthorized", 403, detail != null ? detail : "User lacks authorization or required permissions");
    }
}
