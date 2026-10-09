package io.octopus.live.domain.services.middleware;

import java.util.UUID;

public class CorrelationIdMiddleware extends LiveMiddleware {
    private String headerName = "X-Correlation-ID";

    public CorrelationIdMiddleware() {
        super("CorrelationIdMiddleware");
    }

    public String getHeaderName() { return headerName; }
    public void setHeaderName(String headerName) { this.headerName = headerName != null ? headerName : "X-Correlation-ID"; }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        String corrId = ctx.getHeaders().get(headerName);
        if (corrId == null || corrId.isBlank()) {
            corrId = ctx.getHeaders().get(headerName.toLowerCase());
        }
        if (corrId == null || corrId.isBlank()) {
            corrId = ctx.getHeaders().get("X-Request-ID");
        }
        if (corrId == null || corrId.isBlank()) {
            corrId = ctx.getHeaders().get("x-request-id");
        }
        if (corrId == null || corrId.isBlank()) {
            corrId = ctx.getHeaders().get("X-Job-ID");
        }
        if (corrId == null || corrId.isBlank()) {
            corrId = ctx.getHeaders().get("x-job-id");
        }
        if (corrId == null || corrId.isBlank()) {
            corrId = "req_" + System.currentTimeMillis() + "_" + UUID.randomUUID().toString().substring(0, 6);
        }

        ctx.setCorrelationId(corrId);
        ctx.getResponseHeaders().put(headerName, corrId);
        ctx.getResponseHeaders().put("X-Job-ID", corrId);
    }
}
