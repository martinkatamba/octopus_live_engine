package io.octopus.live.domain.services.middleware;

public class InvalidCredentialsException extends OctopusMiddlewareException {
    public InvalidCredentialsException(String detail) {
        super("InvalidCredentials", 401, detail != null ? detail : "Invalid username or password. Please try again.");
    }
    public InvalidCredentialsException() {
        this("Invalid username or password. Please try again.");
    }
}
