package io.octopus.live.adapters.inbound.rest;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpHandler;
import com.sun.net.httpserver.HttpServer;
import io.octopus.live.LiveEngine;
import io.octopus.live.adapters.inbound.VisualizerHtmlRenderer;
import io.octopus.live.domain.models.*;
import io.octopus.live.domain.services.middleware.*;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class LiveHttpServer {

    private final LiveEngine engine;
    private final int port;
    private final ObjectMapper mapper = new ObjectMapper();
    private HttpServer server;

    public LiveHttpServer(LiveEngine engine, int port) {
        this.engine = engine;
        this.port = port;
    }

    public void start() throws IOException {
        server = HttpServer.create(new InetSocketAddress("0.0.0.0", port), 0);
        try {
            var method = Executors.class.getMethod("newVirtualThreadPerTaskExecutor");
            server.setExecutor((Executor) method.invoke(null));
        } catch (Exception e) {
            server.setExecutor(Executors.newCachedThreadPool());
        }

        server.createContext("/", new MainHttpHandler());
        server.start();
    }

    public void stop() {
        if (server != null) {
            server.stop(0);
        }
    }

    private class MainHttpHandler implements HttpHandler {
        @Override
        public void handle(HttpExchange exchange) throws IOException {
            URI uri = exchange.getRequestURI();
            String path = uri.getPath();
            String method = exchange.getRequestMethod().toUpperCase();

            try {
                // 1. Health
                if ("/health".equals(path) || "/api/v1.0.0/health".equals(path)) {
                    Map<String, Object> health = Map.of(
                        "status", "UP",
                        "engine", "OCTOPUS Live In-Memory Engine (Java)",
                        "mode", "sync",
                        "version", "1.0.0"
                    );
                    sendJson(exchange, 200, health);
                    return;
                }

                // 2. OpenAPI Spec & Swagger Docs
                if ("/openapi.json".equals(path)) {
                    List<WorkflowBlueprint> blueprints = engine.getVisualizerUseCase().listBlueprints();
                    Map<String, Object> paths = new LinkedHashMap<>();
                    Map<String, Object> schemas = new LinkedHashMap<>();

                    // Pre-register ErrorResponse schema
                    schemas.put("ErrorResponse", Map.of(
                        "type", "object",
                        "properties", Map.of(
                            "error", Map.of("type", "string", "example", "UserNotAuthenticated"),
                            "code", Map.of("type", "integer", "example", 401),
                            "message", Map.of("type", "string", "example", "Authentication required. Please provide a valid Bearer token via the Authorization header.")
                        )
                    ));

                    // Register all models from engine's modelRegistry
                    Map<String, Class<?>> modelRegistry = engine.getModelRegistry();
                    if (modelRegistry != null) {
                        for (Map.Entry<String, Class<?>> entry : modelRegistry.entrySet()) {
                            if (!schemas.containsKey(entry.getKey())) {
                                schemas.put(entry.getKey(), generateOpenAPISchemaFromJavaClass(entry.getValue(), schemas, modelRegistry));
                            }
                        }
                    }

                    // Discover security schemes from middlewares & engine config
                    Map<String, Object> securitySchemes = new LinkedHashMap<>();
                    List<String> bearerExcludes = new ArrayList<>();
                    List<String> bearerIncludes = new ArrayList<>();
                    boolean hasBearer = false;
                    boolean hasApiKey = false;

                    for (WorkflowBlueprint bp : blueprints) {
                        if (bp.getMiddlewares() != null) {
                            for (MiddlewareConfig mw : bp.getMiddlewares()) {
                                if (mw.getName() != null && (mw.getName().contains("Bearer") || mw.getName().contains("Auth") || mw.getName().contains("Jwt"))) {
                                    hasBearer = true;
                                    if (mw.getExcludes() != null) bearerExcludes.addAll(mw.getExcludes());
                                    if (mw.getIncludes() != null) bearerIncludes.addAll(mw.getIncludes());
                                }
                                if (mw.getName() != null && mw.getName().contains("ApiKey")) {
                                    hasApiKey = true;
                                }
                            }
                        }
                    }

                    if (engine.getApiKey() != null && !engine.getApiKey().isEmpty()) {
                        hasApiKey = true;
                    }

                    if (engine.getCustomMiddlewares() != null) {
                        for (LiveMiddleware mw : engine.getCustomMiddlewares()) {
                            if (mw.getName() != null && (mw.getName().contains("Bearer") || mw.getName().contains("Auth") || mw.getName().contains("Jwt"))) {
                                hasBearer = true;
                                if (mw.getExcludes() != null) bearerExcludes.addAll(mw.getExcludes());
                                if (mw.getIncludes() != null) bearerIncludes.addAll(mw.getIncludes());
                            }
                            if (mw.getName() != null && mw.getName().contains("ApiKey")) {
                                hasApiKey = true;
                            }
                        }
                    }

                    if (hasBearer) {
                        Map<String, Object> bearerScheme = new LinkedHashMap<>();
                        bearerScheme.put("type", "http");
                        bearerScheme.put("scheme", "bearer");
                        bearerScheme.put("bearerFormat", "JWT");
                        bearerScheme.put("description", "Enter your Bearer JWT token (e.g. obtained via /auth/login)");
                        securitySchemes.put("HTTPBearer", bearerScheme);
                    }
                    if (hasApiKey) {
                        Map<String, Object> apiKeyScheme = new LinkedHashMap<>();
                        apiKeyScheme.put("type", "apiKey");
                        apiKeyScheme.put("in", "header");
                        apiKeyScheme.put("name", "X-API-Key");
                        apiKeyScheme.put("description", "Optional API Key for client authorization");
                        securitySchemes.put("ApiKeyAuth", apiKeyScheme);
                    }

                    for (WorkflowBlueprint bp : blueprints) {
                        if (bp.getApi() != null) {
                            for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                                String r = ep.getRoute();
                                String m = (ep.getMethod() != null ? ep.getMethod() : "POST").toLowerCase();
                                @SuppressWarnings("unchecked")
                                Map<String, Object> methodsMap = (Map<String, Object>) paths.computeIfAbsent(r, k -> new LinkedHashMap<String, Object>());

                                Map<String, Object> op = new LinkedHashMap<>();
                                op.put("summary", ep.getSummary());
                                op.put("description", ep.getDescription());
                                op.put("tags", ep.getTags());

                                // 1. Parameters
                                List<Map<String, Object>> params = new ArrayList<>();
                                params.add(Map.of(
                                    "name", "X-Job-ID",
                                    "in", "header",
                                    "required", false,
                                    "description", "Optional custom Job ID for idempotency and live tracing",
                                    "schema", Map.of("type", "string")
                                ));
                                op.put("parameters", params);

                                // 2. Request Body
                                if (("post".equals(m) || "put".equals(m) || "patch".equals(m)) && ep.getInputModel() != null && !ep.getInputModel().isEmpty()) {
                                    String modelName = ep.getInputModel();
                                    if (!schemas.containsKey(modelName)) {
                                        Class<?> modelCls = engine.getModel(modelName);
                                        if (modelCls != null) {
                                            schemas.put(modelName, generateOpenAPISchemaFromJavaClass(modelCls, schemas, modelRegistry));
                                        } else {
                                            schemas.put(modelName, Map.of(
                                                "type", "object",
                                                "description", "Auto-declared schema for " + modelName
                                            ));
                                        }
                                    }
                                    op.put("requestBody", Map.of(
                                        "required", true,
                                        "description", "Request payload for " + modelName,
                                        "content", Map.of(
                                            "application/json", Map.of(
                                                "schema", Map.of("$ref", "#/components/schemas/" + modelName)
                                            )
                                        )
                                    ));
                                }

                                // 3. Responses
                                int statusCode = ep.getStatusCode() > 0 ? ep.getStatusCode() : 200;
                                Map<String, Object> responses = new LinkedHashMap<>();
                                Map<String, Object> successResp = new LinkedHashMap<>();
                                successResp.put("description", "Successful operation");

                                if (ep.getOutputModel() != null && !ep.getOutputModel().isEmpty()) {
                                    String modelName = ep.getOutputModel();
                                    if (!schemas.containsKey(modelName)) {
                                        Class<?> modelCls = engine.getModel(modelName);
                                        if (modelCls != null) {
                                            schemas.put(modelName, generateOpenAPISchemaFromJavaClass(modelCls, schemas, modelRegistry));
                                        } else {
                                            schemas.put(modelName, Map.of(
                                                "type", "object",
                                                "description", "Auto-declared schema for " + modelName
                                            ));
                                        }
                                    }
                                    successResp.put("content", Map.of(
                                        "application/json", Map.of(
                                            "schema", Map.of("$ref", "#/components/schemas/" + modelName)
                                        )
                                    ));
                                }
                                responses.put(String.valueOf(statusCode), successResp);

                                boolean isExcluded = RouteMatcher.shouldSkip(r, bearerExcludes, bearerIncludes);
                                if (hasBearer && !isExcluded) {
                                    responses.put("401", Map.of(
                                        "description", "Unauthorized - Missing or invalid Bearer JWT token",
                                        "content", Map.of(
                                            "application/json", Map.of(
                                                "schema", Map.of("$ref", "#/components/schemas/ErrorResponse")
                                            )
                                        )
                                    ));
                                    responses.put("403", Map.of(
                                        "description", "Forbidden - Insufficient permissions or IP blocked",
                                        "content", Map.of(
                                            "application/json", Map.of(
                                                "schema", Map.of("$ref", "#/components/schemas/ErrorResponse")
                                            )
                                        )
                                    ));
                                    responses.put("429", Map.of(
                                        "description", "Too Many Requests - Rate limit exceeded",
                                        "content", Map.of(
                                            "application/json", Map.of(
                                                "schema", Map.of("$ref", "#/components/schemas/ErrorResponse")
                                            )
                                        )
                                    ));
                                }
                                op.put("responses", responses);

                                // 4. Security
                                if (hasBearer) {
                                    if (!isExcluded) {
                                        op.put("security", List.of(Map.of("HTTPBearer", List.of())));
                                    } else {
                                        op.put("security", List.of());
                                    }
                                } else if (hasApiKey) {
                                    op.put("security", List.of(Map.of("ApiKeyAuth", List.of())));
                                }

                                methodsMap.put(m, op);
                            }
                        }
                    }

                    Map<String, Object> components = new LinkedHashMap<>();
                    components.put("schemas", schemas);
                    if (!securitySchemes.isEmpty()) {
                        components.put("securitySchemes", securitySchemes);
                    }

                    Map<String, Object> openApi = new LinkedHashMap<>();
                    openApi.put("openapi", "3.0.0");
                    openApi.put("info", Map.of(
                        "title", "OCTOPUS Live Engine API",
                        "version", "1.0.0",
                        "description", "Declarative Model-Driven In-Memory Synchronous API Framework (Java)"
                    ));
                    openApi.put("paths", paths);
                    openApi.put("components", components);
                    sendJson(exchange, 200, openApi);
                    return;
                }

                if ("/docs".equals(path)) {
                    String html = """
                    <!DOCTYPE html>
                    <html>
                    <head>
                        <title>OCTOPUS Live Engine API Docs</title>
                        <link rel="stylesheet" type="text/css" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
                    </head>
                    <body>
                        <div id="swagger-ui"></div>
                        <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
                        <script>
                            window.onload = () => {
                                window.ui = SwaggerUIBundle({
                                    url: '/openapi.json',
                                    dom_id: '#swagger-ui',
                                    persistAuthorization: true,
                                    displayRequestDuration: true,
                                    filter: true,
                                    deepLinking: true
                                });
                            };
                        </script>
                    </body>
                    </html>
                    """;
                    sendHtml(exchange, 200, html);
                    return;
                }

                // 3. Visualizer UI
                if ("/visualizer".equals(path)) {
                    Map<String, String> queryParams = parseQueryParams(uri.getQuery());
                    List<WorkflowBlueprint> blueprints = engine.getVisualizerUseCase().listBlueprints();
                    String domain = queryParams.getOrDefault("domain", blueprints.isEmpty() ? "default" : blueprints.get(0).getWorkflowName());
                    String prefix = "/api/v1.0.0/" + domain;
                    String html = VisualizerHtmlRenderer.renderHtml(domain, prefix);
                    sendHtml(exchange, 200, html);
                    return;
                }

                // 4. Domains Registry
                if ("/api/visualizer/domains".equals(path)) {
                    List<WorkflowBlueprint> blueprints = engine.getVisualizerUseCase().listBlueprints();
                    List<Map<String, Object>> result = new ArrayList<>();
                    for (WorkflowBlueprint bp : blueprints) {
                        Map<String, Object> item = new HashMap<>();
                        item.put("domain", bp.getWorkflowName());
                        item.put("namespace", bp.getNamespace());
                        item.put("prefix", "/api/v" + bp.getVersion() + "/" + bp.getWorkflowName());
                        item.put("workflow_name", bp.getWorkflowName());
                        item.put("version", bp.getVersion());
                        item.put("mode", bp.getMode());
                        item.put("description", bp.getDescription());

                        List<Map<String, Object>> endpointsList = new ArrayList<>();
                        if (bp.getApi() != null) {
                            for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                                Map<String, Object> epMap = new HashMap<>();
                                epMap.put("route", ep.getRoute());
                                epMap.put("method", ep.getMethod());
                                epMap.put("input_model", ep.getInputModel());
                                epMap.put("output_model", ep.getOutputModel());
                                epMap.put("status_code", ep.getStatusCode());
                                epMap.put("summary", ep.getSummary());
                                epMap.put("tags", ep.getTags());
                                endpointsList.add(epMap);
                            }
                        }
                        item.put("api", endpointsList);
                        item.put("api_endpoints", endpointsList);
                        result.add(item);
                    }
                    sendJson(exchange, 200, result);
                    return;
                }

                // 5. Resolve Job
                if ("/api/visualizer/resolve-job".equals(path)) {
                    Map<String, String> queryParams = parseQueryParams(uri.getQuery());
                    String jobId = queryParams.get("job_id");
                    if (jobId == null || jobId.isBlank()) {
                        sendJson(exchange, 200, Map.of("exists", false, "job_id", ""));
                        return;
                    }
                    ExecutionState state = engine.getWorkflowUseCase().getExecutionState(jobId);
                    if (state == null) {
                        sendJson(exchange, 200, Map.of("exists", false, "job_id", jobId));
                        return;
                    }
                    Map<String, Object> resp = new HashMap<>();
                    resp.put("exists", true);
                    resp.put("job_id", jobId);
                    resp.put("domain", state.getWorkflowName());
                    resp.put("workflow_name", state.getWorkflowName());
                    resp.put("route", state.getRoute());
                    resp.put("method", state.getMethod());
                    resp.put("status", state.getStatus().name());
                    resp.put("overall_status", state.getStatus().name());
                    resp.put("prefix", "/api/v1.0.0/" + state.getWorkflowName());
                    resp.put("belongs_to_service", true);
                    sendJson(exchange, 200, resp);
                    return;
                }

                // 5b. Visualizer / Global Jobs List
                if ("/api/visualizer/jobs".equals(path) || "/api/jobs".equals(path)) {
                    Map<String, String> queryParams = parseQueryParams(uri.getQuery());
                    String wf = queryParams.get("workflow");
                    if (wf == null) wf = queryParams.get("workflow_name");
                    if (wf == null) wf = queryParams.get("domain");
                    String limitStr = queryParams.get("limit");
                    int limit = 100;
                    if (limitStr != null && !limitStr.isBlank()) {
                        try { limit = Integer.parseInt(limitStr); } catch (Exception ignored) {}
                    }

                    List<ExecutionState> states;
                    if (engine.getStateStore() != null) {
                        states = engine.getStateStore().listStates(wf, limit);
                    } else {
                        states = engine.getWorkflowUseCase().listExecutionStates(wf, limit);
                    }

                    List<Map<String, Object>> result = new ArrayList<>();
                    for (ExecutionState s : states) {
                        Map<String, Object> item = new HashMap<>();
                        item.put("job_id", s.getJobId());
                        item.put("workflow_name", s.getWorkflowName());
                        item.put("workflow", s.getWorkflowName());
                        item.put("route", s.getRoute() != null ? s.getRoute() : "");
                        item.put("route_id", s.getRoute() != null ? s.getRoute() : "");
                        item.put("method", s.getMethod() != null ? s.getMethod() : "POST");
                        item.put("route_method", s.getMethod() != null ? s.getMethod() : "POST");
                        item.put("status", s.getStatus() != null ? s.getStatus().name() : "IDLE");
                        int sc = s.getErrorCode() != null ? s.getErrorCode() : (s.getStatus() == ExecutionStatus.COMPLETED ? 200 : (s.getStatus() == ExecutionStatus.FAILED ? 500 : 0));
                        item.put("status_code", sc);
                        item.put("source_ip", s.getClientHost() != null ? s.getClientHost() : "127.0.0.1");
                        item.put("client_host", s.getClientHost() != null ? s.getClientHost() : "127.0.0.1");
                        double dur = s.getDurationMs() != null ? Math.round(s.getDurationMs() * 100.0) / 100.0 : 0.0;
                        item.put("duration_ms", dur);
                        item.put("execution_time_ms", dur);
                        item.put("started_at", s.getStartedAt());
                        item.put("finished_at", s.getFinishedAt());
                        item.put("error", s.getError());
                        item.put("error_type", s.getErrorType());
                        result.add(item);
                    }
                    sendJson(exchange, 200, result);
                    return;
                }

                // 6. Custom Declared Endpoints across blueprints
                List<WorkflowBlueprint> blueprints = engine.getVisualizerUseCase().listBlueprints();
                for (WorkflowBlueprint bp : blueprints) {
                    if (bp.getApi() != null) {
                        for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                            String epMethod = ep.getMethod() != null ? ep.getMethod().toUpperCase() : "POST";
                            Map<String, String> pathParams = matchRoute(ep.getRoute(), path);
                            if (pathParams == null && bp.getWorkflowName() != null && ep.getRoute() != null) {
                                String prefix = "/api/v" + (bp.getVersion() != null ? bp.getVersion() : "1.0.0") + "/" + bp.getWorkflowName();
                                String epRoute = ep.getRoute().startsWith("/") ? ep.getRoute() : "/" + ep.getRoute();
                                pathParams = matchRoute(prefix + epRoute, path);
                            }
                            if (pathParams != null && epMethod.equalsIgnoreCase(method)) {
                                handleCustomEndpoint(exchange, bp, ep, path, method, uri, pathParams);
                                return;
                            }
                        }
                    }
                }

                // 7. Dynamic Blueprint RPC Routes: /api/v{version}/{workflowName}/...
                for (WorkflowBlueprint bp : blueprints) {
                    String prefix = "/api/v" + bp.getVersion() + "/" + bp.getWorkflowName();
                    if (path.startsWith(prefix)) {
                        String subPath = path.substring(prefix.length());
                        handleBlueprintRpcRoute(exchange, bp, subPath, method, uri);
                        return;
                    }
                }

                sendJson(exchange, 404, Map.of("error", "Not Found: " + path));
            } catch (Exception ex) {
                sendJson(exchange, 500, Map.of("error", ex.getMessage() != null ? ex.getMessage() : ex.toString()));
            }
        }

        private LiveRequestContext buildRequestContext(HttpExchange exchange, String path, String method, Map<String, Object> payload) {
            LiveRequestContext ctx = new LiveRequestContext();
            ctx.setPath(path);
            ctx.setMethod(method);
            ctx.setBody(payload != null ? payload : new HashMap<>());

            Map<String, String> headers = new HashMap<>();
            for (Map.Entry<String, List<String>> entry : exchange.getRequestHeaders().entrySet()) {
                if (entry.getValue() != null && !entry.getValue().isEmpty()) {
                    headers.put(entry.getKey(), entry.getValue().get(0));
                    headers.put(entry.getKey().toLowerCase(), entry.getValue().get(0));
                }
            }
            ctx.setHeaders(headers);

            String clientIp = headers.get("x-forwarded-for");
            if (clientIp != null && !clientIp.isBlank()) {
                clientIp = clientIp.split(",")[0].trim();
            } else if (headers.get("x-real-ip") != null && !headers.get("x-real-ip").isBlank()) {
                clientIp = headers.get("x-real-ip").trim();
            } else if (exchange.getRemoteAddress() != null && exchange.getRemoteAddress().getAddress() != null) {
                clientIp = exchange.getRemoteAddress().getAddress().getHostAddress();
            } else {
                clientIp = "127.0.0.1";
            }
            ctx.setClientIp(clientIp);

            return ctx;
        }

        private boolean runMiddlewares(HttpExchange exchange, WorkflowBlueprint bp, LiveRequestContext ctx) throws IOException {
            MiddlewarePipeline pipeline = new MiddlewarePipeline(engine.getActivityExecutor(), engine.getAsyncDispatcher());
            for (LiveMiddleware cm : engine.getCustomMiddlewares()) {
                pipeline.addMiddleware(cm);
            }
            if (bp != null && bp.getMiddlewares() != null) {
                pipeline.buildFromBlueprintConfigs(bp.getMiddlewares());
            }

            try {
                pipeline.processPre(ctx);
                for (Map.Entry<String, String> rh : ctx.getResponseHeaders().entrySet()) {
                    exchange.getResponseHeaders().set(rh.getKey(), rh.getValue());
                }
                if (ctx.isShortCircuited()) {
                    sendJson(exchange, ctx.getShortCircuitStatusCode(), ctx.getShortCircuitResponse());
                    return false;
                }
                return true;
            } catch (OctopusMiddlewareException exc) {
                handleMiddlewareException(exchange, bp, ctx, exc);
                return false;
            }
        }

        private void handleMiddlewareException(HttpExchange exchange, WorkflowBlueprint bp, LiveRequestContext ctx, OctopusMiddlewareException exc) throws IOException {
            for (Map.Entry<String, String> h : exc.getHeaders().entrySet()) {
                exchange.getResponseHeaders().set(h.getKey(), h.getValue());
            }
            System.err.printf("🚨 [SECURITY AUDIT] %s: %s %s from %s (%s)%n", exc.getDetail(), ctx != null ? ctx.getMethod() : exchange.getRequestMethod(), ctx != null ? ctx.getPath() : exchange.getRequestURI().getPath(), ctx != null ? ctx.getClientIp() : "127.0.0.1", exc.getErrorName());
            Map<String, Object> errBody = new HashMap<>();
            errBody.put("error", exc.getErrorName());
            errBody.put("detail", exc.getDetail());
            errBody.put("status_code", exc.getStatusCode());
            if (exc.getDetails() != null && !exc.getDetails().isEmpty()) {
                errBody.put("details", exc.getDetails());
            }
            String reqJobId = ctx != null && ctx.getHeaders() != null ? ctx.getHeaders().get("x-job-id") : exchange.getRequestHeaders().getFirst("X-Job-ID");
            if (reqJobId != null && !reqJobId.isBlank()) {
                ExecutionState failedState = new ExecutionState();
                failedState.setJobId(reqJobId);
                failedState.setWorkflowName(bp != null ? bp.getWorkflowName() : "default");
                failedState.setStatus(ExecutionStatus.FAILED);
                failedState.setError(exc.getDetail());
                failedState.setErrorType(exc.getErrorName());
                failedState.setErrorCode(exc.getStatusCode());
                failedState.setRoute(ctx != null ? ctx.getPath() : exchange.getRequestURI().getPath());
                failedState.setMethod(ctx != null ? ctx.getMethod() : exchange.getRequestMethod());
                failedState.setClientHost(ctx != null ? ctx.getClientIp() : "127.0.0.1");
                failedState.setStartedAt(java.time.Instant.now().toString());
                failedState.setFinishedAt(java.time.Instant.now().toString());
                
                Map<String, Object> excPayload = new HashMap<>();
                excPayload.put("exception", exc.getErrorName());
                excPayload.put("error", exc.getErrorName());
                excPayload.put("status_code", exc.getStatusCode());
                excPayload.put("detail", exc.getDetail());
                excPayload.put("client_ip", ctx != null ? ctx.getClientIp() : "127.0.0.1");
                failedState.setData(excPayload);

                StepExecutionTrace trace = new StepExecutionTrace();
                trace.setStepId("exc_" + exc.getErrorName().toLowerCase());
                trace.setActivityName("LogSecurityAuditViolation");
                trace.setStatus("COMPLETED");
                trace.setDurationMs(0.5);
                trace.setInputPayload(excPayload);
                failedState.getTraces().add(trace);
                if (engine.getStateStore() != null) {
                    engine.getStateStore().saveState(failedState);
                }
            }

            sendJson(exchange, exc.getStatusCode(), errBody);
        }

        private void runPostMiddlewares(HttpExchange exchange, WorkflowBlueprint bp, LiveRequestContext ctx, Object responseData, int statusCode, double durationMs, ExecutionState state) {
            try {
                MiddlewarePipeline pipeline = new MiddlewarePipeline(engine.getActivityExecutor(), engine.getAsyncDispatcher());
                for (LiveMiddleware cm : engine.getCustomMiddlewares()) {
                    pipeline.addMiddleware(cm);
                }
                if (bp != null && bp.getMiddlewares() != null) {
                    pipeline.buildFromBlueprintConfigs(bp.getMiddlewares());
                }
                pipeline.processPost(ctx, responseData, statusCode, durationMs, state);
                for (Map.Entry<String, String> rh : ctx.getResponseHeaders().entrySet()) {
                    exchange.getResponseHeaders().set(rh.getKey(), rh.getValue());
                }
            } catch (Exception e) {
                System.err.println("⚠️ [POST-MIDDLEWARE] Exception in post pipeline: " + e.getMessage());
            }
        }

        private void handleCustomEndpoint(
            HttpExchange exchange,
            WorkflowBlueprint bp,
            WorkflowBlueprint.ApiEndpointConfig ep,
            String path,
            String method,
            URI uri,
            Map<String, String> pathParams
        ) throws IOException {
            if (!checkRole(exchange, bp)) {
                String reqJobId = exchange.getRequestHeaders().getFirst("X-Job-ID");
                if (reqJobId != null && !reqJobId.isBlank()) {
                    ExecutionState failedState = new ExecutionState();
                    failedState.setJobId(reqJobId);
                    failedState.setWorkflowName(bp != null ? bp.getWorkflowName() : "default");
                    failedState.setStatus(ExecutionStatus.FAILED);
                    failedState.setError("INITIATION_ACCESS_DENIED: User not authorized for workflow '" + (bp != null ? bp.getWorkflowName() : "") + "'");
                    failedState.setErrorType("AccessDeniedException");
                    failedState.setErrorCode(403);
                    failedState.setRoute(path);
                    failedState.setMethod(method);
                    failedState.setClientHost(exchange.getRemoteAddress() != null && exchange.getRemoteAddress().getAddress() != null ? exchange.getRemoteAddress().getAddress().getHostAddress() : "127.0.0.1");
                    failedState.setStartedAt(java.time.Instant.now().toString());
                    failedState.setFinishedAt(java.time.Instant.now().toString());
                    if (engine.getStateStore() != null) {
                        engine.getStateStore().saveState(failedState);
                    }
                }
                sendJson(exchange, 403, Map.of("detail", "INITIATION_ACCESS_DENIED: User not authorized for workflow '" + bp.getWorkflowName() + "'"));
                return;
            }

            LiveRequestContext ctx = buildRequestContext(exchange, path, method, new HashMap<>());
            Map<String, Object> payload = new HashMap<>();
            if ("POST".equalsIgnoreCase(method) || "PUT".equalsIgnoreCase(method) || "PATCH".equalsIgnoreCase(method)) {
                try {
                    payload = readJsonBody(exchange, ctx);
                } catch (OctopusMiddlewareException exc) {
                    handleMiddlewareException(exchange, bp, ctx, exc);
                    return;
                }
            }

            // Ingest query parameters
            Map<String, String> qp = parseQueryParams(uri.getQuery());
            payload.putAll(qp);

            // Ingest path parameters
            payload.putAll(pathParams);
            ctx.setBody(payload);

            if (!runMiddlewares(exchange, bp, ctx)) {
                return;
            }

            if (ctx.getWorkflowInjections() != null) {
                payload.putAll(ctx.getWorkflowInjections());
            }

            String jobId = extractOrCreateJobId(exchange, payload, ctx);
            payload.put("job_id", jobId);

            long startNano = System.nanoTime();
            try {
                ExecutionState state = engine.executeEndpointSync(bp.getWorkflowName(), ep, path, method, payload);
                double durationMs = (System.nanoTime() - startNano) / 1_000_000.0;

                state.setClientHost(ctx.getClientIp());
                state.setRoute(path);
                state.setMethod(method);
                state.setDurationMs(durationMs);
                if (engine.getStateStore() != null) {
                    engine.getStateStore().saveState(state);
                }

                exchange.getResponseHeaders().set("X-Job-ID", state.getJobId());
                exchange.getResponseHeaders().set("X-Execution-Status", state.getStatus().name());
                exchange.getResponseHeaders().set("X-Execution-Duration-Ms", String.format(Locale.US, "%.2f", durationMs));

                int sc = ep.getStatusCode() > 0 ? ep.getStatusCode() : 200;
                Object finalPayload = state.getOutputData() != null ? state.getOutputData() : state.getData();
                if (ep.getOutputModel() != null && !ep.getOutputModel().isBlank()) {
                    Class<?> modelCls = engine.getModel(ep.getOutputModel());
                    if (modelCls != null) {
                        try {
                            finalPayload = mapper.convertValue(finalPayload, modelCls);
                        } catch (Exception ignored) {}
                    }
                }

                // Execute Post-Request Middlewares (Auditing, Metrics, etc.)
                runPostMiddlewares(exchange, bp, ctx, finalPayload, sc, durationMs, state);

                if ("true".equals(qp.get("raw")) || "1".equals(qp.get("raw"))) {
                    Map<String, Object> rawResp = new HashMap<>();
                    rawResp.put("job_id", state.getJobId());
                    rawResp.put("workflow_name", state.getWorkflowName());
                    rawResp.put("status", state.getStatus().name());
                    rawResp.put("route", state.getRoute());
                    rawResp.put("method", state.getMethod());
                    rawResp.put("output_data", state.getOutputData());
                    rawResp.put("data", state.getData());
                    rawResp.put("traces", state.getTraces());
                    sendJson(exchange, sc, rawResp);
                } else {
                    sendJson(exchange, sc, finalPayload);
                }
            } catch (Exception ex) {
                handleExecutionException(exchange, bp, path, method, ctx, jobId, startNano, ex);
            }
        }

        private void handleBlueprintRpcRoute(
            HttpExchange exchange,
            WorkflowBlueprint bp,
            String subPath,
            String method,
            URI uri
        ) throws IOException {
            String wfName = bp.getWorkflowName();

            // Execute POST
            if ("/execute".equals(subPath) && "POST".equalsIgnoreCase(method)) {
                if (!checkRole(exchange, bp)) {
                    String reqJobId = exchange.getRequestHeaders().getFirst("X-Job-ID");
                    if (reqJobId != null && !reqJobId.isBlank()) {
                        ExecutionState failedState = new ExecutionState();
                        failedState.setJobId(reqJobId);
                        failedState.setWorkflowName(wfName);
                        failedState.setStatus(ExecutionStatus.FAILED);
                        failedState.setError("INITIATION_ACCESS_DENIED: User not authorized for workflow '" + wfName + "'");
                        failedState.setErrorType("AccessDeniedException");
                        failedState.setErrorCode(403);
                        failedState.setRoute(uri.getPath());
                        failedState.setMethod(method);
                        failedState.setClientHost(exchange.getRemoteAddress() != null && exchange.getRemoteAddress().getAddress() != null ? exchange.getRemoteAddress().getAddress().getHostAddress() : "127.0.0.1");
                        failedState.setStartedAt(java.time.Instant.now().toString());
                        failedState.setFinishedAt(java.time.Instant.now().toString());
                        if (engine.getStateStore() != null) {
                            engine.getStateStore().saveState(failedState);
                        }
                    }
                    sendJson(exchange, 403, Map.of("detail", "INITIATION_ACCESS_DENIED: User not authorized for workflow '" + wfName + "'"));
                    return;
                }
                LiveRequestContext ctx = buildRequestContext(exchange, uri.getPath(), method, new HashMap<>());
                Map<String, Object> payload = new HashMap<>();
                try {
                    payload = readJsonBody(exchange, ctx);
                    ctx.setBody(payload);
                } catch (OctopusMiddlewareException exc) {
                    handleMiddlewareException(exchange, bp, ctx, exc);
                    return;
                }

                if (!runMiddlewares(exchange, bp, ctx)) {
                    return;
                }

                if (ctx.getWorkflowInjections() != null) {
                    payload.putAll(ctx.getWorkflowInjections());
                }

                String jobId = extractOrCreateJobId(exchange, payload, ctx);
                payload.put("job_id", jobId);

                long startNano = System.nanoTime();
                try {
                    ExecutionState state = engine.executeSync(wfName, payload);
                    double durationMs = (System.nanoTime() - startNano) / 1_000_000.0;

                    state.setClientHost(ctx.getClientIp());
                    state.setRoute(uri.getPath());
                    state.setMethod(method);
                    state.setDurationMs(durationMs);
                    if (engine.getStateStore() != null) {
                        engine.getStateStore().saveState(state);
                    }

                    exchange.getResponseHeaders().set("X-Job-ID", state.getJobId());
                    exchange.getResponseHeaders().set("X-Execution-Status", state.getStatus().name());
                    exchange.getResponseHeaders().set("X-Execution-Duration-Ms", String.format(Locale.US, "%.2f", durationMs));

                    Map<String, String> qp = parseQueryParams(uri.getQuery());
                    Object finalPayload = state.getOutputData() != null ? state.getOutputData() : state.getData();

                    // Execute Post-Request Middlewares (Auditing, Metrics, etc.)
                    runPostMiddlewares(exchange, bp, ctx, finalPayload, 200, durationMs, state);

                    if ("true".equals(qp.get("raw")) || "1".equals(qp.get("raw"))) {
                        Map<String, Object> rawResp = new HashMap<>();
                        rawResp.put("job_id", state.getJobId());
                        rawResp.put("workflow_name", state.getWorkflowName());
                        rawResp.put("status", state.getStatus().name());
                        rawResp.put("output_data", state.getOutputData());
                        rawResp.put("data", state.getData());
                        rawResp.put("traces", state.getTraces());
                        sendJson(exchange, 200, rawResp);
                    } else {
                        sendJson(exchange, 200, finalPayload);
                    }
                } catch (Exception ex) {
                    handleExecutionException(exchange, bp, uri.getPath(), method, ctx, jobId, startNano, ex);
                }
                return;
            }

            // Initiate POST
            if ("/initiate".equals(subPath) && "POST".equalsIgnoreCase(method)) {
                if (!checkRole(exchange, bp)) {
                    String reqJobId = exchange.getRequestHeaders().getFirst("X-Job-ID");
                    if (reqJobId != null && !reqJobId.isBlank()) {
                        ExecutionState failedState = new ExecutionState();
                        failedState.setJobId(reqJobId);
                        failedState.setWorkflowName(wfName);
                        failedState.setStatus(ExecutionStatus.FAILED);
                        failedState.setError("INITIATION_ACCESS_DENIED: User not authorized for workflow '" + wfName + "'");
                        failedState.setErrorType("AccessDeniedException");
                        failedState.setErrorCode(403);
                        failedState.setRoute(uri.getPath());
                        failedState.setMethod(method);
                        failedState.setClientHost(exchange.getRemoteAddress() != null && exchange.getRemoteAddress().getAddress() != null ? exchange.getRemoteAddress().getAddress().getHostAddress() : "127.0.0.1");
                        failedState.setStartedAt(java.time.Instant.now().toString());
                        failedState.setFinishedAt(java.time.Instant.now().toString());
                        if (engine.getStateStore() != null) {
                            engine.getStateStore().saveState(failedState);
                        }
                    }
                    sendJson(exchange, 403, Map.of("detail", "INITIATION_ACCESS_DENIED: User not authorized for workflow '" + wfName + "'"));
                    return;
                }
                LiveRequestContext ctx = buildRequestContext(exchange, uri.getPath(), method, new HashMap<>());
                Map<String, Object> payload = new HashMap<>();
                try {
                    payload = readJsonBody(exchange, ctx);
                    ctx.setBody(payload);
                } catch (OctopusMiddlewareException exc) {
                    handleMiddlewareException(exchange, bp, ctx, exc);
                    return;
                }

                if (!runMiddlewares(exchange, bp, ctx)) {
                    return;
                }

                if (ctx.getWorkflowInjections() != null) {
                    payload.putAll(ctx.getWorkflowInjections());
                }

                String jobId = extractOrCreateJobId(exchange, payload, ctx);
                payload.put("job_id", jobId);

                ExecutionState state = engine.initiateAsync(wfName, payload);
                state.setClientHost(ctx.getClientIp());
                state.setRoute(uri.getPath());
                state.setMethod(method);
                if (engine.getStateStore() != null) {
                    engine.getStateStore().saveState(state);
                }

                exchange.getResponseHeaders().set("X-Job-ID", state.getJobId());
                sendJson(exchange, 200, Map.of(
                    "status", "INITIATED",
                    "job_id", state.getJobId(),
                    "workflow_name", state.getWorkflowName()
                ));
                return;
            }

            // Per-workflow Jobs GET
            if ("/jobs".equals(subPath) && "GET".equalsIgnoreCase(method)) {
                if (!checkRole(exchange, bp)) {
                    sendJson(exchange, 403, Map.of("detail", "INITIATION_ACCESS_DENIED: User not authorized for workflow '" + wfName + "'"));
                    return;
                }
                Map<String, String> queryParams = parseQueryParams(uri.getQuery());
                String limitStr = queryParams.get("limit");
                int limit = 100;
                if (limitStr != null && !limitStr.isBlank()) {
                    try { limit = Integer.parseInt(limitStr); } catch (Exception ignored) {}
                }

                List<ExecutionState> states;
                if (engine.getStateStore() != null) {
                    states = engine.getStateStore().listStates(wfName, limit);
                } else {
                    states = engine.getWorkflowUseCase().listExecutionStates(wfName, limit);
                }

                List<Map<String, Object>> result = new ArrayList<>();
                for (ExecutionState s : states) {
                    Map<String, Object> item = new HashMap<>();
                    item.put("job_id", s.getJobId());
                    item.put("workflow_name", s.getWorkflowName());
                    item.put("workflow", s.getWorkflowName());
                    item.put("route", s.getRoute() != null ? s.getRoute() : "");
                    item.put("route_id", s.getRoute() != null ? s.getRoute() : "");
                    item.put("method", s.getMethod() != null ? s.getMethod() : "POST");
                    item.put("route_method", s.getMethod() != null ? s.getMethod() : "POST");
                    item.put("status", s.getStatus() != null ? s.getStatus().name() : "IDLE");
                    int sc = s.getErrorCode() != null ? s.getErrorCode() : (s.getStatus() == ExecutionStatus.COMPLETED ? 200 : (s.getStatus() == ExecutionStatus.FAILED ? 500 : 0));
                    item.put("status_code", sc);
                    item.put("source_ip", s.getClientHost() != null ? s.getClientHost() : "127.0.0.1");
                    item.put("client_host", s.getClientHost() != null ? s.getClientHost() : "127.0.0.1");
                    double dur = s.getDurationMs() != null ? Math.round(s.getDurationMs() * 100.0) / 100.0 : 0.0;
                    item.put("duration_ms", dur);
                    item.put("execution_time_ms", dur);
                    item.put("started_at", s.getStartedAt());
                    item.put("finished_at", s.getFinishedAt());
                    item.put("error", s.getError());
                    item.put("error_type", s.getErrorType());
                    result.add(item);
                }
                sendJson(exchange, 200, result);
                return;
            }

            // Diagram X6 GET
            if ("/diagram/x6".equals(subPath) && "GET".equalsIgnoreCase(method)) {
                Map<String, String> qp = parseQueryParams(uri.getQuery());
                String jobId = qp.get("job_id");
                String route = qp.get("route");
                String methodQ = qp.get("method");
                Map<String, Object> diagram = engine.getVisualizerUseCase().compileX6Diagram(wfName, jobId, route, methodQ);
                sendJson(exchange, 200, diagram);
                return;
            }

            // Status GET: /status/{job_id}
            if (subPath.startsWith("/status/") && "GET".equalsIgnoreCase(method)) {
                String jobId = subPath.substring("/status/".length());
                ExecutionState state = engine.getWorkflowUseCase().getExecutionState(jobId);
                if (state == null) {
                    sendJson(exchange, 404, Map.of("error", "Job '" + jobId + "' not found"));
                    return;
                }
                Map<String, Object> statusResp = new HashMap<>();
                statusResp.put("job_id", state.getJobId());
                statusResp.put("workflow_name", state.getWorkflowName());
                statusResp.put("status", state.getStatus().name());
                statusResp.put("lifecycle_state", state.getStatus().name());
                statusResp.put("route", state.getRoute());
                statusResp.put("method", state.getMethod());
                statusResp.put("current_step_id", state.getCurrentStepId());
                statusResp.put("current_index", state.getCurrentIndex());
                statusResp.put("total_steps", state.getTotalSteps());
                statusResp.put("started_at", state.getStartedAt());
                statusResp.put("finished_at", state.getFinishedAt());
                sendJson(exchange, 200, statusResp);
                return;
            }

            // Timeline GET: /timeline/{job_id}
            if (subPath.startsWith("/timeline/") && "GET".equalsIgnoreCase(method)) {
                String jobId = subPath.substring("/timeline/".length());
                ExecutionState state = engine.getWorkflowUseCase().getExecutionState(jobId);
                if (state == null) {
                    sendJson(exchange, 404, Map.of("error", "Job '" + jobId + "' not found"));
                    return;
                }
                Map<String, Object> timeResp = new HashMap<>();
                timeResp.put("job_id", state.getJobId());
                timeResp.put("workflow_name", state.getWorkflowName());
                timeResp.put("status", state.getStatus().name());
                timeResp.put("route", state.getRoute());
                timeResp.put("method", state.getMethod());
                timeResp.put("data", state.getData());
                timeResp.put("steps", state.getTraces());
                sendJson(exchange, 200, timeResp);
                return;
            }

            sendJson(exchange, 404, Map.of("error", "Endpoint not found: " + subPath));
        }

        private void handleExecutionException(
            HttpExchange exchange,
            WorkflowBlueprint bp,
            String path,
            String method,
            LiveRequestContext ctx,
            String jobId,
            long startNano,
            Exception ex
        ) throws IOException {
            double durationMs = (System.nanoTime() - startNano) / 1_000_000.0;
            exchange.getResponseHeaders().set("X-Job-ID", jobId);
            exchange.getResponseHeaders().set("X-Execution-Status", "FAILED");
            exchange.getResponseHeaders().set("X-Execution-Duration-Ms", String.format(Locale.US, "%.2f", durationMs));

            Throwable cause = ex;
            while (cause != null && !(cause instanceof OctopusMiddlewareException) && cause.getCause() != null && cause.getCause() != cause) {
                cause = cause.getCause();
            }

            int sc = 500;
            String detail = ex.getMessage() != null ? ex.getMessage() : ex.toString();
            String errName = ex.getClass().getSimpleName();
            Map<String, Object> errDetails = null;

            ExecutionState failedState = null;
            if (jobId != null && !jobId.isBlank() && engine.getStateStore() != null) {
                failedState = engine.getStateStore().getState(jobId);
            }

            if (failedState != null) {
                if (failedState.getErrorCode() != null && failedState.getErrorCode() > 0) {
                    sc = failedState.getErrorCode();
                }
                if (failedState.getError() != null && !failedState.getError().isBlank()) {
                    detail = failedState.getError();
                }
                if (failedState.getErrorType() != null && !failedState.getErrorType().isBlank()) {
                    errName = failedState.getErrorType();
                }
            } else if (cause instanceof OctopusMiddlewareException ome) {
                sc = ome.getStatusCode();
                detail = ome.getDetail();
                errName = ome.getErrorName();
                errDetails = ome.getDetails();

                if (bp != null && bp.getMiddlewares() != null) {
                    for (var mw : bp.getMiddlewares()) {
                        if (mw.getExceptions() != null) {
                            for (var ec : mw.getExceptions()) {
                                if (ec.getException() != null && ec.getException().equalsIgnoreCase(errName)) {
                                    if (ec.getCode() > 0) sc = ec.getCode();
                                    if (ec.getMessage() != null) detail = ec.getMessage();
                                    if (ec.getActivity() != null && engine.getActivityExecutor() != null) {
                                        Map<String, Object> actPayload = new HashMap<>();
                                        actPayload.put("exception", errName);
                                        actPayload.put("error", errName);
                                        actPayload.put("message", detail);
                                        actPayload.put("status_code", sc);
                                        actPayload.put("path", path);
                                        actPayload.put("method", method);
                                        actPayload.put("client_ip", ctx != null ? ctx.getClientIp() : "127.0.0.1");
                                        if (ec.isAsync()) {
                                            if (engine.getAsyncDispatcher() != null) {
                                                engine.getAsyncDispatcher().dispatch(
                                                    ec.getActivity(),
                                                    actPayload,
                                                    "req_middleware_" + System.currentTimeMillis(),
                                                    "middleware_exception_" + sc,
                                                    engine.getActivityExecutor()
                                                );
                                            } else {
                                                java.util.concurrent.CompletableFuture.runAsync(() -> {
                                                    try { engine.getActivityExecutor().executeActivity(ec.getActivity(), actPayload); } catch (Exception ignored) {}
                                                });
                                            }
                                        } else {
                                            try { engine.getActivityExecutor().executeActivity(ec.getActivity(), actPayload); } catch (Exception ignored) {}
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if (jobId != null && !jobId.isBlank()) {
                if (failedState == null) {
                    failedState = new ExecutionState();
                    failedState.setJobId(jobId);
                    failedState.setWorkflowName(bp != null ? bp.getWorkflowName() : "default");
                    failedState.setStartedAt(java.time.Instant.now().toString());
                }
                failedState.setStatus(ExecutionStatus.FAILED);
                failedState.setFinishedAt(java.time.Instant.now().toString());
                failedState.setRoute(path);
                failedState.setMethod(method);
                failedState.setClientHost(ctx != null ? ctx.getClientIp() : "127.0.0.1");
                failedState.setDurationMs(durationMs);
                failedState.setError(detail);
                failedState.setErrorType(errName);
                failedState.setErrorCode(sc);

                if (engine.getStateStore() != null) {
                    engine.getStateStore().saveState(failedState);
                }
            }

            Map<String, Object> errBody = new HashMap<>();
            errBody.put("error", errName);
            errBody.put("detail", detail);
            errBody.put("status_code", sc);
            if (errDetails != null && !errDetails.isEmpty()) {
                errBody.put("details", errDetails);
            }
            sendJson(exchange, sc, errBody);
        }

        private String extractOrCreateJobId(HttpExchange exchange, Map<String, Object> payload, LiveRequestContext ctx) {
            String jobId = exchange.getRequestHeaders().getFirst("X-Job-ID");
            if (jobId == null || jobId.isBlank()) {
                jobId = exchange.getRequestHeaders().getFirst("X-Request-ID");
            }
            if (jobId == null || jobId.isBlank()) {
                jobId = exchange.getRequestHeaders().getFirst("X-Correlation-ID");
            }
            if ((jobId == null || jobId.isBlank()) && ctx != null && ctx.getCorrelationId() != null) {
                jobId = ctx.getCorrelationId();
            }
            if ((jobId == null || jobId.isBlank()) && payload != null && payload.get("job_id") != null) {
                jobId = payload.get("job_id").toString();
            }
            if (jobId == null || jobId.isBlank()) {
                jobId = "job_" + UUID.randomUUID().toString().replace("-", "");
            }
            return jobId;
        }

        private Map<String, String> matchRoute(String routePattern, String actualPath) {
            if (routePattern == null || actualPath == null) return null;
            if (routePattern.equals(actualPath)) return new HashMap<>();

            String regex = routePattern.replaceAll("\\{([^}]+)\\}", "(?<$1>[^/]+)");
            try {
                Pattern pattern = Pattern.compile("^" + regex + "$");
                Matcher matcher = pattern.matcher(actualPath);
                if (matcher.matches()) {
                    Map<String, String> params = new HashMap<>();
                    Matcher nameMatcher = Pattern.compile("\\{([^}]+)\\}").matcher(routePattern);
                    while (nameMatcher.find()) {
                        String paramName = nameMatcher.group(1);
                        params.put(paramName, matcher.group(paramName));
                    }
                    return params;
                }
            } catch (Exception ignored) {}

            return null;
        }

        private boolean checkRole(HttpExchange exchange, WorkflowBlueprint bp) {
            List<String> allowedRoles = bp.getAllowedInitiatorRoles();
            if (allowedRoles == null || allowedRoles.isEmpty()) {
                return true;
            }

            String roleHeader = exchange.getRequestHeaders().getFirst("X-User-Roles");
            if (roleHeader == null || roleHeader.isBlank()) {
                roleHeader = exchange.getRequestHeaders().getFirst("X-Initiator-Role");
            }

            if (roleHeader == null || roleHeader.isBlank()) {
                return false;
            }

            for (String r : allowedRoles) {
                if (roleHeader.toLowerCase().contains(r.toLowerCase())) {
                    return true;
                }
            }

            return roleHeader.toLowerCase().contains("domain_admin")
                || roleHeader.toLowerCase().contains("system")
                || roleHeader.toLowerCase().contains("admin");
        }

        @SuppressWarnings("unchecked")
        private Map<String, Object> readJsonBody(HttpExchange exchange, LiveRequestContext ctx) {
            try (InputStream is = exchange.getRequestBody()) {
                byte[] bytes = is.readAllBytes();
                if (bytes == null || bytes.length == 0) {
                    if (ctx != null) ctx.setRawBodyBytes(new byte[0]);
                    return new HashMap<>();
                }
                if (ctx != null) ctx.setRawBodyBytes(bytes);
                String trimmed = new String(bytes, StandardCharsets.UTF_8).trim();
                if (trimmed.isEmpty()) {
                    return new HashMap<>();
                }
                if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
                    try {
                        Map<String, Object> res = mapper.readValue(bytes, Map.class);
                        return res != null ? res : new HashMap<>();
                    } catch (Exception e) {
                        throw new MalformedJsonException("Malformed JSON syntax in request body: " + e.getMessage());
                    }
                }
                return new HashMap<>();
            } catch (OctopusMiddlewareException ome) {
                throw ome;
            } catch (Exception e) {
                return new HashMap<>();
            }
        }

        private Map<String, Object> readJsonBody(HttpExchange exchange) {
            return readJsonBody(exchange, null);
        }

        private void sendJson(HttpExchange exchange, int status, Object data) throws IOException {
            byte[] bytes = mapper.writeValueAsBytes(data);
            exchange.getResponseHeaders().set("Content-Type", "application/json; charset=utf-8");
            exchange.sendResponseHeaders(status, bytes.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(bytes);
            }
        }

        private void sendHtml(HttpExchange exchange, int status, String html) throws IOException {
            byte[] bytes = html.getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().set("Content-Type", "text/html; charset=utf-8");
            exchange.sendResponseHeaders(status, bytes.length);
            try (OutputStream os = exchange.getResponseBody()) {
                os.write(bytes);
            }
        }

        private Map<String, String> parseQueryParams(String query) {
            Map<String, String> map = new HashMap<>();
            if (query == null || query.isBlank()) return map;
            String[] pairs = query.split("&");
            for (String p : pairs) {
                String[] kv = p.split("=");
                if (kv.length == 2) {
                    map.put(kv[0], kv[1]);
                } else if (kv.length == 1) {
                    map.put(kv[0], "");
                }
            }
            return map;
        }

        private Map<String, Object> generateOpenAPISchemaFromJavaClass(
                Class<?> clazz,
                Map<String, Object> registeredSchemas,
                Map<String, Class<?>> modelRegistry
        ) {
            if (clazz == null || clazz == Object.class || clazz == void.class || clazz == Void.class) {
                return Map.of("type", "object");
            }

            if (clazz == String.class || clazz == CharSequence.class) {
                return Map.of("type", "string");
            }
            if (clazz == int.class || clazz == Integer.class || clazz == long.class || clazz == Long.class || clazz == short.class || clazz == Short.class || clazz == byte.class || clazz == Byte.class) {
                return Map.of("type", "integer");
            }
            if (clazz == double.class || clazz == Double.class || clazz == float.class || clazz == Float.class) {
                return Map.of("type", "number", "format", "double");
            }
            if (clazz == boolean.class || clazz == Boolean.class) {
                return Map.of("type", "boolean");
            }
            if (java.util.Collection.class.isAssignableFrom(clazz) || clazz.isArray()) {
                return Map.of("type", "array", "items", Map.of("type", "string"));
            }
            if (java.util.Map.class.isAssignableFrom(clazz)) {
                return Map.of("type", "object");
            }

            Map<String, Object> properties = new LinkedHashMap<>();
            List<String> required = new ArrayList<>();

            java.lang.reflect.Field[] fields = clazz.getDeclaredFields();
            for (java.lang.reflect.Field field : fields) {
                if (java.lang.reflect.Modifier.isStatic(field.getModifiers()) || java.lang.reflect.Modifier.isTransient(field.getModifiers())) {
                    continue;
                }
                String fieldName = field.getName();
                com.fasterxml.jackson.annotation.JsonProperty jsonProp = field.getAnnotation(com.fasterxml.jackson.annotation.JsonProperty.class);
                if (jsonProp != null && !jsonProp.value().isEmpty()) {
                    fieldName = jsonProp.value();
                }

                Class<?> fieldType = field.getType();
                Map<String, Object> fieldSchema;

                if (fieldType == String.class) {
                    fieldSchema = new LinkedHashMap<>(Map.of("type", "string"));
                } else if (fieldType == int.class || fieldType == Integer.class || fieldType == long.class || fieldType == Long.class) {
                    fieldSchema = new LinkedHashMap<>(Map.of("type", "integer"));
                } else if (fieldType == double.class || fieldType == Double.class || fieldType == float.class || fieldType == Float.class) {
                    fieldSchema = new LinkedHashMap<>(Map.of("type", "number", "format", "double"));
                } else if (fieldType == boolean.class || fieldType == Boolean.class) {
                    fieldSchema = new LinkedHashMap<>(Map.of("type", "boolean"));
                } else if (java.util.Collection.class.isAssignableFrom(fieldType) || fieldType.isArray()) {
                    Class<?> itemType = String.class;
                    java.lang.reflect.Type genericType = field.getGenericType();
                    if (genericType instanceof java.lang.reflect.ParameterizedType pt) {
                        java.lang.reflect.Type[] args = pt.getActualTypeArguments();
                        if (args.length > 0 && args[0] instanceof Class<?> itemCls) {
                            itemType = itemCls;
                        }
                    } else if (fieldType.isArray()) {
                        itemType = fieldType.getComponentType();
                    }

                    if (itemType != String.class && itemType != Integer.class && itemType != Long.class && itemType != Double.class && itemType != Float.class && itemType != Boolean.class && itemType != Object.class) {
                        String itemTypeName = itemType.getSimpleName();
                        if (!registeredSchemas.containsKey(itemTypeName)) {
                            registeredSchemas.put(itemTypeName, new HashMap<String, Object>());
                            registeredSchemas.put(itemTypeName, generateOpenAPISchemaFromJavaClass(itemType, registeredSchemas, modelRegistry));
                        }
                        fieldSchema = new LinkedHashMap<>(Map.of(
                            "type", "array",
                            "items", Map.of("$ref", "#/components/schemas/" + itemTypeName)
                        ));
                    } else {
                        String itemSimpleType = itemType == Integer.class || itemType == Long.class ? "integer" :
                                (itemType == Double.class || itemType == Float.class ? "number" :
                                (itemType == Boolean.class ? "boolean" : "string"));
                        fieldSchema = new LinkedHashMap<>(Map.of(
                            "type", "array",
                            "items", Map.of("type", itemSimpleType)
                        ));
                    }
                } else if (java.util.Map.class.isAssignableFrom(fieldType)) {
                    fieldSchema = new LinkedHashMap<>(Map.of("type", "object"));
                } else {
                    String complexTypeName = fieldType.getSimpleName();
                    if (!registeredSchemas.containsKey(complexTypeName)) {
                        registeredSchemas.put(complexTypeName, new HashMap<String, Object>());
                        registeredSchemas.put(complexTypeName, generateOpenAPISchemaFromJavaClass(fieldType, registeredSchemas, modelRegistry));
                    }
                    fieldSchema = new LinkedHashMap<>(Map.of("$ref", "#/components/schemas/" + complexTypeName));
                }

                properties.put(fieldName, fieldSchema);
            }

            Map<String, Object> schema = new LinkedHashMap<>();
            schema.put("type", "object");
            schema.put("properties", properties);
            if (!required.isEmpty()) {
                schema.put("required", required);
            }
            return schema;
        }
    }
}
