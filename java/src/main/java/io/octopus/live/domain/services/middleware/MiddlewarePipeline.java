package io.octopus.live.domain.services.middleware;

import io.octopus.live.domain.models.MiddlewareConfig;
import io.octopus.live.domain.ports.IActivityExecutor;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class MiddlewarePipeline {
    private final List<LiveMiddleware> middlewares = new ArrayList<>();
    private final IActivityExecutor activityExecutor;
    private final io.octopus.live.domain.ports.IAsyncActivityDispatcher asyncDispatcher;

    public MiddlewarePipeline(IActivityExecutor activityExecutor) {
        this(activityExecutor, null);
    }

    public MiddlewarePipeline(IActivityExecutor activityExecutor, io.octopus.live.domain.ports.IAsyncActivityDispatcher asyncDispatcher) {
        this.activityExecutor = activityExecutor;
        this.asyncDispatcher = asyncDispatcher;
    }

    public void addMiddleware(LiveMiddleware middleware) {
        if (middleware != null) {
            middleware.setActivityExecutor(activityExecutor);
            middleware.setAsyncDispatcher(asyncDispatcher);
            middlewares.add(middleware);
        }
    }

    public void buildFromBlueprintConfigs(List<MiddlewareConfig> configs) {
        if (configs == null || configs.isEmpty()) return;

        for (MiddlewareConfig mc : configs) {
            String name = mc.getName();
            if (name == null) continue;

            LiveMiddleware mw = null;
            if ("IpWhitelistMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("ipwhitelist")) {
                IpWhitelistMiddleware ipMw = new IpWhitelistMiddleware();
                ipMw.setAllowedIps(mc.getAllowedIps());
                ipMw.setBlockedIps(mc.getBlockedIps());
                mw = ipMw;
            } else if ("BearerAuthAndClaimsMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("bearerauth") || name.toLowerCase().contains("jwt")) {
                BearerAuthAndClaimsMiddleware jwtMw = new BearerAuthAndClaimsMiddleware();
                if (mc.getTokenHeader() != null && !mc.getTokenHeader().isBlank()) {
                    jwtMw.setTokenHeader(mc.getTokenHeader());
                }
                jwtMw.setClaimsToInject(mc.getClaimsToInject());
                mw = jwtMw;
            } else if ("RateLimitMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("ratelimit")) {
                RateLimitMiddleware rlMw = new RateLimitMiddleware();
                rlMw.setLimit(mc.getLimit());
                rlMw.setWindowSeconds(mc.getWindowSeconds());
                if (mc.getKeyBy() != null) rlMw.setKeyBy(mc.getKeyBy());
                rlMw.setTierLimits(mc.getTierLimits());
                mw = rlMw;
            } else if ("CorrelationIdMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("correlation")) {
                CorrelationIdMiddleware cMw = new CorrelationIdMiddleware();
                mw = cMw;
            } else if ("ApiKeyAuthMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("apikey")) {
                ApiKeyAuthMiddleware akMw = new ApiKeyAuthMiddleware();
                mw = akMw;
            } else if ("RedisCacheMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("cache")) {
                RedisCacheMiddleware rcMw = new RedisCacheMiddleware();
                if (mc.getTtlSeconds() > 0) rcMw.setTtlSeconds(mc.getTtlSeconds());
                if (mc.getKeyStrategy() != null && !mc.getKeyStrategy().isEmpty()) rcMw.setKeyStrategy(mc.getKeyStrategy());
                if (mc.getMethods() != null && !mc.getMethods().isEmpty()) rcMw.setMethods(mc.getMethods());
                if (mc.getHeaderName() != null && !mc.getHeaderName().isBlank()) rcMw.setHeaderName(mc.getHeaderName());
                if (mc.getPrefix() != null && !mc.getPrefix().isBlank()) rcMw.setPrefix(mc.getPrefix());
                rcMw.setCacheControlHeaders(mc.isCacheControlHeaders());
                if (mc.getRedisUrl() != null && !mc.getRedisUrl().isBlank()) rcMw.setRedisUrl(mc.getRedisUrl());
                mw = rcMw;
            } else if ("PayloadValidationMiddleware".equalsIgnoreCase(name) || "BadRequestMiddleware".equalsIgnoreCase(name) || "ValidationMiddleware".equalsIgnoreCase(name) || "InputValidationMiddleware".equalsIgnoreCase(name) || name.toLowerCase().contains("payloadvalidation") || name.toLowerCase().contains("inputvalidation") || name.toLowerCase().contains("badrequest")) {
                PayloadValidationMiddleware pvMw = new PayloadValidationMiddleware();
                long maxSize = mc.getMaxBodySizeBytes();
                if (maxSize <= 0 && mc.getConfig() != null) {
                    Object maxSizeObj = mc.getConfig().get("max_body_size_bytes");
                    if (maxSizeObj == null) maxSizeObj = mc.getConfig().get("max_size");
                    if (maxSizeObj instanceof Number n) maxSize = n.longValue();
                }
                pvMw.setMaxBodySizeBytes(maxSize);

                List<String> reqHdrs = mc.getRequiredHeaders();
                if ((reqHdrs == null || reqHdrs.isEmpty()) && mc.getConfig() != null) {
                    Object reqHdrsObj = mc.getConfig().get("required_headers");
                    if (reqHdrsObj instanceof List<?> list) {
                        reqHdrs = new ArrayList<>();
                        for (Object o : list) if (o != null) reqHdrs.add(String.valueOf(o));
                    }
                }
                pvMw.setRequiredHeaders(reqHdrs != null ? reqHdrs : new ArrayList<>());

                List<String> reqFields = mc.getRequiredFields();
                Map<String, List<String>> reqFieldsMap = mc.getRequiredFieldsMap();
                if ((reqFields == null || reqFields.isEmpty()) && (reqFieldsMap == null || reqFieldsMap.isEmpty()) && mc.getConfig() != null) {
                    Object reqFieldsObj = mc.getConfig().get("required_fields");
                    if (reqFieldsObj instanceof List<?> list) {
                        reqFields = new ArrayList<>();
                        for (Object o : list) if (o != null) reqFields.add(String.valueOf(o));
                    } else if (reqFieldsObj instanceof Map<?, ?> map) {
                        reqFieldsMap = new HashMap<>();
                        for (Map.Entry<?, ?> entry : map.entrySet()) {
                            String k = String.valueOf(entry.getKey());
                            List<String> fl = new ArrayList<>();
                            if (entry.getValue() instanceof List<?> l) {
                                for (Object o : l) if (o != null) fl.add(String.valueOf(o));
                            } else if (entry.getValue() != null) {
                                fl.add(String.valueOf(entry.getValue()));
                            }
                            reqFieldsMap.put(k, fl);
                        }
                    }
                }
                pvMw.setRequiredFields(reqFields != null ? reqFields : new ArrayList<>());
                pvMw.setRequiredFieldsMap(reqFieldsMap != null ? reqFieldsMap : new HashMap<>());
                mw = pvMw;
            } else if ("ResponseAuditMiddleware".equalsIgnoreCase(name) || "post-request".equalsIgnoreCase(mc.getType()) || name.toLowerCase().contains("audit")) {
                ResponseAuditMiddleware raMw = new ResponseAuditMiddleware();
                if (mc.getActivity() != null) raMw.setActivity(mc.getActivity());
                raMw.setAsync(mc.isAsync());
                mw = raMw;
            } else {
                // Generic fallback middleware
                final String mwName = name;
                mw = new LiveMiddleware(mwName) {
                    @Override
                    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {}
                };
            }

            if (mw != null) {
                if (mc.getType() != null) {
                    mw.setType(mc.getType());
                }
                if (mc.getActivity() != null) {
                    mw.setActivity(mc.getActivity());
                }
                mw.setAsync(mc.isAsync());
                mw.setExcludes(mc.getExcludes());
                mw.setIncludes(mc.getIncludes());
                mw.setExceptions(mc.getExceptions());
                mw.setConfig(mc.getConfig());
                mw.setActivityExecutor(activityExecutor);
                mw.setAsyncDispatcher(asyncDispatcher);
                middlewares.add(mw);
            }
        }
    }

    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        processPre(ctx);
    }

    public void processPre(LiveRequestContext ctx) throws OctopusMiddlewareException {
        for (LiveMiddleware mw : middlewares) {
            if ("post-request".equalsIgnoreCase(mw.getType())) {
                continue;
            }
            try {
                mw.process(ctx);
                if (ctx.isShortCircuited()) {
                    return;
                }
            } catch (OctopusMiddlewareException exc) {
                mw.handleException(ctx, exc);
                throw exc;
            }
        }
    }

    public void processPost(LiveRequestContext ctx, Object responseData, int statusCode, double durationMs, io.octopus.live.domain.models.ExecutionState state) {
        for (LiveMiddleware mw : middlewares) {
            if ("post-request".equalsIgnoreCase(mw.getType()) || mw instanceof RedisCacheMiddleware) {
                try {
                    mw.processPost(ctx, responseData, statusCode, durationMs, state);
                } catch (Exception e) {
                    System.err.println("⚠️ [POST-MIDDLEWARE] Error in " + mw.getName() + ": " + e.getMessage());
                }
            }
        }
    }
}
