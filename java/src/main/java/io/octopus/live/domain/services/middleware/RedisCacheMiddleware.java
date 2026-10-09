package io.octopus.live.domain.services.middleware;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;
import io.octopus.live.domain.models.ExecutionState;
import redis.clients.jedis.Jedis;
import redis.clients.jedis.JedisPool;
import redis.clients.jedis.JedisPoolConfig;

import java.net.URI;
import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

public class RedisCacheMiddleware extends LiveMiddleware {

    private int ttlSeconds = 300;
    private List<String> keyStrategy = new ArrayList<>(List.of("route", "claim:user_id", "query"));
    private List<String> methods = new ArrayList<>(List.of("GET"));
    private String headerName = "X-Cache";
    private String prefix = "octopus:cache";
    private boolean cacheControlHeaders = true;
    private String redisUrl = "127.0.0.1:6379";

    private JedisPool jedisPool;
    private final Map<String, CacheItem> fallbackCache = new ConcurrentHashMap<>();
    private final ObjectMapper mapper = new ObjectMapper()
            .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false)
            .setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE);

    public static class CacheItem {
        private final String content;
        private final long expiryTimestampMs;

        public CacheItem(String content, long expiryTimestampMs) {
            this.content = content;
            this.expiryTimestampMs = expiryTimestampMs;
        }

        public String getContent() { return content; }
        public long getExpiryTimestampMs() { return expiryTimestampMs; }
        public boolean isExpired() { return System.currentTimeMillis() > expiryTimestampMs; }
    }

    public RedisCacheMiddleware() {
        super("RedisCacheMiddleware");
        initRedisPool();
    }

    public RedisCacheMiddleware(int ttlSeconds, List<String> keyStrategy, List<String> methods, String headerName, String prefix, boolean cacheControlHeaders, String redisUrl) {
        super("RedisCacheMiddleware");
        this.ttlSeconds = ttlSeconds > 0 ? ttlSeconds : 300;
        if (keyStrategy != null && !keyStrategy.isEmpty()) this.keyStrategy = keyStrategy;
        if (methods != null && !methods.isEmpty()) this.methods = methods;
        if (headerName != null && !headerName.isBlank()) this.headerName = headerName;
        if (prefix != null && !prefix.isBlank()) this.prefix = prefix;
        this.cacheControlHeaders = cacheControlHeaders;
        if (redisUrl != null && !redisUrl.isBlank()) this.redisUrl = redisUrl;
        initRedisPool();
    }

    public void initRedisPool() {
        try {
            String host = "127.0.0.1";
            int port = 6379;
            String url = (redisUrl != null && !redisUrl.isBlank()) ? redisUrl : "redis://127.0.0.1:6379";
            if (!url.startsWith("redis://") && !url.startsWith("rediss://")) {
                url = "redis://" + url;
            }
            try {
                URI uri = URI.create(url);
                if (uri.getHost() != null && !uri.getHost().isBlank() && !"localhost".equalsIgnoreCase(uri.getHost())) {
                    host = uri.getHost();
                }
                if (uri.getPort() > 0) {
                    port = uri.getPort();
                }
            } catch (Exception ignored) {}

            JedisPoolConfig poolConfig = new JedisPoolConfig();
            poolConfig.setMaxTotal(32);
            poolConfig.setMaxIdle(8);
            jedisPool = new JedisPool(poolConfig, host, port, 2000);
            try (Jedis j = jedisPool.getResource()) {
                j.ping();
            }
        } catch (Exception e) {
            System.err.println("⚠️ [RedisCacheMiddleware] JedisPool init warning: " + e.getMessage() + ". Using fast in-memory fallback.");
            jedisPool = null;
        }
    }

    public int getTtlSeconds() { return ttlSeconds; }
    public void setTtlSeconds(int ttlSeconds) { this.ttlSeconds = ttlSeconds > 0 ? ttlSeconds : 300; }

    public List<String> getKeyStrategy() { return keyStrategy; }
    public void setKeyStrategy(List<String> keyStrategy) { this.keyStrategy = keyStrategy != null ? keyStrategy : new ArrayList<>(); }

    public List<String> getMethods() { return methods; }
    public void setMethods(List<String> methods) { this.methods = methods != null ? methods : new ArrayList<>(); }

    public String getHeaderName() { return headerName; }
    public void setHeaderName(String headerName) { this.headerName = headerName; }

    public String getPrefix() { return prefix; }
    public void setPrefix(String prefix) { this.prefix = prefix; }

    public boolean isCacheControlHeaders() { return cacheControlHeaders; }
    public void setCacheControlHeaders(boolean cacheControlHeaders) { this.cacheControlHeaders = cacheControlHeaders; }

    public String getRedisUrl() { return redisUrl; }
    public void setRedisUrl(String redisUrl) { 
        this.redisUrl = redisUrl; 
        initRedisPool();
    }

    public String buildCacheKey(LiveRequestContext ctx, String path) {
        StringBuilder sb = new StringBuilder();
        sb.append(prefix);
        sb.append(':');
        sb.append(ctx.getMethod() != null ? ctx.getMethod().toUpperCase() : "GET");
        sb.append(':');
        sb.append(path);

        for (String strategy : keyStrategy) {
            String sLower = strategy.toLowerCase();
            if (sLower.startsWith("claim:")) {
                String claimKey = strategy.substring(6);
                String claimVal = null;
                if (ctx.getAuthClaims() != null && ctx.getAuthClaims().containsKey(claimKey)) {
                    Object val = ctx.getAuthClaims().get(claimKey);
                    if (val != null) claimVal = val.toString();
                } else if (ctx.getWorkflowInjections() != null && ctx.getWorkflowInjections().containsKey(claimKey)) {
                    Object val = ctx.getWorkflowInjections().get(claimKey);
                    if (val != null) claimVal = val.toString();
                }
                if (claimVal != null && !claimVal.isBlank()) {
                    sb.append(':').append(claimKey).append('=').append(claimVal);
                }
            } else if ("query".equals(sLower)) {
                if (ctx.getQueryParams() != null && !ctx.getQueryParams().isEmpty()) {
                    StringBuilder qb = new StringBuilder();
                    new TreeMap<>(ctx.getQueryParams()).forEach((k, v) -> {
                        if (!"force_refresh".equalsIgnoreCase(k) && !"no_cache".equalsIgnoreCase(k)) {
                            if (qb.length() > 0) qb.append('&');
                            qb.append(k).append('=').append(v);
                        }
                    });
                    if (qb.length() > 0) {
                        sb.append(":q=").append(qb);
                    }
                }
            } else if (sLower.startsWith("header:")) {
                String hdrKey = strategy.substring(7);
                if (ctx.getHeaders() != null && ctx.getHeaders().containsKey(hdrKey.toLowerCase())) {
                    String val = ctx.getHeaders().get(hdrKey.toLowerCase());
                    if (val != null && !val.isBlank()) {
                        sb.append(':').append(hdrKey).append('=').append(val);
                    }
                }
            }
        }
        return sb.toString();
    }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        String method = ctx.getMethod() != null ? ctx.getMethod().toUpperCase() : "GET";
        boolean methodMatches = false;
        for (String m : methods) {
            if (m.equalsIgnoreCase(method)) {
                methodMatches = true;
                break;
            }
        }
        if (!methodMatches) return;
        if (shouldSkip(ctx.getPath())) return;

        if (cacheControlHeaders) {
            String cc = ctx.getHeaders().get("cache-control");
            boolean hasNoCacheQuery = ctx.getQueryParams() != null && 
                (ctx.getQueryParams().containsKey("no_cache") || ctx.getQueryParams().containsKey("force_refresh"));
            if ((cc != null && (cc.toLowerCase().contains("no-cache") || cc.toLowerCase().contains("no-store"))) || hasNoCacheQuery) {
                ctx.getResponseHeaders().put(headerName, "BYPASS");
                return;
            }
        }

        String cacheKey = buildCacheKey(ctx, ctx.getPath());
        ctx.getWorkflowInjections().put("octopus_cache_key", cacheKey);

        String cachedContent = null;
        Long remainingTtl = null;

        if (jedisPool != null) {
            try (Jedis jedis = jedisPool.getResource()) {
                String val = jedis.get(cacheKey);
                if (val != null && !val.isBlank()) {
                    cachedContent = val;
                    remainingTtl = jedis.ttl(cacheKey);
                }
            } catch (Exception ex) {
                System.err.println("⚠️ [RedisCacheMiddleware] Jedis get error: " + ex.getMessage());
            }
        }

        if (cachedContent == null) {
            CacheItem item = fallbackCache.get(cacheKey);
            if (item != null) {
                if (!item.isExpired()) {
                    cachedContent = item.getContent();
                    remainingTtl = Math.max(0, (item.getExpiryTimestampMs() - System.currentTimeMillis()) / 1000);
                } else {
                    fallbackCache.remove(cacheKey);
                }
            }
        }

        if (cachedContent != null) {
            ctx.getResponseHeaders().put(headerName, "HIT");
            ctx.getResponseHeaders().put("X-Cache-Key", cacheKey);
            if (remainingTtl != null && remainingTtl >= 0) {
                ctx.getResponseHeaders().put("X-Cache-TTL", String.valueOf(remainingTtl));
            }
            ctx.getResponseHeaders().put("X-Job-ID", "job_cache_hit_" + System.currentTimeMillis());
            ctx.getResponseHeaders().put("X-Execution-Status", "COMPLETED");

            try {
                Object parsed = mapper.readValue(cachedContent, Object.class);
                ctx.setShortCircuitResponse(parsed);
            } catch (Exception ignored) {
                ctx.setShortCircuitResponse(cachedContent);
            }
            ctx.setShortCircuitStatusCode(200);
            return;
        }

        ctx.getResponseHeaders().put(headerName, "MISS");
        ctx.getResponseHeaders().put("X-Cache-Key", cacheKey);
    }

    @Override
    public void processPost(LiveRequestContext ctx, Object responseData, int statusCode, double durationMs, ExecutionState state) {
        if (statusCode != 200 || responseData == null) return;
        String method = ctx.getMethod() != null ? ctx.getMethod().toUpperCase() : "GET";
        boolean methodMatches = false;
        for (String m : methods) {
            if (m.equalsIgnoreCase(method)) {
                methodMatches = true;
                break;
            }
        }
        if (!methodMatches) return;
        if (shouldSkip(ctx.getPath())) return;

        String cacheKey = (String) ctx.getWorkflowInjections().get("octopus_cache_key");
        if (cacheKey == null || cacheKey.isBlank()) {
            cacheKey = buildCacheKey(ctx, ctx.getPath());
        }

        try {
            String json;
            if (responseData instanceof String s) {
                json = s;
            } else {
                json = mapper.writeValueAsString(responseData);
            }

            fallbackCache.put(cacheKey, new CacheItem(json, System.currentTimeMillis() + (ttlSeconds * 1000L)));

            if (jedisPool != null) {
                try (Jedis jedis = jedisPool.getResource()) {
                    jedis.setex(cacheKey, ttlSeconds, json);
                }
            }
            System.out.println("[RedisCacheMiddleware] Successfully cached key: " + cacheKey + " (TTL: " + ttlSeconds + "s)");
        } catch (Exception ex) {
            System.err.println("⚠️ [RedisCacheMiddleware] Error caching response for key " + cacheKey + ": " + ex.getMessage());
        }
    }
}
