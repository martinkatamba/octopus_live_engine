package io.octopus.live.domain.services.middleware;

import java.util.HashMap;
import java.util.Map;

public class RateLimitExceededException extends OctopusMiddlewareException {
    public RateLimitExceededException(String detail, int limit, int windowSeconds, int retryAfter, String key, Map<String, String> headers) {
        super("RateLimitExceeded", 429,
            detail != null ? detail : String.format("Rate limit exceeded. Quota: %d req/%ds. Retry in %ds.", limit, windowSeconds, retryAfter),
            createDetails(limit, windowSeconds, retryAfter, key),
            headers);
    }

    private static Map<String, Object> createDetails(int limit, int windowSeconds, int retryAfter, String key) {
        Map<String, Object> d = new HashMap<>();
        d.put("limit", limit);
        d.put("window_seconds", windowSeconds);
        d.put("retry_after_seconds", retryAfter);
        d.put("rate_limited_key", key);
        return d;
    }
}
