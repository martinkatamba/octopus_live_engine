package io.octopus.live.domain.services.middleware;

import java.util.Map;

public class MissingRequiredFieldException extends ValidationException {
    public MissingRequiredFieldException(String fieldOrMsg) {
        super("MissingRequiredFieldException",
              (fieldOrMsg != null && !fieldOrMsg.contains(" "))
                  ? "Missing required field '" + fieldOrMsg + "' in request payload."
                  : (fieldOrMsg != null ? fieldOrMsg : "Missing required field in request payload."));
    }
}
