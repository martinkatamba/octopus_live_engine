package io.octopus.live.domain.services.middleware;

import java.time.Instant;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

public class RateLimitMiddleware extends LiveMiddleware {
    private int limit = 60;
    private int windowSeconds = 60;
    private String keyBy = "client_ip";
    private Map<String, Integer> tierLimits = new HashMap<>();
    private final ConcurrentHashMap<String, List<Long>> requestTimestamps = new ConcurrentHashMap<>();

    public RateLimitMiddleware() {
        super("RateLimitMiddleware");
    }

    public int getLimit() { return limit; }
    public void setLimit(int limit) { this.limit = limit; }

    public int getWindowSeconds() { return windowSeconds; }
    public void setWindowSeconds(int windowSeconds) { this.windowSeconds = windowSeconds; }

    public String getKeyBy() { return keyBy; }
    public void setKeyBy(String keyBy) { this.keyBy = keyBy != null ? keyBy : "client_ip"; }

    public Map<String, Integer> getTierLimits() { return tierLimits; }
    public void setTierLimits(Map<String, Integer> tierLimits) { this.tierLimits = tierLimits != null ? tierLimits : new HashMap<>(); }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        long nowSec = Instant.now().getEpochSecond();
        String key = resolveKey(ctx);
        int maxLimit = resolveLimit(ctx);
        long cutoff = nowSec - windowSeconds;

        List<Long> timestamps = requestTimestamps.compute(key, (k, v) -> {
            List<Long> list = v != null ? v : new ArrayList<>();
            list.removeIf(ts -> ts <= cutoff);
            return list;
        });

        synchronized (timestamps) {
            timestamps.removeIf(ts -> ts <= cutoff);
            if (timestamps.size() >= maxLimit) {
                long oldest = timestamps.isEmpty() ? nowSec : timestamps.get(0);
                int retryAfter = (int) Math.max(1, windowSeconds - (nowSec - oldest));
                long resetTs = nowSec + retryAfter;

                Map<String, String> headers = new HashMap<>();
                headers.put("Retry-After", String.valueOf(retryAfter));
                headers.put("X-RateLimit-Limit", String.valueOf(maxLimit));
                headers.put("X-RateLimit-Remaining", "0");
                headers.put("X-RateLimit-Reset", String.valueOf(resetTs));

                throw new RateLimitExceededException(
                    String.format("API request quota exceeded. Please slow down your requests or upgrade to VIP tier. Quota: %d req/%ds. Retry in %ds.", maxLimit, windowSeconds, retryAfter),
                    maxLimit,
                    windowSeconds,
                    retryAfter,
                    key,
                    headers
                );
            }

            timestamps.add(nowSec);
            int remaining = Math.max(0, maxLimit - timestamps.size());
            long resetTs = timestamps.get(0) + windowSeconds;

            ctx.getResponseHeaders().put("X-RateLimit-Limit", String.valueOf(maxLimit));
            ctx.getResponseHeaders().put("X-RateLimit-Remaining", String.valueOf(remaining));
            ctx.getResponseHeaders().put("X-RateLimit-Reset", String.valueOf(resetTs));
        }
    }

    private String resolveKey(LiveRequestContext ctx) {
        String strat = keyBy.toLowerCase();
        if ("jwt_user".equals(strat) || "user".equals(strat) || "user_id".equals(strat)) {
            if (ctx.getAuthClaims() != null && !ctx.getAuthClaims().isEmpty()) {
                Object uid = ctx.getAuthClaims().get("user_id");
                if (uid == null) uid = ctx.getAuthClaims().get("sub");
                if (uid != null) return "user:" + uid;
            }
            return "ip:" + (ctx.getClientIp() != null ? ctx.getClientIp() : "127.0.0.1");
        }
        if ("api_key".equals(strat) || "key".equals(strat)) {
            String k = ctx.getHeaders().get("X-API-Key");
            if (k == null) k = ctx.getHeaders().get("x-api-key");
            return "apikey:" + (k != null ? k : "anonymous");
        }
        if ("path".equals(strat)) {
            return "path:" + ctx.getPath();
        }
        return "ip:" + (ctx.getClientIp() != null ? ctx.getClientIp() : "127.0.0.1");
    }

    private int resolveLimit(LiveRequestContext ctx) {
        if (tierLimits != null && !tierLimits.isEmpty()) {
            String tier = null;
            if (ctx.getAuthClaims() != null && !ctx.getAuthClaims().isEmpty()) {
                Object t = ctx.getAuthClaims().get("customer_tier");
                if (t == null) t = ctx.getAuthClaims().get("tier");
                if (t != null) tier = t.toString();
            }
            if (tier == null) {
                tier = ctx.getHeaders().get("X-User-Tier");
            }
            if (tier == null) {
                tier = "ANONYMOUS";
            }

            if (tierLimits.containsKey(tier)) {
                return tierLimits.get(tier);
            }
            if (tierLimits.containsKey(tier.toUpperCase())) {
                return tierLimits.get(tier.toUpperCase());
            }
        }
        return limit;
    }
}
