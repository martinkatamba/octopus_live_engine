package io.octopus.live.domain.services.middleware;

import java.util.Map;

public class PayloadTooLargeException extends OctopusMiddlewareException {
    public PayloadTooLargeException(String detail, long maxSize, long actualSize) {
        super("PayloadTooLargeException", 413,
              detail != null ? detail : "Request payload size " + actualSize + " bytes exceeds maximum limit of " + maxSize + " bytes.",
              Map.of("max_size_bytes", maxSize, "actual_size_bytes", actualSize),
              Map.of());
    }
}
