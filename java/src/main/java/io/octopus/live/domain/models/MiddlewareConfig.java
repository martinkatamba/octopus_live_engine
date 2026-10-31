package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonAnySetter;
import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

@JsonIgnoreProperties(ignoreUnknown = true)
public class MiddlewareConfig {
    private String name;
    private String type = "pre-request";
    private String activity;
    private boolean async = true;
    private List<String> excludes = new ArrayList<>();
    private List<String> includes = new ArrayList<>();
    private List<MiddlewareExceptionConfig> exceptions = new ArrayList<>();
    private Map<String, Object> config = new HashMap<>();

    @JsonProperty("allowed_ips")
    private List<String> allowedIps = new ArrayList<>();
    @JsonProperty("blocked_ips")
    private List<String> blockedIps = new ArrayList<>();

    private int limit = 60;
    @JsonProperty("window_seconds")
    private int windowSeconds = 60;
    @JsonProperty("key_by")
    private String keyBy = "client_ip";
    @JsonProperty("tier_limits")
    private Map<String, Integer> tierLimits = new HashMap<>();

    @JsonProperty("token_header")
    private String tokenHeader = "Authorization";
    @JsonProperty("claims_to_inject")
    private List<String> claimsToInject = new ArrayList<>();

    @JsonProperty("ttl_seconds")
    private int ttlSeconds = 300;
    @JsonProperty("key_strategy")
    private List<String> keyStrategy = new ArrayList<>(List.of("route", "claim:user_id", "query"));
    @JsonProperty("methods")
    private List<String> methods = new ArrayList<>(List.of("GET"));
    @JsonProperty("header_name")
    private String headerName = "X-Cache";
    @JsonProperty("prefix")
    private String prefix = "octopus:cache";
    @JsonProperty("cache_control_headers")
    private boolean cacheControlHeaders = true;
    @JsonProperty("redis_url")
    private String redisUrl = "127.0.0.1:6379";

    public int getTtlSeconds() { return ttlSeconds; }
    public void setTtlSeconds(int ttlSeconds) { this.ttlSeconds = ttlSeconds; }

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
    public void setRedisUrl(String redisUrl) { this.redisUrl = redisUrl; }

    public String getName() { return name; }
    public void setName(String name) { this.name = name; }
    public String getType() { return type; }
    public void setType(String type) { this.type = type != null ? type : "pre-request"; }
    public String getActivity() { return activity; }
    public void setActivity(String activity) { this.activity = activity; }
    public boolean isAsync() { return async; }
    public void setAsync(boolean async) { this.async = async; }

    public List<String> getExcludes() { return excludes; }
    public void setExcludes(List<String> excludes) { this.excludes = excludes != null ? excludes : new ArrayList<>(); }

    public List<String> getIncludes() { return includes; }
    public void setIncludes(List<String> includes) { this.includes = includes != null ? includes : new ArrayList<>(); }

    public List<MiddlewareExceptionConfig> getExceptions() { return exceptions; }
    public void setExceptions(List<MiddlewareExceptionConfig> exceptions) { this.exceptions = exceptions != null ? exceptions : new ArrayList<>(); }

    public Map<String, Object> getConfig() { return config; }
    public void setConfig(Map<String, Object> config) {
        Map<String, Object> merged = config != null ? new HashMap<>(config) : new HashMap<>();
        // Top-level settings seen before config: keep them unless config: sets the same key.
        this.config.forEach(merged::putIfAbsent);
        this.config = merged;
    }

    // Settings written next to name: (e.g. valid_keys) belong to config:, as in the other SDKs.
    @JsonAnySetter
    public void setOtherSetting(String key, Object value) {
        config.putIfAbsent(key, value);
    }

    public List<String> getAllowedIps() { return allowedIps; }
    public void setAllowedIps(List<String> allowedIps) { this.allowedIps = allowedIps != null ? allowedIps : new ArrayList<>(); }

    public List<String> getBlockedIps() { return blockedIps; }
    public void setBlockedIps(List<String> blockedIps) { this.blockedIps = blockedIps != null ? blockedIps : new ArrayList<>(); }

    public int getLimit() { return limit; }
    public void setLimit(int limit) { this.limit = limit; }

    public int getWindowSeconds() { return windowSeconds; }
    public void setWindowSeconds(int windowSeconds) { this.windowSeconds = windowSeconds; }

    public String getKeyBy() { return keyBy; }
    public void setKeyBy(String keyBy) { this.keyBy = keyBy; }

    public Map<String, Integer> getTierLimits() { return tierLimits; }
    public void setTierLimits(Map<String, Integer> tierLimits) { this.tierLimits = tierLimits != null ? tierLimits : new HashMap<>(); }

    public String getTokenHeader() { return tokenHeader; }
    public void setTokenHeader(String tokenHeader) { this.tokenHeader = tokenHeader; }

    public List<String> getClaimsToInject() { return claimsToInject; }
    public void setClaimsToInject(List<String> claimsToInject) { this.claimsToInject = claimsToInject != null ? claimsToInject : new ArrayList<>(); }

    @JsonProperty("max_body_size_bytes")
    private long maxBodySizeBytes = 0;
    @JsonProperty("require_json_body")
    private boolean requireJsonBody = false;
    @JsonProperty("required_headers")
    private List<String> requiredHeaders = new ArrayList<>();
    @JsonProperty("required_fields")
    private Object requiredFields;
    private List<String> requiredFieldsList = new ArrayList<>();
    private Map<String, List<String>> requiredFieldsMap = new HashMap<>();

    public long getMaxBodySizeBytes() { return maxBodySizeBytes; }
    public void setMaxBodySizeBytes(long maxBodySizeBytes) { this.maxBodySizeBytes = maxBodySizeBytes; }

    public boolean isRequireJsonBody() { return requireJsonBody; }
    public void setRequireJsonBody(boolean requireJsonBody) { this.requireJsonBody = requireJsonBody; }

    public List<String> getRequiredHeaders() { return requiredHeaders; }
    public void setRequiredHeaders(List<String> requiredHeaders) { this.requiredHeaders = requiredHeaders != null ? requiredHeaders : new ArrayList<>(); }

    public List<String> getRequiredFields() { return requiredFieldsList; }
    public Map<String, List<String>> getRequiredFieldsMap() { return requiredFieldsMap; }

    @JsonProperty("required_fields")
    public void setRequiredFields(Object obj) {
        this.requiredFields = obj;
        if (obj instanceof List<?> list) {
            this.requiredFieldsList = new ArrayList<>();
            for (Object o : list) if (o != null) this.requiredFieldsList.add(String.valueOf(o));
        } else if (obj instanceof Map<?, ?> map) {
            this.requiredFieldsMap = new HashMap<>();
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                String k = String.valueOf(entry.getKey());
                List<String> fl = new ArrayList<>();
                if (entry.getValue() instanceof List<?> l) {
                    for (Object o : l) if (o != null) fl.add(String.valueOf(o));
                } else if (entry.getValue() != null) {
                    fl.add(String.valueOf(entry.getValue()));
                }
                this.requiredFieldsMap.put(k, fl);
            }
        }
    }
}
