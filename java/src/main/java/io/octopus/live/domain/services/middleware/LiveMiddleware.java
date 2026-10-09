package io.octopus.live.domain.services.middleware;

import io.octopus.live.domain.models.MiddlewareExceptionConfig;
import io.octopus.live.domain.ports.IActivityExecutor;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;

public abstract class LiveMiddleware {
    protected String name;
    protected String type = "pre-request";
    protected String activity;
    protected boolean async = true;
    protected List<String> excludes = new ArrayList<>();
    protected List<String> includes = new ArrayList<>();
    protected List<MiddlewareExceptionConfig> exceptions = new ArrayList<>();
    protected IActivityExecutor activityExecutor;
    protected io.octopus.live.domain.ports.IAsyncActivityDispatcher asyncDispatcher;
    protected Map<String, Object> config = new HashMap<>();

    public LiveMiddleware(String name) {
        this.name = name;
    }

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

    public IActivityExecutor getActivityExecutor() { return activityExecutor; }
    public void setActivityExecutor(IActivityExecutor activityExecutor) { this.activityExecutor = activityExecutor; }

    public io.octopus.live.domain.ports.IAsyncActivityDispatcher getAsyncDispatcher() { return asyncDispatcher; }
    public void setAsyncDispatcher(io.octopus.live.domain.ports.IAsyncActivityDispatcher asyncDispatcher) { this.asyncDispatcher = asyncDispatcher; }

    public Map<String, Object> getConfig() { return config; }
    public void setConfig(Map<String, Object> config) { this.config = config != null ? config : new HashMap<>(); }

    public boolean shouldSkip(String path) {
        return RouteMatcher.shouldSkip(path, excludes, includes);
    }

    public MiddlewareExceptionConfig findExceptionConfig(Exception exc) {
        String excName = exc instanceof OctopusMiddlewareException ome ? ome.getErrorName() : exc.getClass().getSimpleName();
        for (MiddlewareExceptionConfig ec : exceptions) {
            if (ec.getException() != null && ec.getException().equalsIgnoreCase(excName)) {
                return ec;
            }
        }
        for (MiddlewareExceptionConfig ec : exceptions) {
            if (ec.getException() != null && exc.getClass().getSimpleName().equalsIgnoreCase(ec.getException())) {
                return ec;
            }
        }
        return null;
    }

    public void handleException(LiveRequestContext ctx, Exception exc) {
        MiddlewareExceptionConfig excCfg = findExceptionConfig(exc);
        if (excCfg != null && excCfg.getActivity() != null && !excCfg.getActivity().isBlank()) {
            Map<String, Object> activityPayload = new HashMap<>();
            String excName = exc instanceof OctopusMiddlewareException ome ? ome.getErrorName() : exc.getClass().getSimpleName();
            int statusCode = exc instanceof OctopusMiddlewareException ome ? ome.getStatusCode() : (excCfg.getCode() > 0 ? excCfg.getCode() : 500);
            String message = excCfg.getMessage() != null ? excCfg.getMessage() : exc.getMessage();

            activityPayload.put("exception", excName);
            activityPayload.put("error", excName);
            activityPayload.put("message", message);
            activityPayload.put("status_code", statusCode);
            activityPayload.put("path", ctx.getPath());
            activityPayload.put("route", ctx.getPath());
            activityPayload.put("method", ctx.getMethod());
            activityPayload.put("client_ip", ctx.getClientIp());
            activityPayload.put("client_host", ctx.getClientIp());
            activityPayload.put("headers", ctx.getHeaders());
            activityPayload.put("query_params", ctx.getQueryParams());
            if (ctx.getBody() != null) {
                activityPayload.put("data", ctx.getBody());
            }
            if (ctx.getAuthClaims() != null) {
                activityPayload.put("auth_claims", ctx.getAuthClaims());
            }
            String jobId = ctx.getHeaders().getOrDefault("x-job-id", "job_" + System.currentTimeMillis());
            activityPayload.put("job_id", jobId);

            if (excCfg.isAsync() && asyncDispatcher != null) {
                String stepId = "exc_" + excCfg.getActivity().toLowerCase();
                asyncDispatcher.dispatch(excCfg.getActivity(), activityPayload, jobId, stepId, activityExecutor);
            } else if (excCfg.isAsync()) {
                CompletableFuture.runAsync(() -> {
                    try {
                        if (activityExecutor != null) activityExecutor.executeActivity(excCfg.getActivity(), activityPayload);
                    } catch (Exception ignored) {}
                });
            } else {
                try {
                    if (activityExecutor != null) activityExecutor.executeActivity(excCfg.getActivity(), activityPayload);
                } catch (Exception ignored) {}
            }
        }
    }

    public abstract void process(LiveRequestContext ctx) throws OctopusMiddlewareException;

    public void processPost(LiveRequestContext ctx, Object responseData, int statusCode, double durationMs, io.octopus.live.domain.models.ExecutionState state) {
        if (shouldSkip(ctx.getPath())) return;

        if (activity != null && (activityExecutor != null || asyncDispatcher != null)) {
            String jobId = state != null && state.getJobId() != null ? state.getJobId() : ctx.getHeaders().getOrDefault("x-job-id", "job_unknown");
            String wfName = state != null && state.getWorkflowName() != null ? state.getWorkflowName() : "default";

            Map<String, Object> auditPayload = new HashMap<>();
            auditPayload.put("job_id", jobId);
            auditPayload.put("workflow_name", wfName);
            auditPayload.put("route", ctx.getPath());
            auditPayload.put("method", ctx.getMethod());
            auditPayload.put("status_code", statusCode);
            auditPayload.put("duration_ms", durationMs);
            auditPayload.put("client_ip", ctx.getClientIp());
            auditPayload.put("status", state != null && state.getStatus() != null ? state.getStatus().name() : (statusCode < 400 ? "COMPLETED" : "FAILED"));
            if (ctx.getAuthClaims() != null) {
                auditPayload.put("auth_claims", ctx.getAuthClaims());
                if (ctx.getAuthClaims().containsKey("user_id")) {
                    auditPayload.put("user_id", ctx.getAuthClaims().get("user_id"));
                }
                if (ctx.getAuthClaims().containsKey("customer_tier")) {
                    auditPayload.put("customer_tier", ctx.getAuthClaims().get("customer_tier"));
                }
            }
            if (responseData != null) {
                auditPayload.put("response_data", responseData);
            }

            if (async && asyncDispatcher != null) {
                String stepId = "post_audit_" + activity.toLowerCase();
                asyncDispatcher.dispatch(activity, auditPayload, jobId, stepId, activityExecutor);
            } else if (async) {
                CompletableFuture.runAsync(() -> {
                    try {
                        if (activityExecutor != null) activityExecutor.executeActivity(activity, auditPayload);
                    } catch (Exception e) {
                        System.err.println("⚠️ [POST-REQUEST AUDIT] Error executing " + activity + ": " + e.getMessage());
                    }
                });
            } else {
                try {
                    if (activityExecutor != null) activityExecutor.executeActivity(activity, auditPayload);
                } catch (Exception e) {
                    System.err.println("⚠️ [POST-REQUEST AUDIT] Error executing " + activity + ": " + e.getMessage());
                }
            }
        }
    }
}
