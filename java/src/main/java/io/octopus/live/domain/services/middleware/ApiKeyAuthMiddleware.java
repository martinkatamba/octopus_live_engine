package io.octopus.live.domain.services.middleware;

import java.util.HashSet;
import java.util.Set;

public class ApiKeyAuthMiddleware extends LiveMiddleware {
    private String headerName = "X-API-Key";
    // Empty until configured: with no keys, no request passes.
    private Set<String> validKeys = new HashSet<>();

    public ApiKeyAuthMiddleware() {
        super("ApiKeyAuthMiddleware");
    }

    public String getHeaderName() { return headerName; }
    public void setHeaderName(String headerName) { this.headerName = headerName != null ? headerName : "X-API-Key"; }

    public Set<String> getValidKeys() { return validKeys; }
    public void setValidKeys(Set<String> validKeys) { this.validKeys = validKeys != null ? validKeys : new HashSet<>(); }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        String key = ctx.getHeaders().get(headerName);
        if (key == null || key.isBlank()) {
            key = ctx.getHeaders().get(headerName.toLowerCase());
        }

        if (key == null || !validKeys.contains(key)) {
            throw new InvalidApiKeyException("Invalid or missing '" + headerName + "' header");
        }
    }
}
