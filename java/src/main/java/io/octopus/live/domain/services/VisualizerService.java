package io.octopus.live.domain.services;

import io.octopus.live.domain.models.*;
import io.octopus.live.domain.ports.*;
import java.util.*;

public class VisualizerService implements IVisualizerUseCase {

    private final IBlueprintRepository blueprintRepo;
    private final IStateStore stateStore;

    private static class PrevItem {
        final String id;
        final String label;
        final String color;
        final boolean executed;

        PrevItem(String id, String label, String color, boolean executed) {
            this.id = id;
            this.label = label != null ? label : "";
            this.color = color != null ? color : "#64748b";
            this.executed = executed;
        }

        PrevItem(String id, String label, String color) {
            this(id, label, color, false);
        }
    }

    private static class ExcConfigItem {
        final String activity;
        final String exception;
        final int code;
        final boolean isAsync;
        final boolean isMiddleware;
        final String sourceStepId;
        final String sourceName;
        final String message;
        final boolean isGlobal;

        ExcConfigItem(String activity, String exception, int code, boolean isAsync, boolean isMiddleware, String sourceStepId, String sourceName, String message, boolean isGlobal) {
            this.activity = activity;
            this.exception = exception != null ? exception : "Exception";
            this.code = code;
            this.isAsync = isAsync;
            this.isMiddleware = isMiddleware;
            this.sourceStepId = sourceStepId;
            this.sourceName = sourceName;
            this.message = message;
            this.isGlobal = isGlobal;
        }

        ExcConfigItem(String activity, String exception, int code, boolean isAsync, boolean isMiddleware, String sourceStepId, String sourceName, String message) {
            this(activity, exception, code, isAsync, isMiddleware, sourceStepId, sourceName, message, false);
        }

        ExcConfigItem(String activity, String exception, int code, boolean isAsync, boolean isMiddleware, String sourceStepId) {
            this(activity, exception, code, isAsync, isMiddleware, sourceStepId, null, null, false);
        }
    }

    public VisualizerService(IBlueprintRepository blueprintRepo, IStateStore stateStore) {
        this.blueprintRepo = blueprintRepo;
        this.stateStore = stateStore;
    }

    @Override
    public List<WorkflowBlueprint> listBlueprints() {
        return blueprintRepo.listBlueprints();
    }

    @Override
    public Map<String, Object> compileX6Diagram(String workflowName, String jobId) {
        return compileX6Diagram(workflowName, jobId, null, null);
    }

    @Override
    public Map<String, Object> compileX6Diagram(String workflowName, String jobId, String route, String method) {
        WorkflowBlueprint bp = blueprintRepo.getBlueprint(workflowName);
        if (bp == null) {
            throw new IllegalArgumentException("Blueprint '" + workflowName + "' not found.");
        }

        ExecutionState state = null;
        if (jobId != null && !jobId.isBlank()) {
            state = stateStore.getState(jobId);
            if (state == null) {
                // Fallback for Temporal Workflow IDs: e.g. "wf-customer_auth_portal-12345" -> "12345"
                if (jobId.startsWith("wf-" + workflowName + "-")) {
                    String cleanId = jobId.substring(("wf-" + workflowName + "-").length());
                    state = stateStore.getState(cleanId);
                } else if (jobId.startsWith("wf-")) {
                    int nextDash = jobId.indexOf('-', 3);
                    if (nextDash != -1 && nextDash + 1 < jobId.length()) {
                        state = stateStore.getState(jobId.substring(nextDash + 1));
                    }
                }
            }
        }

        Map<String, String> statusMap = new HashMap<>();
        Map<String, Double> durationMap = new HashMap<>();
        Map<String, Object> inputMap = new HashMap<>();
        Map<String, Object> outputMap = new HashMap<>();
        Map<String, String> errorMap = new HashMap<>();

        if (state != null && state.getTraces() != null) {
            for (StepExecutionTrace trace : state.getTraces()) {
                if (trace.getStepId() != null) {
                    statusMap.put(trace.getStepId(), trace.getStatus());
                    durationMap.put(trace.getStepId(), trace.getDurationMs());
                    if (trace.getInputPayload() != null) inputMap.put(trace.getStepId(), trace.getInputPayload());
                    if (trace.getOutputPayload() != null) outputMap.put(trace.getStepId(), trace.getOutputPayload());
                    if (trace.getErrorMessage() != null) errorMap.put(trace.getStepId(), trace.getErrorMessage());
                }
                if (trace.getActivityName() != null && !trace.getActivityName().isBlank()) {
                    statusMap.put(trace.getActivityName(), trace.getStatus());
                    if (trace.getInputPayload() != null) inputMap.put(trace.getActivityName(), trace.getInputPayload());
                    if (trace.getOutputPayload() != null) outputMap.put(trace.getActivityName(), trace.getOutputPayload());
                }
            }
        }

        List<Map<String, Object>> routesList = new ArrayList<>();
        if (bp.getApi() != null && !bp.getApi().isEmpty()) {
            for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                Map<String, Object> rMap = new HashMap<>();
                rMap.put("route", ep.getRoute());
                rMap.put("method", ep.getMethod() != null ? ep.getMethod().toUpperCase() : "POST");
                rMap.put("summary", ep.getSummary() != null ? ep.getSummary() : "");
                rMap.put("description", ep.getDescription() != null ? ep.getDescription() : "");
                rMap.put("step_count", ep.getSteps() != null ? ep.getSteps().size() : 0);
                routesList.add(rMap);
            }
        }

        WorkflowBlueprint.ApiEndpointConfig selectedEp = null;
        String selectedRoute = null;
        String selectedMethod = null;

        // 1. If we have a live job execution state, match by state.getRoute() / getMethod() or trace step IDs
        if (state != null) {
            if (state.getRoute() != null && !state.getRoute().isBlank() && bp.getApi() != null) {
                for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                    if (ep.getRoute() != null && (ep.getRoute().equalsIgnoreCase(state.getRoute())
                            || state.getRoute().endsWith(ep.getRoute())
                            || ep.getRoute().endsWith(state.getRoute()))) {
                        selectedEp = ep;
                        break;
                    }
                }
            }

            if (selectedEp == null && bp.getApi() != null && state.getTraces() != null && !state.getTraces().isEmpty()) {
                Set<String> traceStepIds = new HashSet<>();
                for (StepExecutionTrace t : state.getTraces()) {
                    if (t.getStepId() != null) traceStepIds.add(t.getStepId().toLowerCase());
                    if (t.getActivityName() != null) traceStepIds.add(t.getActivityName().toLowerCase());
                }
                for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                    if (ep.getSteps() != null) {
                        boolean match = false;
                        for (WorkflowStep s : ep.getSteps()) {
                            if ((s.getId() != null && traceStepIds.contains(s.getId().toLowerCase()))
                                    || (s.getActivity() != null && traceStepIds.contains(s.getActivity().toLowerCase()))) {
                                match = true;
                                break;
                            }
                        }
                        if (match) {
                            selectedEp = ep;
                            break;
                        }
                    }
                }
            }
        }

        // 2. If no job match, check explicit requested route / method from query parameter
        if (selectedEp == null && route != null && !route.isBlank() && bp.getApi() != null) {
            for (WorkflowBlueprint.ApiEndpointConfig ep : bp.getApi()) {
                boolean matchRoute = ep.getRoute() != null && (ep.getRoute().equalsIgnoreCase(route)
                        || route.endsWith(ep.getRoute())
                        || ep.getRoute().endsWith(route));
                boolean matchMethod = method == null || method.isBlank() || (ep.getMethod() != null && ep.getMethod().equalsIgnoreCase(method));
                if (matchRoute && matchMethod) {
                    selectedEp = ep;
                    break;
                }
            }
        }

        // 3. Resolve the step list to render
        List<WorkflowStep> stepsToVisualize;
        if (selectedEp != null && selectedEp.getSteps() != null && !selectedEp.getSteps().isEmpty()) {
            stepsToVisualize = selectedEp.getSteps();
            selectedRoute = selectedEp.getRoute();
            selectedMethod = selectedEp.getMethod() != null ? selectedEp.getMethod().toUpperCase() : "POST";
        } else if (bp.getSteps() != null && !bp.getSteps().isEmpty()) {
            stepsToVisualize = bp.getSteps();
            selectedRoute = "default";
            selectedMethod = "EXECUTE";
        } else if (bp.getApi() != null && !bp.getApi().isEmpty() && bp.getApi().get(0).getSteps() != null) {
            selectedEp = bp.getApi().get(0);
            stepsToVisualize = selectedEp.getSteps();
            selectedRoute = selectedEp.getRoute();
            selectedMethod = selectedEp.getMethod() != null ? selectedEp.getMethod().toUpperCase() : "POST";
        } else {
            stepsToVisualize = Collections.emptyList();
        }

        boolean hasLiveExecution = state != null;
        String startStatus = hasLiveExecution ? "COMPLETED" : "PENDING";

        List<Map<String, Object>> nodes = new ArrayList<>();
        List<Map<String, Object>> edges = new ArrayList<>();

        // 1. Start Node
        Map<String, Object> startNode = new HashMap<>();
        startNode.put("id", "node__start");
        startNode.put("shape", "octopus-start-node");
        startNode.put("label", "START");
        startNode.put("width", 120);
        startNode.put("height", 50);
        Map<String, Object> startData = new HashMap<>();
        startData.put("step_id", "start");
        startData.put("type", "start");
        startData.put("title", "Workflow Start");
        startData.put("status", startStatus);
        startData.put("executed", hasLiveExecution);
        startNode.put("data", startData);
        if (hasLiveExecution) {
            startNode.put("attrs", Map.of(
                "body", Map.of(
                    "rx", 25,
                    "ry", 25,
                    "fill", "#064e3b",
                    "stroke", "#10b981",
                    "strokeWidth", 3,
                    "filter", "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    "opacity", 1.0
                ),
                "label", Map.of(
                    "fill", "#6ee7b7",
                    "fontSize", 13,
                    "fontWeight", 700
                )
            ));
        }
        nodes.add(startNode);

        // 2. Ingress Middleware Guards Node (Filtered by route excludes / includes)
        List<MiddlewareConfig> allMiddlewares = bp.getMiddlewares();
        List<MiddlewareConfig> preMiddlewares = new ArrayList<>();
        List<MiddlewareConfig> postMiddlewares = new ArrayList<>();
        if (allMiddlewares != null) {
            for (MiddlewareConfig m : allMiddlewares) {
                if (isMiddlewareActiveForRoute(m, selectedRoute)) {
                    if ("post-request".equalsIgnoreCase(m.getType())) {
                        postMiddlewares.add(m);
                    } else {
                        preMiddlewares.add(m);
                    }
                }
            }
        }
        boolean hasIngressGuards = !preMiddlewares.isEmpty();
        boolean hasEgressGuards = !postMiddlewares.isEmpty();
        List<PrevItem> initialPrev;

        boolean isIngressFailed = false;
        if (hasIngressGuards) {
            List<String> mwShortNames = new ArrayList<>();
            Set<String> mwExcNames = new HashSet<>();
            for (MiddlewareConfig m : preMiddlewares) {
                if (m.getName() != null) {
                    String sName = m.getName().replace("Middleware", "");
                    if (!sName.isBlank() && !mwShortNames.contains(sName)) {
                        mwShortNames.add(sName);
                    }
                }
                if (m.getExceptions() != null) {
                    for (MiddlewareExceptionConfig e : m.getExceptions()) {
                        if (e.getException() != null) {
                            mwExcNames.add(e.getException().toLowerCase());
                        }
                    }
                }
            }

            String ingressLabel = "🛡️ Ingress Middleware Guards";

            if (state != null && state.getStatus() == ExecutionStatus.FAILED) {
                String stateErr = state.getError() != null ? state.getError().toLowerCase() : "";
                for (String me : mwExcNames) {
                    if (stateErr.contains(me)) {
                        isIngressFailed = true;
                        break;
                    }
                }
                if (!isIngressFailed && state.getTraces() != null) {
                    for (StepExecutionTrace t : state.getTraces()) {
                        String tErr = t.getErrorMessage() != null ? t.getErrorMessage().toLowerCase() : "";
                        for (String me : mwExcNames) {
                            if (tErr.contains(me)) {
                                isIngressFailed = true;
                                break;
                            }
                        }
                        if (isIngressFailed) break;
                    }
                }
            }

            String ingressStatus = isIngressFailed ? "FAILED" : (hasLiveExecution ? "COMPLETED" : "PENDING");

            Map<String, Object> ingressNode = new HashMap<>();
            ingressNode.put("id", "node__ingress_guards");
            ingressNode.put("shape", "octopus-activity-node");
            ingressNode.put("label", ingressLabel);
            ingressNode.put("width", 220);
            ingressNode.put("height", 55);

            Map<String, Object> ingressData = new HashMap<>();
            ingressData.put("step_id", "ingress_guards");
            ingressData.put("type", "ingress_guards");
            ingressData.put("title", "🛡️ Ingress Middleware Guards");
            ingressData.put("description", "Pre-execution security filters, JWT authentication, IP whitelisting, and rate-limiting guards executed prior to workflow pipeline dispatch.");
            ingressData.put("status", ingressStatus);
            ingressData.put("executed", hasLiveExecution);
            List<String> mwNamesList = new ArrayList<>();
            for (MiddlewareConfig m : preMiddlewares) {
                if (m.getName() != null) mwNamesList.add(m.getName());
            }
            ingressData.put("middlewares", mwNamesList);
            ingressNode.put("data", ingressData);

            if (isIngressFailed) {
                ingressNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#ef4444",
                        "strokeWidth", 3,
                        "fill", "#2d1215",
                        "filter", "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#fca5a5",
                        "fontWeight", 700
                    )
                ));
            } else if (hasLiveExecution) {
                ingressNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#10b981",
                        "strokeWidth", 3,
                        "fill", "#0f2e22",
                        "filter", "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#6ee7b7",
                        "fontWeight", 600
                    )
                ));
            }
            nodes.add(ingressNode);

            // Edge from start to ingress guards
            String startToIngressStroke = isIngressFailed ? "#ef4444" : (hasLiveExecution ? "#10b981" : "#6366f1");
            Map<String, Object> startToIngressEdge = new HashMap<>();
            startToIngressEdge.put("id", "edge__node__start__to__node__ingress_guards");
            startToIngressEdge.put("source", "node__start");
            startToIngressEdge.put("target", "node__ingress_guards");
            startToIngressEdge.put("attrs", Map.of(
                "line", Map.of(
                    "stroke", startToIngressStroke,
                    "strokeWidth", 2,
                    "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", startToIngressStroke)
                )
            ));
            edges.add(startToIngressEdge);

            initialPrev = List.of(new PrevItem("node__ingress_guards", "✅ Quota & Security OK", "#10b981", hasLiveExecution && !isIngressFailed));
        } else {
            initialPrev = List.of(new PrevItem("node__start", "", "#6366f1", hasLiveExecution));
        }

        List<PrevItem> finalHeads = processStepList(stepsToVisualize, initialPrev, "", true, statusMap, durationMap, inputMap, outputMap, errorMap, state, hasLiveExecution, nodes, edges);

        // 3. Exception Handlers & Failure Terminal Nodes Collection
        List<ExcConfigItem> excConfigs = new ArrayList<>();

        // Global Exception Handler (Declarative Safety Net)
        if (bp != null && bp.getErrorHandling() != null && bp.getErrorHandling().getGlobalHandler() != null) {
            var gh = bp.getErrorHandling().getGlobalHandler();
            if (gh.getActivity() != null && !gh.getActivity().isBlank()) {
                int ghCode = gh.getCode() > 0 ? gh.getCode() : 500;
                excConfigs.add(new ExcConfigItem(gh.getActivity(), "*", ghCode, gh.isAsync(), false, null, "Global Incident Safety Net", gh.getMessage(), true));
            }
        } else if (bp != null && bp.getGlobalExceptionHandler() != null) {
            var gh = bp.getGlobalExceptionHandler();
            if (gh.getActivity() != null && !gh.getActivity().isBlank()) {
                int ghCode = gh.getCode() > 0 ? gh.getCode() : 500;
                excConfigs.add(new ExcConfigItem(gh.getActivity(), "*", ghCode, gh.isAsync(), false, null, "Global Incident Safety Net", gh.getMessage(), true));
            }
        }

        if (preMiddlewares != null) {
            for (MiddlewareConfig mw : preMiddlewares) {
                if (mw.getExceptions() != null) {
                    for (MiddlewareExceptionConfig exc : mw.getExceptions()) {
                        if (exc.getActivity() != null && !exc.getActivity().isBlank()) {
                            excConfigs.add(new ExcConfigItem(exc.getActivity(), exc.getException(), exc.getCode(), exc.isAsync(), true, null, mw.getName(), exc.getMessage(), false));
                        }
                    }
                }
            }
        }

        // Collect step-level exceptions recursively only for the route being visualized
        collectStepExceptions(stepsToVisualize, excConfigs);

        // Check if any trace activity was an exception recovery activity
        if (state != null && state.getTraces() != null) {
            for (StepExecutionTrace trace : state.getTraces()) {
                String sId = trace.getStepId() != null ? trace.getStepId() : "";
                String aName = trace.getActivityName() != null ? trace.getActivityName() : "";
                if ((sId.startsWith("exc_") || aName.toLowerCase().contains("audit") || aName.toLowerCase().contains("handle")) && excConfigs.stream().noneMatch(e -> e.activity.equalsIgnoreCase(aName))) {
                    boolean isGlob = aName.toLowerCase().contains("global") || aName.toLowerCase().contains("unhandled");
                    excConfigs.add(new ExcConfigItem(aName, trace.getErrorMessage() != null ? trace.getErrorMessage() : (isGlob ? "*" : "Exception"), isGlob ? 500 : 400, true, false, null, null, null, isGlob));
                }
            }
        }

        Set<String> processedExcIds = new HashSet<>();
        for (ExcConfigItem exc : excConfigs) {
            String excName = exc.exception != null ? exc.exception : "MiddlewareException";
            int excCode = exc.code > 0 ? exc.code : 400;
            String excActName = exc.activity;
            boolean isGlobalExc = exc.isGlobal || "*".equals(excName) || excCode == 500 || (excActName != null && excActName.toLowerCase().contains("global"));
            String excNodeId = isGlobalExc ? ("node__exc_" + (excActName != null ? excActName.toLowerCase() : "global")) : ("node__exc_" + (exc.sourceStepId != null ? exc.sourceStepId.toLowerCase() + "_" : "") + excName.toLowerCase());

            // Connect Source -> Exception Handler Node
            String srcId;
            if (exc.sourceStepId != null && !exc.sourceStepId.isBlank()) {
                srcId = "node__" + exc.sourceStepId;
            } else if (isGlobalExc) {
                srcId = hasIngressGuards ? "node__ingress_guards" : "node__start";
                if (state != null && state.getTraces() != null) {
                    for (StepExecutionTrace t : state.getTraces()) {
                        if ("FAILED".equalsIgnoreCase(t.getStatus()) && !(t.getStepId() != null && t.getStepId().startsWith("exc_"))) {
                            srcId = "node__" + t.getStepId();
                            break;
                        }
                    }
                }
                if ("node__ingress_guards".equals(srcId) && stepsToVisualize != null && !stepsToVisualize.isEmpty()) {
                    srcId = "node__" + stepsToVisualize.get(0).getId();
                }
            } else if (exc.isMiddleware) {
                srcId = hasIngressGuards ? "node__ingress_guards" : "node__start";
            } else {
                srcId = hasIngressGuards ? "node__ingress_guards" : "node__start";
            }

            // Ensure source node actually exists in the graph
            final String finalSrcId = srcId;
            boolean srcExists = nodes.stream().anyMatch(n -> finalSrcId.equals(n.get("id")));
            if (!srcExists) {
                continue;
            }

            if (processedExcIds.contains(excNodeId)) {
                continue;
            }
            processedExcIds.add(excNodeId);

            // Determine if this exception actually fired in this job execution
            boolean isExcExecuted = false;
            if (state != null) {
                String sErr = state.getError() != null ? state.getError().toLowerCase() : "";
                String excSrcStep = exc.sourceStepId != null ? exc.sourceStepId.toLowerCase() : "";

                if (isGlobalExc) {
                    if (state.getStatus() == ExecutionStatus.FAILED) {
                        if (state.getTraces() != null) {
                            for (StepExecutionTrace t : state.getTraces()) {
                                String aName = t.getActivityName() != null ? t.getActivityName() : "";
                                String sId = t.getStepId() != null ? t.getStepId() : "";
                                if (aName.equalsIgnoreCase(excActName) || sId.equalsIgnoreCase("exc_" + excActName)) {
                                    isExcExecuted = true;
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    if (state.getTraces() != null) {
                        for (StepExecutionTrace t : state.getTraces()) {
                            String tErr = t.getErrorMessage() != null ? t.getErrorMessage().toLowerCase() : "";
                            String tStep = t.getStepId() != null ? t.getStepId().toLowerCase() : "";
                            String aName = t.getActivityName() != null ? t.getActivityName() : "";

                            String inpExc = "";
                            String inpErr = "";
                            String inpStep = "";
                            int inpCode = 0;
                            Map<String, Object> inp = t.getInputPayload();
                            if (inp != null) {
                                if (inp.get("exception") != null) inpExc = inp.get("exception").toString().toLowerCase();
                                if (inp.get("error") != null) inpErr = inp.get("error").toString().toLowerCase();
                                if (inp.get("step_id") != null) inpStep = inp.get("step_id").toString().toLowerCase();
                                else if (inp.get("failing_step_id") != null) inpStep = inp.get("failing_step_id").toString().toLowerCase();
                                if (inp.get("status_code") instanceof Number num) inpCode = num.intValue();
                            }

                            if (!excSrcStep.isEmpty()) {
                                if (aName.equalsIgnoreCase(excActName) || tStep.equals("exc_" + excSrcStep + "_" + excName.toLowerCase()) || tStep.equals("exc_" + excActName.toLowerCase())) {
                                    if (inpStep.equals(excSrcStep) || inpExc.equals(excName.toLowerCase()) || inpErr.equals(excName.toLowerCase()) || inpExc.contains(excName.toLowerCase()) || (inpCode > 0 && inpCode == excCode)) {
                                        isExcExecuted = true;
                                        break;
                                    }
                                }
                            } else {
                                if (aName.equalsIgnoreCase(excActName) || tStep.equals("exc_" + excName.toLowerCase()) || tStep.equals("exc_" + excActName.toLowerCase())) {
                                    if (!inpStep.isEmpty() && !inpStep.equals("ingress_guards") && !inpStep.equals("egress_guards") && !inpStep.equals("middleware")) {
                                        continue;
                                    }
                                    if (inpExc.equals(excName.toLowerCase()) || inpErr.equals(excName.toLowerCase()) || tErr.contains(excName.toLowerCase()) || tStep.equals("exc_" + excName.toLowerCase())) {
                                        isExcExecuted = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if (!isExcExecuted) {
                        if (state.getData() != null) {
                            Object dExc = state.getData().get("exception");
                            Object dErr = state.getData().get("error");
                            if (dExc != null && dExc.toString().equalsIgnoreCase(excName)) {
                                isExcExecuted = true;
                            } else if (dErr != null && dErr.toString().equalsIgnoreCase(excName)) {
                                isExcExecuted = true;
                            }
                        }
                        if (!isExcExecuted && sErr.contains(excName.toLowerCase())) {
                            isExcExecuted = true;
                        }
                    }
                }
            }

            String excStatus = isExcExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

            String subLbl;
            String excEdgeLbl;
            String termLbl;
            String termId;

            String excLower = excName.toLowerCase();
            if (excLower.contains("rate")) {
                subLbl = "(Async Rate Limit Audit)";
                excEdgeLbl = "⚠️ 429 RateLimitExceeded\n(Quota > 60 req/min)";
                termLbl = "🛑 429 TOO MANY REQUESTS";
                termId = "node__failed_term_429";
            } else if (excLower.contains("ip") || excLower.contains("denied")) {
                subLbl = "(Async Security Audit)";
                excEdgeLbl = "🚫 403 IpAccessDenied\n(Blocked IP / Untrusted Subnet)";
                termLbl = "🛑 403 FORBIDDEN";
                termId = "node__failed_term_403";
            } else if (excLower.contains("authorized")) {
                subLbl = "(Async RBAC Audit)";
                excEdgeLbl = "🚫 403 UserNotAuthorized\n(Forbidden Role/Permissions)";
                termLbl = "🛑 403 FORBIDDEN";
                termId = "node__failed_term_403";
            } else if (excLower.contains("invalidcredentials") || excLower.contains("credentials")) {
                subLbl = "(Async Auth Handler)";
                excEdgeLbl = "🔒 401 InvalidCredentials\n(Invalid Username/Password)";
                termLbl = "🛑 401 UNAUTHORIZED";
                termId = "node__failed_term_401";
            } else if (excLower.contains("authenticated")) {
                subLbl = "(Async Auth Handler)";
                excEdgeLbl = "🔒 401 UserNotAuthenticated\n(Missing/Invalid Token)";
                termLbl = "🛑 401 UNAUTHORIZED";
                termId = "node__failed_term_401";
            } else if (excCode == 400 || excLower.contains("badrequest") || excLower.contains("validation") || excLower.contains("malformed") || excLower.contains("field")) {
                subLbl = "(Async Validation Audit)";
                excEdgeLbl = "🛑 400 " + excName + "\n(Bad Request / Schema Violation)";
                termLbl = "🛑 400 BAD REQUEST";
                termId = "node__failed_term_400";
            } else if (excCode == 413 || excLower.contains("toolarge") || excLower.contains("payload")) {
                subLbl = "(Async Payload Audit)";
                excEdgeLbl = "🛑 413 PayloadTooLarge\n(Payload Size Exceeded)";
                termLbl = "🛑 413 PAYLOAD TOO LARGE";
                termId = "node__failed_term_413";
            } else {
                subLbl = "(Async Error Handler)";
                excEdgeLbl = "On " + excName + " (" + excCode + ")";
                termLbl = "🛑 FAILED (" + excCode + ")";
                termId = "node__failed_term_" + excCode;
            }

            String desc;
            String originName;
            if (exc.isMiddleware) {
                originName = "Middleware Pipeline (" + (exc.sourceName != null ? exc.sourceName : "Ingress Guards") + ")";
                desc = "Invoked " + (exc.isAsync ? "asynchronously " : "") + "by " + (exc.sourceName != null ? exc.sourceName : "Ingress Guard") + " upon " + excName + " (" + excCode + ").";
            } else if (exc.sourceStepId != null) {
                originName = "Workflow Step (" + exc.sourceStepId + ")";
                desc = "Invoked " + (exc.isAsync ? "asynchronously " : "") + "when step '" + exc.sourceStepId + "' encounters exception " + excName + " (" + excCode + ").";
            } else {
                originName = "Workflow Exception Handler";
                desc = "Invoked upon exception " + excName + " (" + excCode + ").";
            }
            if (exc.message != null && !exc.message.isBlank()) {
                desc += " Policy: " + exc.message;
            }

            // Create Exception Handler Node
            Map<String, Object> excNode = new HashMap<>();
            excNode.put("id", excNodeId);
            excNode.put("shape", "octopus-activity-node");
            excNode.put("label", "🚨 " + excActName);
            excNode.put("width", 220);
            excNode.put("height", 55);

            Map<String, Object> excData = new HashMap<>();
            excData.put("step_id", "exc_" + excActName.toLowerCase());
            excData.put("activity", excActName);
            excData.put("type", "exception_handler");
            excData.put("is_async", exc.isAsync);
            excData.put("exception_type", excName);
            excData.put("status_code", excCode);
            excData.put("source_type", exc.isMiddleware ? "Middleware Exception Pipeline" : "Workflow Step Exception Handler");
            excData.put("source_location", originName);
            excData.put("source_name", exc.sourceName);
            excData.put("source_step_id", exc.sourceStepId);
            excData.put("is_middleware", exc.isMiddleware);
            excData.put("middleware_name", exc.isMiddleware ? exc.sourceName : null);
            excData.put("title", "🚨 Exception Handler: " + excActName);
            excData.put("description", desc);
            excData.put("status", excStatus);
            excData.put("executed", isExcExecuted);
            excNode.put("data", excData);

            if (isExcExecuted) {
                excNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#10b981",
                        "strokeWidth", 3,
                        "fill", "#0f2e22",
                        "filter", "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#6ee7b7",
                        "fontWeight", 600
                    )
                ));
            } else if (hasLiveExecution) {
                excNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#334155",
                        "strokeWidth", 1.5,
                        "fill", "#111827",
                        "opacity", 0.35
                    ),
                    "label", Map.of(
                        "fill", "#64748b",
                        "opacity", 0.45
                    )
                ));
            }
            nodes.add(excNode);

            // Connect Source -> Exception Handler Node
            Map<String, Object> excEdge = new HashMap<>();
            excEdge.put("id", "edge__" + srcId + "__to__" + excNodeId);
            excEdge.put("source", srcId);
            excEdge.put("target", excNodeId);

            String strokeColor = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#1e293b" : "#f59e0b");
            double strokeWidth = isExcExecuted ? 2.5 : 1.5;
            String strokeDash = isExcExecuted ? "5 5" : (hasLiveExecution ? "4 4" : "5 5");
            double opacity = isExcExecuted ? 1.0 : (hasLiveExecution ? 0.25 : 0.8);
            String markerColor = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#334155" : "#f59e0b");

            excEdge.put("attrs", Map.of(
                "line", Map.of(
                    "stroke", strokeColor,
                    "strokeWidth", strokeWidth,
                    "strokeDasharray", strokeDash,
                    "opacity", opacity,
                    "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", markerColor)
                )
            ));

            String textFill = isExcExecuted ? "#f87171" : (hasLiveExecution ? "#475569" : "#fbbf24");
            String rectFill = isExcExecuted ? "#2d1215" : (hasLiveExecution ? "#090d16" : "#0f172a");
            String rectStroke = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#1e293b" : "#f59e0b");
            double labelOpacity = isExcExecuted ? 1.0 : (hasLiveExecution ? 0.3 : 1.0);

            excEdge.put("labels", List.of(Map.of(
                "position", 0.4,
                "attrs", Map.of(
                    "text", Map.of(
                        "text", excEdgeLbl,
                        "fill", textFill,
                        "fontSize", 10,
                        "fontWeight", isExcExecuted ? "bold" : "normal",
                        "opacity", labelOpacity
                    ),
                    "rect", Map.of(
                        "fill", rectFill,
                        "rx", 4,
                        "ry", 4,
                        "stroke", rectStroke,
                        "strokeWidth", 1,
                        "opacity", labelOpacity
                    )
                )
            )));
            edges.add(excEdge);

            // Create / Locate Terminal Failure Node
            Optional<Map<String, Object>> existingTermNode = nodes.stream().filter(n -> termId.equals(n.get("id"))).findFirst();
            if (existingTermNode.isEmpty()) {
                String failedTermStatus = isExcExecuted ? "FAILED" : (hasLiveExecution ? "SKIPPED" : "PENDING");
                int termWidth = Math.max(160, (int)(termLbl.length() * 9.5) + 40);
                Map<String, Object> termNode = new HashMap<>();
                termNode.put("id", termId);
                termNode.put("shape", "octopus-end-node");
                termNode.put("label", termLbl);
                termNode.put("width", termWidth);
                termNode.put("height", 50);

                Map<String, Object> termData = new HashMap<>();
                termData.put("step_id", "failed");
                termData.put("type", "end_failed");
                termData.put("title", "Failed: " + excName);
                termData.put("status", failedTermStatus);
                termData.put("executed", isExcExecuted);
                termNode.put("data", termData);

                if (isExcExecuted) {
                    termNode.put("attrs", Map.of(
                        "body", Map.of(
                            "rx", 25,
                            "ry", 25,
                            "fill", "#450a0a",
                            "stroke", "#ef4444",
                            "strokeWidth", 3,
                            "filter", "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                            "opacity", 1.0
                        ),
                        "label", Map.of(
                            "fill", "#fca5a5",
                            "fontSize", 13,
                            "fontWeight", 700
                        )
                    ));
                } else if (hasLiveExecution) {
                    termNode.put("attrs", Map.of(
                        "body", Map.of(
                            "rx", 25,
                            "ry", 25,
                            "fill", "#0f172a",
                            "stroke", "#334155",
                            "strokeWidth", 1.5,
                            "opacity", 0.35
                        ),
                        "label", Map.of(
                            "fill", "#64748b",
                            "fontSize", 13,
                            "fontWeight", 600,
                            "opacity", 0.45
                        )
                    ));
                }
                nodes.add(termNode);
            } else if (isExcExecuted) {
                Map<String, Object> termNode = existingTermNode.get();
                @SuppressWarnings("unchecked")
                Map<String, Object> termData = (Map<String, Object>) termNode.get("data");
                if (termData != null) {
                    termData.put("status", "FAILED");
                    termData.put("executed", true);
                }
                termNode.put("attrs", Map.of(
                    "body", Map.of(
                        "rx", 25,
                        "ry", 25,
                        "fill", "#450a0a",
                        "stroke", "#ef4444",
                        "strokeWidth", 3,
                        "filter", "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#fca5a5",
                        "fontSize", 13,
                        "fontWeight", 700
                    )
                ));
            }

            // Connect Exception Handler Node -> Terminal Failure Node
            Map<String, Object> excToTermEdge = new HashMap<>();
            excToTermEdge.put("id", "edge__" + excNodeId + "__to__" + termId);
            excToTermEdge.put("source", excNodeId);
            excToTermEdge.put("target", termId);
            excToTermEdge.put("attrs", Map.of(
                "line", Map.of(
                    "stroke", strokeColor,
                    "strokeWidth", strokeWidth,
                    "strokeDasharray", "5 5",
                    "opacity", opacity,
                    "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", markerColor)
                )
            ));
            edges.add(excToTermEdge);
        }

        // 4. Egress Post-Execution Middleware Guards Node
        if (hasEgressGuards) {
            String egressLabel = "📊 Egress Middleware Guards";
            boolean hasPredExec = false;
            for (PrevItem p : finalHeads) {
                if (p.executed) {
                    hasPredExec = true;
                    break;
                }
            }
            boolean egressExecuted = (state != null && state.getStatus() == ExecutionStatus.COMPLETED && hasPredExec);
            String egressStatus = egressExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

            Map<String, Object> egressNode = new HashMap<>();
            egressNode.put("id", "node__egress_guards");
            egressNode.put("shape", "octopus-activity-node");
            egressNode.put("label", egressLabel);
            egressNode.put("width", 220);
            egressNode.put("height", 55);

            Map<String, Object> egressData = new HashMap<>();
            egressData.put("step_id", "egress_guards");
            egressData.put("type", "egress_guards");
            egressData.put("title", "📊 Egress Middleware Guards");
            egressData.put("description", "Post-execution response auditing, latency logging, and metrics dispatch executed following workflow pipeline completion.");
            egressData.put("status", egressStatus);
            egressData.put("executed", egressExecuted);
            List<String> postMwNamesList = new ArrayList<>();
            for (MiddlewareConfig m : postMiddlewares) {
                if (m.getName() != null) postMwNamesList.add(m.getName());
            }
            egressData.put("middlewares", postMwNamesList);
            egressNode.put("data", egressData);

            if (egressExecuted) {
                egressNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#10b981",
                        "strokeWidth", 3,
                        "fill", "#0f2e22",
                        "filter", "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#6ee7b7",
                        "fontWeight", 600
                    )
                ));
            } else {
                egressNode.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#334155",
                        "strokeWidth", 1.5,
                        "fill", "#111827",
                        "opacity", hasLiveExecution ? 0.35 : 0.6
                    ),
                    "label", Map.of(
                        "fill", hasLiveExecution ? "#475569" : "#64748b",
                        "fontWeight", 600
                    )
                ));
            }
            nodes.add(egressNode);

            // Connect finalHeads -> node__egress_guards
            for (PrevItem p : finalHeads) {
                Map<String, Object> edgeAttrs;
                if (p.executed && egressExecuted) {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#10b981",
                            "strokeWidth", 3.5,
                            "targetMarker", Map.of("name", "block", "width", 10, "height", 8, "fill", "#10b981"),
                            "filter", "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                            "opacity", 1.0
                        )
                    );
                } else if (hasLiveExecution) {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#334155",
                            "strokeWidth", 1.5,
                            "targetMarker", Map.of("name", "block", "width", 6, "height", 5, "fill", "#334155"),
                            "opacity", 0.35
                        )
                    );
                } else {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#6366f1",
                            "strokeWidth", 2,
                            "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", "#6366f1")
                        )
                    );
                }
                Map<String, Object> edge = new HashMap<>();
                edge.put("id", "edge__" + p.id + "__to__node__egress_guards");
                edge.put("source", p.id);
                edge.put("target", "node__egress_guards");
                edge.put("attrs", edgeAttrs);
                edges.add(edge);
            }

            finalHeads = List.of(new PrevItem("node__egress_guards", "", "#10b981", egressExecuted));
        }

        // 5. End Node
        boolean hasExecPred = false;
        for (PrevItem p : finalHeads) {
            if (p.executed) {
                hasExecPred = true;
                break;
            }
        }
        boolean endExecuted = (state != null && state.getStatus() == ExecutionStatus.COMPLETED && hasExecPred);
        String endStatus = endExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

        Integer successStatusCode = null;
        if (selectedEp != null && selectedEp.getStatusCode() > 0) {
            successStatusCode = selectedEp.getStatusCode();
        } else if (bp.getApi() != null && !bp.getApi().isEmpty()) {
            successStatusCode = 200;
        }

        String endLabel;
        if (successStatusCode != null) {
            endLabel = switch (successStatusCode) {
                case 200 -> "✅ 200 OK";
                case 201 -> "✅ 201 CREATED";
                case 202 -> "✅ 202 ACCEPTED";
                case 204 -> "✅ 204 NO CONTENT";
                default -> "✅ " + successStatusCode + " OK";
            };
        } else {
            endLabel = "✅ COMPLETED";
        }

        int endWidth = Math.max(140, (int)(endLabel.length() * 9.5) + 36);
        Map<String, Object> endNode = new HashMap<>();
        endNode.put("id", "node__end");
        endNode.put("shape", "octopus-end-node");
        endNode.put("width", endWidth);
        endNode.put("height", 50);
        endNode.put("label", endLabel);
        Map<String, Object> endData = new HashMap<>();
        endData.put("step_id", "end");
        endData.put("type", "end");
        endData.put("title", successStatusCode != null ? "Completed: " + successStatusCode : "Workflow Completed");
        endData.put("status", endStatus);
        endData.put("executed", endExecuted);
        if (successStatusCode != null) {
            endData.put("status_code", successStatusCode);
        }
        endNode.put("data", endData);
        if (endExecuted) {
            endNode.put("attrs", Map.of(
                "body", Map.of(
                    "rx", 25,
                    "ry", 25,
                    "fill", "#064e3b",
                    "stroke", "#10b981",
                    "strokeWidth", 3,
                    "filter", "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    "opacity", 1.0
                ),
                "label", Map.of(
                    "fill", "#6ee7b7",
                    "fontSize", 13,
                    "fontWeight", 700
                )
            ));
        } else {
            endNode.put("attrs", Map.of(
                "body", Map.of(
                    "rx", 25,
                    "ry", 25,
                    "fill", "#0f172a",
                    "stroke", "#334155",
                    "strokeWidth", 1.5,
                    "opacity", hasLiveExecution ? 0.35 : 0.6
                ),
                "label", Map.of(
                    "fill", hasLiveExecution ? "#475569" : "#64748b",
                    "fontSize", 13,
                    "fontWeight", 600
                )
            ));
        }
        nodes.add(endNode);

        for (PrevItem p : finalHeads) {
            Map<String, Object> edgeAttrs;
            if (p.executed && "COMPLETED".equals(endStatus)) {
                edgeAttrs = Map.of(
                    "line", Map.of(
                        "stroke", "#10b981",
                        "strokeWidth", 3.5,
                        "targetMarker", Map.of("name", "block", "width", 10, "height", 8, "fill", "#10b981"),
                        "filter", "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                        "opacity", 1.0
                    )
                );
            } else if (hasLiveExecution) {
                edgeAttrs = Map.of(
                    "line", Map.of(
                        "stroke", "#1e293b",
                        "strokeWidth", 1.5,
                        "strokeDasharray", "4 4",
                        "targetMarker", Map.of("name", "block", "width", 6, "height", 4, "fill", "#334155"),
                        "opacity", 0.25
                    )
                );
            } else {
                edgeAttrs = Map.of(
                    "line", Map.of(
                        "stroke", "#64748b",
                        "strokeWidth", 2,
                        "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", "#64748b"),
                        "opacity", 1.0
                    )
                );
            }

            Map<String, Object> edge = new HashMap<>();
            edge.put("id", "edge_" + p.id + "_to_node__end");
            edge.put("source", p.id);
            edge.put("target", "node__end");
            edge.put("attrs", edgeAttrs);
            edges.add(edge);
        }

        String statusStr = state != null ? state.getStatus().name() : "IDLE";
        String overallStatusStr = state != null ? state.getStatus().name() : "";
        boolean jobExists = (jobId == null || jobId.isBlank() || state != null);

        Map<String, Object> result = new HashMap<>();
        result.put("workflow_name", workflowName);
        result.put("version", bp.getVersion());
        result.put("description", bp.getDescription());
        result.put("namespace", bp.getNamespace());
        result.put("mode", bp.getMode());
        result.put("job_id", jobId != null ? jobId : "");
        result.put("job_exists", jobExists);
        result.put("status", statusStr);
        result.put("overall_status", overallStatusStr);
        result.put("nodes", nodes);
        result.put("edges", edges);

        List<Map<String, Object>> apiList = new ArrayList<>();
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
                apiList.add(epMap);
            }
        }
        result.put("api", apiList);
        result.put("api_endpoints", apiList);
        result.put("routes", routesList);
        if (selectedRoute != null) {
            result.put("selected_route", selectedRoute);
        }
        if (selectedMethod != null) {
            result.put("selected_method", selectedMethod);
        }

        return result;
    }

    private void collectStepExceptions(List<WorkflowStep> steps, List<ExcConfigItem> excConfigs) {
        if (steps == null || steps.isEmpty()) return;
        for (WorkflowStep step : steps) {
            String sId = step.getId() != null ? step.getId() : step.getActivity();
            if (step.getExceptions() != null) {
                for (MiddlewareExceptionConfig exc : step.getExceptions()) {
                    if (exc.getActivity() != null && !exc.getActivity().isBlank()) {
                        excConfigs.add(new ExcConfigItem(exc.getActivity(), exc.getException(), exc.getCode(), exc.isAsync(), false, sId, sId, exc.getMessage()));
                    }
                }
            }
            if (step.getThenSteps() != null) collectStepExceptions(step.getThenSteps(), excConfigs);
            if (step.getElseSteps() != null) collectStepExceptions(step.getElseSteps(), excConfigs);
        }
    }

    private List<PrevItem> processStepList(
        List<WorkflowStep> stepList,
        List<PrevItem> prevHeads,
        String scopePrefix,
        boolean isBranchActive,
        Map<String, String> statusMap,
        Map<String, Double> durationMap,
        Map<String, Object> inputMap,
        Map<String, Object> outputMap,
        Map<String, String> errorMap,
        ExecutionState state,
        boolean hasLiveExecution,
        List<Map<String, Object>> nodes,
        List<Map<String, Object>> edges
    ) {
        List<PrevItem> currPrev = new ArrayList<>(prevHeads);

        for (int idx = 0; idx < stepList.size(); idx++) {
            WorkflowStep step = stepList.get(idx);
            String rawStepId = step.getId() != null && !step.getId().isBlank() ? step.getId() : "step_" + idx;
            String nodeId = (scopePrefix == null || scopePrefix.isBlank())
                ? "node__" + rawStepId
                : "node__" + scopePrefix + "__" + rawStepId;

            boolean executed = false;
            String stepStatus = "PENDING";

            if (statusMap.containsKey(rawStepId)) {
                String st = statusMap.get(rawStepId);
                if ("ASYNC_DISPATCHED".equals(st) || "BACKGROUND_DISPATCHED".equals(st) || "DISPATCHED".equals(st)) {
                    stepStatus = "COMPLETED";
                    executed = true;
                } else {
                    stepStatus = st;
                    executed = "COMPLETED".equals(st) || "RUNNING".equals(st) || "FAILED".equals(st) || "ROLLED_BACK".equals(st);
                }
            } else if (step.getActivity() != null && statusMap.containsKey(step.getActivity())) {
                String st = statusMap.get(step.getActivity());
                if ("ASYNC_DISPATCHED".equals(st) || "BACKGROUND_DISPATCHED".equals(st) || "DISPATCHED".equals(st)) {
                    stepStatus = "COMPLETED";
                    executed = true;
                } else {
                    stepStatus = st;
                    executed = "COMPLETED".equals(st) || "RUNNING".equals(st) || "FAILED".equals(st) || "ROLLED_BACK".equals(st);
                }
            } else if (state != null && rawStepId.equals(state.getCurrentStepId())) {
                stepStatus = "RUNNING";
                executed = true;
            } else if (hasLiveExecution) {
                stepStatus = "SKIPPED";
            }

            boolean isLoop = (step.getLoop() != null && !step.getLoop().isEmpty()) || (step.getForEach() != null && !step.getForEach().isEmpty());
            boolean isAsync = step.isAsync();

            List<WorkflowStep> thenSteps = step.getThenSteps();
            List<WorkflowStep> elseSteps = step.getElseSteps();
            boolean hasBranches = (thenSteps != null && !thenSteps.isEmpty()) || (elseSteps != null && !elseSteps.isEmpty()) || (step.getBranchingStep() != null && !step.getBranchingStep().isBlank()) || step.getCondition() != null || step.getConditions() != null;
            boolean thenExecuted = false;
            boolean elseExecuted = false;

            String shape = "octopus-activity-node";
            String typeStr = "activity";
            String nodeLabel = rawStepId;

            if (isLoop) {
                shape = "octopus-loop-node";
                typeStr = "loop";
                nodeLabel = "⚏ " + rawStepId + "\n[Loop Iteration]";
            } else if (isAsync) {
                shape = "octopus-activity-node";
                typeStr = "async";
                nodeLabel = "⚡ " + rawStepId + "\n[Async Background]";
            } else if (hasBranches) {
                shape = "octopus-branch-node";
                typeStr = "branch";
                nodeLabel = "⚏ " + rawStepId;

                if (thenSteps != null) {
                    for (WorkflowStep ts : thenSteps) {
                        String tst = statusMap.get(ts.getId());
                        String tast = ts.getActivity() != null ? statusMap.get(ts.getActivity()) : null;
                        if ("COMPLETED".equals(tst) || "ASYNC_DISPATCHED".equals(tst) || "COMPLETED".equals(tast) || "ASYNC_DISPATCHED".equals(tast)) {
                            thenExecuted = true;
                            executed = true;
                            stepStatus = "COMPLETED";
                            break;
                        }
                    }
                }
                if (elseSteps != null) {
                    for (WorkflowStep es : elseSteps) {
                        String est = statusMap.get(es.getId());
                        String east = es.getActivity() != null ? statusMap.get(es.getActivity()) : null;
                        if ("COMPLETED".equals(est) || "ASYNC_DISPATCHED".equals(est) || "COMPLETED".equals(east) || "ASYNC_DISPATCHED".equals(east)) {
                            elseExecuted = true;
                            executed = true;
                            stepStatus = "COMPLETED";
                            break;
                        }
                    }
                }
            } else if (step.getHumanStep() != null && !step.getHumanStep().isBlank()) {
                shape = "octopus-human-node";
                typeStr = "human";
            }

            Map<String, Object> node = new HashMap<>();
            node.put("id", nodeId);
            node.put("shape", shape);
            node.put("width", 220);
            node.put("height", 75);
            node.put("label", nodeLabel);

            Map<String, Object> data = new HashMap<>();
            data.put("step_id", rawStepId);
            data.put("title", rawStepId);
            data.put("activity", step.getActivity());
            data.put("status", stepStatus);
            data.put("executed", executed);
            data.put("type", typeStr);
            data.put("is_loop", isLoop);
            data.put("loop", step.getLoop());
            data.put("for_each", step.getForEach());
            data.put("is_async", isAsync);
            data.put("condition", step.getCondition());
            data.put("conditions", step.getConditions());
            data.put("compensation", step.getCompensation());
            data.put("description", step.getDescription());
            data.put("timeout_seconds", step.getTimeoutSeconds());
            data.put("is_output", step.isOutput());
            data.put("index", idx + 1);
            data.put("duration_ms", durationMap.getOrDefault(rawStepId, 0.0));
            data.put("input_payload", inputMap.containsKey(rawStepId) ? inputMap.get(rawStepId) : (step.getActivity() != null ? inputMap.get(step.getActivity()) : null));
            data.put("output_payload", outputMap.containsKey(rawStepId) ? outputMap.get(rawStepId) : (step.getActivity() != null ? outputMap.get(step.getActivity()) : null));
            data.put("error", errorMap.get(rawStepId));

            if (hasBranches) {
                Map<String, Object> branchInfo = new HashMap<>();
                branchInfo.put("has_branches", true);
                branchInfo.put("condition", step.getCondition());
                branchInfo.put("conditions", step.getConditions());

                List<String> thenStepIds = new ArrayList<>();
                if (thenSteps != null) {
                    for (WorkflowStep s : thenSteps) {
                        thenStepIds.add(s.getId() != null ? s.getId() : s.getActivity());
                    }
                }
                branchInfo.put("then_step_ids", thenStepIds);

                List<String> elseStepIds = new ArrayList<>();
                if (elseSteps != null) {
                    for (WorkflowStep s : elseSteps) {
                        elseStepIds.add(s.getId() != null ? s.getId() : s.getActivity());
                    }
                }
                branchInfo.put("else_step_ids", elseStepIds);

                if (hasLiveExecution && (thenExecuted || elseExecuted || executed)) {
                    Map<String, Object> evalInfo = new HashMap<>();
                    evalInfo.put("evaluated", true);
                    evalInfo.put("verdict", thenExecuted ? "TRUE" : (elseExecuted ? "FALSE" : "UNKNOWN"));
                    evalInfo.put("verdict_label", thenExecuted ? "TRUE (Match)" : (elseExecuted ? "FALSE (Else)" : "EXECUTED"));
                    evalInfo.put("branch_taken", thenExecuted ? "THEN" : (elseExecuted ? "ELSE" : "NONE"));

                    if (step.getCondition() != null) {
                        String varName = step.getCondition().getVariable() != null ? step.getCondition().getVariable() : step.getCondition().getField();
                        if (varName != null && !varName.isBlank()) {
                            Object runtimeVal = extractRuntimeVariable(state, varName);
                            evalInfo.put("variable_name", varName);
                            evalInfo.put("runtime_value", runtimeVal);
                            evalInfo.put("expected_value", step.getCondition().getValue());
                            evalInfo.put("operator", step.getCondition().getOperator() != null ? step.getCondition().getOperator() : "==");
                        }
                        if (step.getCondition().getRule() != null) {
                            evalInfo.put("rule", step.getCondition().getRule());
                        }
                    }
                    branchInfo.put("evaluation", evalInfo);
                }
                data.put("branch_info", branchInfo);
            }

            node.put("data", data);

            // Apply Node Path Highlighting
            if ("FAILED".equals(stepStatus) || "ROLLED_BACK".equals(stepStatus)) {
                node.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#ef4444",
                        "strokeWidth", 3,
                        "fill", "#2d1215",
                        "filter", "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#fca5a5",
                        "fontWeight", 700
                    )
                ));
            } else if ("TIMED_OUT".equals(stepStatus)) {
                node.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#f97316",
                        "strokeWidth", 3,
                        "fill", "#2b180d",
                        "filter", "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#fdba74",
                        "fontWeight", 700
                    )
                ));
            } else if ("AWAITING_APPROVAL".equals(stepStatus) || "RUNNING".equals(stepStatus) || "AWAITING_SIGNAL".equals(stepStatus)) {
                node.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#f59e0b",
                        "strokeWidth", 3,
                        "fill", "#2a1d08",
                        "filter", "drop-shadow(0 0 14px rgba(245, 158, 11, 0.85))",
                        "opacity", 1.0
                    ),
                    "label", Map.of(
                        "fill", "#fde68a",
                        "fontWeight", 700
                    )
                ));
            } else if (executed) {
                Map<String, Object> bodyAttrs = new HashMap<>();
                bodyAttrs.put("stroke", "#10b981");
                bodyAttrs.put("strokeWidth", 3);
                bodyAttrs.put("fill", "#0f2e22");
                bodyAttrs.put("filter", "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))");
                bodyAttrs.put("opacity", 1.0);
                if (isLoop) {
                    bodyAttrs.put("strokeDasharray", "5 4");
                }
                node.put("attrs", Map.of(
                    "body", bodyAttrs,
                    "label", Map.of(
                        "fill", "#6ee7b7",
                        "fontWeight", 600
                    )
                ));
            } else if (isLoop) {
                node.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#38bdf8",
                        "strokeWidth", 2,
                        "strokeDasharray", "5 4",
                        "fill", "#0c2333",
                        "filter", "drop-shadow(0 0 12px rgba(56, 189, 248, 0.45))",
                        "opacity", hasLiveExecution ? 0.35 : 1.0
                    ),
                    "label", Map.of(
                        "fill", "#7dd3fc",
                        "fontWeight", 600,
                        "opacity", hasLiveExecution ? 0.45 : 1.0
                    )
                ));
            } else if (hasLiveExecution) {
                node.put("attrs", Map.of(
                    "body", Map.of(
                        "stroke", "#334155",
                        "strokeWidth", 1.5,
                        "fill", "#111827",
                        "opacity", 0.35
                    ),
                    "label", Map.of(
                        "fill", "#64748b",
                        "opacity", 0.45
                    )
                ));
            }

            nodes.add(node);

            // Connect predecessors
            for (PrevItem p : currPrev) {
                Map<String, Object> edgeAttrs;
                if (p.executed && executed) {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#10b981",
                            "strokeWidth", 3.5,
                            "targetMarker", Map.of("name", "block", "width", 10, "height", 8, "fill", "#10b981"),
                            "filter", "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                            "opacity", 1.0
                        )
                    );
                } else if (p.executed && ("RUNNING".equals(stepStatus) || "AWAITING_APPROVAL".equals(stepStatus))) {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#f59e0b",
                            "strokeWidth", 3.5,
                            "strokeDasharray", "6 4",
                            "targetMarker", Map.of("name", "block", "width", 10, "height", 8, "fill", "#f59e0b"),
                            "filter", "drop-shadow(0 0 10px rgba(245, 158, 11, 0.75))",
                            "opacity", 1.0
                        )
                    );
                } else if (hasLiveExecution) {
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", "#1e293b",
                            "strokeWidth", 1.5,
                            "strokeDasharray", "4 4",
                            "targetMarker", Map.of("name", "block", "width", 6, "height", 4, "fill", "#334155"),
                            "opacity", 0.25
                        )
                    );
                } else {
                    String normalStroke = (p.color != null && !p.color.isBlank()) ? p.color : "#64748b";
                    edgeAttrs = Map.of(
                        "line", Map.of(
                            "stroke", normalStroke,
                            "strokeWidth", 2,
                            "targetMarker", Map.of("name", "block", "width", 8, "height", 6, "fill", normalStroke),
                            "opacity", 1.0
                        )
                    );
                }

                Map<String, Object> edge = new HashMap<>();
                edge.put("id", "edge_" + p.id + "_to_" + nodeId);
                edge.put("source", p.id);
                edge.put("target", nodeId);
                edge.put("attrs", edgeAttrs);

                if (!p.label.isEmpty()) {
                    String labelColor = (p.executed && executed) ? "#10b981" : (hasLiveExecution ? "#475569" : p.color);
                    Map<String, Object> labelAttr = new HashMap<>();
                    labelAttr.put("text", Map.of(
                        "text", p.label,
                        "fill", labelColor,
                        "fontSize", 11,
                        "fontWeight", "bold"
                    ));
                    labelAttr.put("rect", Map.of(
                        "fill", "#0f172a",
                        "rx", 4,
                        "ry", 4,
                        "stroke", labelColor,
                        "strokeWidth", 1
                    ));
                    edge.put("labels", List.of(Map.of("attrs", labelAttr)));
                }

                edges.add(edge);
            }

            if (hasBranches) {
                List<PrevItem> branchOuts = new ArrayList<>();
                if (thenSteps != null && !thenSteps.isEmpty()) {
                    String thenScope = (scopePrefix == null || scopePrefix.isBlank()) ? rawStepId + "_then" : scopePrefix + "_" + rawStepId + "_then";
                    boolean thenExec = false;
                    for (WorkflowStep ts : thenSteps) {
                        if ("COMPLETED".equals(statusMap.get(ts.getId())) || (ts.getActivity() != null && "COMPLETED".equals(statusMap.get(ts.getActivity())))) {
                            thenExec = true;
                            break;
                        }
                    }
                    List<PrevItem> thenHeads = List.of(new PrevItem(nodeId, "TRUE (Match)", "#10b981", executed && thenExec));
                    List<PrevItem> thenEnds = processStepList(thenSteps, thenHeads, thenScope, isBranchActive, statusMap, durationMap, inputMap, outputMap, errorMap, state, hasLiveExecution, nodes, edges);
                    branchOuts.addAll(thenEnds);
                }
                if (elseSteps != null && !elseSteps.isEmpty()) {
                    String elseScope = (scopePrefix == null || scopePrefix.isBlank()) ? rawStepId + "_else" : scopePrefix + "_" + rawStepId + "_else";
                    boolean elseExec = false;
                    for (WorkflowStep es : elseSteps) {
                        if ("COMPLETED".equals(statusMap.get(es.getId())) || (es.getActivity() != null && "COMPLETED".equals(statusMap.get(es.getActivity())))) {
                            elseExec = true;
                            break;
                        }
                    }
                    List<PrevItem> elseHeads = List.of(new PrevItem(nodeId, "FALSE (Else)", "#38bdf8", executed && elseExec));
                    List<PrevItem> elseEnds = processStepList(elseSteps, elseHeads, elseScope, isBranchActive, statusMap, durationMap, inputMap, outputMap, errorMap, state, hasLiveExecution, nodes, edges);
                    branchOuts.addAll(elseEnds);
                }
                currPrev = branchOuts;
            } else {
                currPrev = List.of(new PrevItem(nodeId, "", "#64748b", executed));
            }
        }

        return currPrev;
    }

    private Object extractRuntimeVariable(ExecutionState state, String varName) {
        if (state == null || varName == null || varName.isBlank()) return null;
        ConditionEvaluator eval = new ConditionEvaluator();
        if (state.getData() != null) {
            Object val = eval.getNestedField(state.getData(), varName);
            if (val != null) return val;
        }
        if (state.getTraces() != null) {
            for (int i = state.getTraces().size() - 1; i >= 0; i--) {
                StepExecutionTrace t = state.getTraces().get(i);
                if (t.getOutputPayload() != null) {
                    Object val = eval.getNestedField(t.getOutputPayload(), varName);
                    if (val != null) return val;
                }
                if (t.getInputPayload() != null) {
                    Object val = eval.getNestedField(t.getInputPayload(), varName);
                    if (val != null) return val;
                }
            }
        }
        return null;
    }

    private static boolean isPathExcluded(String route, List<String> excludes) {
        if (route == null || route.isBlank() || excludes == null || excludes.isEmpty()) {
            return false;
        }
        for (String pattern : excludes) {
            if (pattern == null || pattern.isBlank()) continue;
            String p = pattern.trim();
            if (p.endsWith("*")) {
                String prefix = p.substring(0, p.length() - 1);
                if (route.startsWith(prefix) || route.contains(prefix)) {
                    return true;
                }
            }
            if (route.equalsIgnoreCase(p) || route.endsWith(p) || p.endsWith(route)) {
                return true;
            }
        }
        return false;
    }

    private static boolean isMiddlewareActiveForRoute(MiddlewareConfig m, String route) {
        if (m == null) return false;
        if (route == null || route.isBlank() || "default".equalsIgnoreCase(route)) {
            return true;
        }
        if (isPathExcluded(route, m.getExcludes())) {
            return false;
        }
        if (m.getIncludes() != null && !m.getIncludes().isEmpty()) {
            return m.getIncludes().stream().anyMatch(inc -> {
                if (inc.endsWith("*")) {
                    String prefix = inc.substring(0, inc.length() - 1);
                    return route.startsWith(prefix) || route.contains(prefix);
                }
                return route.equalsIgnoreCase(inc) || route.endsWith(inc) || inc.endsWith(route);
            });
        }
        return true;
    }
}
