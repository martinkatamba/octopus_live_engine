package io.octopus.live.domain.services;

import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.ExecutionStatus;
import io.octopus.live.domain.models.LiveEvent;
import io.octopus.live.domain.models.MiddlewareExceptionConfig;
import io.octopus.live.domain.models.StepExecutionTrace;
import io.octopus.live.domain.models.WorkflowBlueprint;
import io.octopus.live.domain.models.WorkflowStep;
import io.octopus.live.domain.ports.IActivityExecutor;
import io.octopus.live.domain.ports.IBlueprintRepository;
import io.octopus.live.domain.ports.ILiveWorkflowUseCase;
import io.octopus.live.domain.ports.IStateStore;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.time.Instant;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Flow;

import io.octopus.live.domain.ports.IAsyncActivityDispatcher;
import io.octopus.live.adapters.outbound.InMemoryAsyncActivityDispatcher;

public class PipelineManager implements ILiveWorkflowUseCase {
    private final IBlueprintRepository blueprintRepo;
    private final IActivityExecutor activityExecutor;
    private final IStateStore stateStore;
    private final IAsyncActivityDispatcher asyncDispatcher;
    private final ConditionEvaluator conditionEvaluator;
    private final SagaManager sagaManager;
    private final IdempotencyManager idempotencyManager;

    public PipelineManager(
            IBlueprintRepository blueprintRepo,
            IActivityExecutor activityExecutor,
            IStateStore stateStore
    ) {
        this(blueprintRepo, activityExecutor, stateStore, new InMemoryAsyncActivityDispatcher());
    }

    public PipelineManager(
            IBlueprintRepository blueprintRepo,
            IActivityExecutor activityExecutor,
            IStateStore stateStore,
            IAsyncActivityDispatcher asyncDispatcher
    ) {
        this.blueprintRepo = blueprintRepo;
        this.activityExecutor = activityExecutor;
        this.stateStore = stateStore;
        this.asyncDispatcher = asyncDispatcher != null ? asyncDispatcher : new InMemoryAsyncActivityDispatcher();
        this.conditionEvaluator = new ConditionEvaluator();
        this.sagaManager = new SagaManager(activityExecutor);
        this.idempotencyManager = new IdempotencyManager();
    }

    @Override
    public ExecutionState executeSync(String workflowName, Map<String, Object> payload) {
        return executeEndpointSync(workflowName, null, "", "", payload);
    }

    @Override
    public ExecutionState executeEndpointSync(
            String workflowName,
            WorkflowBlueprint.ApiEndpointConfig endpoint,
            String route,
            String method,
            Map<String, Object> payload
    ) {
        WorkflowBlueprint blueprint = blueprintRepo.getBlueprint(workflowName);
        if (blueprint == null) {
            throw new IllegalArgumentException("Workflow blueprint '" + workflowName + "' not found.");
        }

        String jobId = payload != null && payload.get("job_id") != null && !payload.get("job_id").toString().isBlank()
                ? payload.get("job_id").toString()
                : "job_" + UUID.randomUUID().toString().replace("-", "");

        ExecutionState cached = idempotencyManager.get(jobId);
        if (cached != null) {
            return cached;
        }

        WorkflowBlueprint.ApiEndpointConfig targetEndpoint = endpoint;
        if (targetEndpoint == null && route != null && !route.isBlank() && blueprint.getApi() != null) {
            for (WorkflowBlueprint.ApiEndpointConfig ep : blueprint.getApi()) {
                if (route.equalsIgnoreCase(ep.getRoute()) || route.endsWith(ep.getRoute()) || ep.getRoute().endsWith(route)) {
                    if (method == null || method.isBlank() || ep.getMethod() == null || method.equalsIgnoreCase(ep.getMethod())) {
                        targetEndpoint = ep;
                        break;
                    }
                }
            }
        }

        List<WorkflowStep> stepsToRun = (targetEndpoint != null && targetEndpoint.getSteps() != null && !targetEndpoint.getSteps().isEmpty())
                ? new ArrayList<>(targetEndpoint.getSteps())
                : ((blueprint.getSteps() != null && !blueprint.getSteps().isEmpty())
                    ? new ArrayList<>(blueprint.getSteps())
                    : (blueprint.getApi() != null && !blueprint.getApi().isEmpty() && blueprint.getApi().get(0).getSteps() != null
                        ? new ArrayList<>(blueprint.getApi().get(0).getSteps())
                        : new ArrayList<>()));

        String reqRoute = (route != null && !route.isBlank()) ? route : (targetEndpoint != null ? targetEndpoint.getRoute() : "");
        String reqMethod = (method != null && !method.isBlank()) ? method : (targetEndpoint != null ? targetEndpoint.getMethod() : "");
        String clientHost = payload != null && payload.get("client_host") != null
                ? payload.get("client_host").toString()
                : (payload != null && payload.get("client_ip") != null ? payload.get("client_ip").toString() : "127.0.0.1");

        long startNanos = System.nanoTime();
        ExecutionState state = new ExecutionState();
        state.setJobId(jobId);
        state.setWorkflowName(workflowName);
        state.setStatus(ExecutionStatus.RUNNING);
        state.setRoute(reqRoute);
        state.setMethod(reqMethod);
        state.setClientHost(clientHost);
        state.setTotalSteps(stepsToRun.size());
        state.setData(payload != null ? new HashMap<>(payload) : new HashMap<>());
        state.getData().put("job_id", jobId);
        state.setStartedAt(Instant.now().toString());

        stateStore.saveState(state);
        stateStore.publishEvent(jobId, new LiveEvent("workflow_started", state));

        List<SagaManager.SagaAction> sagaActions = new ArrayList<>();
        List<WorkflowStep> steps = stepsToRun;
        int stepIndex = 0;

        while (stepIndex < steps.size()) {
            WorkflowStep step = steps.get(stepIndex);
            state.setCurrentIndex(stepIndex);
            state.setCurrentStepId(step.getId());

            // Pre-execution delay
            if (step.getDelaySeconds() > 0) {
                try {
                    Thread.sleep((long) (step.getDelaySeconds() * 1000));
                } catch (InterruptedException ignored) {}
            }

            // Early termination / settlement
            if (step.isTerminate()) {
                String reason = step.getReason() != null ? step.getReason() : "Workflow settled gracefully";
                StepExecutionTrace settleTrace = new StepExecutionTrace();
                settleTrace.setStepId(step.getId());
                settleTrace.setActivityName("terminate_settle");
                settleTrace.setStatus("COMPLETED");
                Map<String, Object> settleOut = new HashMap<>();
                settleOut.put("settled", true);
                settleOut.put("reason", reason);
                settleTrace.setOutputPayload(maskPayload(blueprint, step, settleOut));
                settleTrace.setStartedAt(Instant.now().toString());
                settleTrace.setFinishedAt(Instant.now().toString());
                state.getTraces().add(settleTrace);
                state.setStatus(ExecutionStatus.COMPLETED);
                break;
            }

            // Declarative compensation trigger
            if (step.isCompensate()) {
                String reason = step.getReason() != null ? step.getReason() : "Declarative compensation triggered at step '" + step.getId() + "'";
                state.setError(reason);
                state.setStatus(ExecutionStatus.ROLLED_BACK);

                if ("async".equalsIgnoreCase(blueprint.getCompensationMode())) {
                    List<SagaManager.SagaAction> actionsCopy = new ArrayList<>(sagaActions);
                    CompletableFuture.runAsync(() -> sagaManager.executeCompensations(actionsCopy));
                } else {
                    sagaManager.executeCompensations(sagaActions);
                }

                state.setFinishedAt(Instant.now().toString());
                stateStore.saveState(state);
                stateStore.publishEvent(jobId, new LiveEvent("workflow_rolled_back", state));
                throw new IllegalStateException(reason);
            }

            List<WorkflowStep> thenSteps = step.getThenSteps();
            List<WorkflowStep> elseSteps = step.getElseSteps();
            boolean hasBranches = (thenSteps != null && !thenSteps.isEmpty()) || (elseSteps != null && !elseSteps.isEmpty()) || (step.getBranchingStep() != null && !step.getBranchingStep().isBlank());

            // Condition evaluation & branching
            if (hasBranches || step.getCondition() != null || step.getConditions() != null) {
                boolean condResult = conditionEvaluator.evaluateStep(step, state.getData());
                if (!condResult && !hasBranches) {
                    // Step condition not met: skip executing this step activity
                    stepIndex++;
                    continue;
                }

                if (condResult) {
                    if (thenSteps != null && !thenSteps.isEmpty()) {
                        steps.addAll(stepIndex + 1, thenSteps);
                        state.setTotalSteps(steps.size());
                    }
                } else {
                    if (elseSteps != null && !elseSteps.isEmpty()) {
                        steps.addAll(stepIndex + 1, elseSteps);
                        state.setTotalSteps(steps.size());
                    }
                }

                if ((step.getActivity() == null || step.getActivity().isBlank()) && step.getLoop() == null && step.getForEach() == null) {
                    stepIndex++;
                    continue;
                }
            }

            // Loop execution
            if (step.getLoop() != null || step.getForEach() != null) {
                executeLoopStep(step, blueprint, state);
                stepIndex++;
                continue;
            }

            // Activity Execution (Sync or Async)
            if (step.getActivity() != null && !step.getActivity().trim().isEmpty()) {
                if (step.isAsync()) {
                    dispatchAsyncActivity(step, blueprint, state);
                } else {
                    Map<String, Object> stepStartedData = new HashMap<>();
                    stepStartedData.put("step_id", step.getId());
                    stepStartedData.put("activity", step.getActivity());
                    stepStartedData.put("index", stepIndex);
                    stepStartedData.put("total_steps", state.getTotalSteps());
                    stateStore.publishEvent(jobId, new LiveEvent("step_started", stepStartedData));

                    long startTime = System.nanoTime();
                    String startIso = Instant.now().toString();

                    try {
                        Map<String, Object> outPayload = activityExecutor.executeActivity(step.getActivity(), state.getData());
                        long durationNano = System.nanoTime() - startTime;
                        double durationMs = durationNano / 1_000_000.0;

                        StepExecutionTrace trace = new StepExecutionTrace();
                        trace.setStepId(step.getId());
                        trace.setActivityName(step.getActivity());
                        trace.setStatus("COMPLETED");
                        trace.setInputPayload(maskPayload(blueprint, step, state.getData()));
                        trace.setOutputPayload(maskPayload(blueprint, step, outPayload != null ? outPayload : new HashMap<>()));
                        trace.setStartedAt(startIso);
                        trace.setFinishedAt(Instant.now().toString());
                        trace.setDurationMs(durationMs);

                        state.getTraces().add(trace);

                        if (outPayload != null) {
                            state.getData().putAll(outPayload);
                        }

                        if (step.isOutput() || state.getOutputData() == null) {
                            state.setOutputData(outPayload != null ? new HashMap<>(outPayload) : new HashMap<>(state.getData()));
                        }

                        if (step.getCompensation() != null && !step.getCompensation().trim().isEmpty()) {
                            sagaActions.add(new SagaManager.SagaAction(step.getId(), step.getCompensation(), new HashMap<>(state.getData())));
                        }

                        stateStore.publishEvent(jobId, new LiveEvent("step_completed", trace));
                    } catch (Exception ex) {
                        long durationNano = System.nanoTime() - startTime;
                        double durationMs = durationNano / 1_000_000.0;

                        StepExecutionTrace failTrace = new StepExecutionTrace();
                        failTrace.setStepId(step.getId());
                        failTrace.setActivityName(step.getActivity());
                        failTrace.setStatus("FAILED");
                        failTrace.setErrorMessage(ex.getMessage());
                        failTrace.setInputPayload(maskPayload(blueprint, step, state.getData()));
                        failTrace.setStartedAt(startIso);
                        failTrace.setFinishedAt(Instant.now().toString());
                        failTrace.setDurationMs(durationMs);

                        state.getTraces().add(failTrace);
                        state.setError(ex.getMessage());
                        state.setStatus(ExecutionStatus.FAILED);

                        MiddlewareExceptionConfig excCfg = findExceptionConfig(step, blueprint, ex);
                        if (excCfg != null) {
                            String msg = excCfg.getMessage() != null && !excCfg.getMessage().isBlank() ? excCfg.getMessage() : ex.getMessage();
                            int code = excCfg.getCode() != 0 ? excCfg.getCode() : 400;
                            state.setError(msg);
                            state.setErrorCode(code);
                            state.setErrorType(excCfg.getException());

                            if (excCfg.getActivity() != null && !excCfg.getActivity().isBlank() && activityExecutor != null) {
                                StringWriter sw = new StringWriter();
                                ex.printStackTrace(new PrintWriter(sw));
                                String stackTrace = sw.toString();

                                Map<String, Object> actPayload = new HashMap<>();
                                actPayload.put("exception", excCfg.getException());
                                actPayload.put("error", excCfg.getException());
                                actPayload.put("error_type", ex.getClass().getSimpleName());
                                actPayload.put("error_message", ex.getMessage() != null ? ex.getMessage() : "");
                                actPayload.put("message", msg);
                                actPayload.put("status_code", code);
                                actPayload.put("job_id", jobId);
                                actPayload.put("workflow_name", blueprint.getWorkflowName());
                                actPayload.put("failing_step_id", step.getId());
                                actPayload.put("route", state.getRoute() != null ? state.getRoute() : "");
                                actPayload.put("path", state.getRoute() != null ? state.getRoute() : "");
                                actPayload.put("method", state.getMethod() != null ? state.getMethod() : "POST");
                                actPayload.put("client_host", "127.0.0.1");
                                actPayload.put("stack_trace", stackTrace);
                                actPayload.put("traceback", stackTrace);
                                actPayload.put("timestamp", Instant.now().toString());
                                actPayload.put("data", new HashMap<>(state.getData()));

                                if (excCfg.isAsync()) {
                                    StepExecutionTrace excTrace = new StepExecutionTrace();
                                    excTrace.setStepId("exc_" + excCfg.getActivity().toLowerCase());
                                    excTrace.setActivityName(excCfg.getActivity());
                                    excTrace.setStatus("ASYNC_DISPATCHED");
                                    excTrace.setAsync(true);
                                    excTrace.setInputPayload(maskPayload(blueprint, step, actPayload));
                                    Map<String, Object> out = new HashMap<>();
                                    out.put("async_dispatched", true);
                                    out.put("dispatcher", asyncDispatcher.getDispatcherType());
                                    excTrace.setOutputPayload(out);
                                    excTrace.setStartedAt(Instant.now().toString());
                                    excTrace.setFinishedAt(Instant.now().toString());
                                    state.getTraces().add(excTrace);

                                    asyncDispatcher.dispatch(excCfg.getActivity(), actPayload, jobId, "exc_" + excCfg.getActivity().toLowerCase(), activityExecutor);
                                } else {
                                    long actStart = System.nanoTime();
                                    try {
                                        Map<String, Object> actRes = activityExecutor.executeActivity(excCfg.getActivity(), actPayload);
                                        double actDurMs = (System.nanoTime() - actStart) / 1_000_000.0;
                                        StepExecutionTrace excTrace = new StepExecutionTrace();
                                        excTrace.setStepId("exc_" + excCfg.getActivity().toLowerCase());
                                        excTrace.setActivityName(excCfg.getActivity());
                                        excTrace.setStatus("COMPLETED");
                                        excTrace.setInputPayload(maskPayload(blueprint, step, actPayload));
                                        excTrace.setOutputPayload(maskPayload(blueprint, step, actRes != null ? actRes : new HashMap<>()));
                                        excTrace.setDurationMs(actDurMs);
                                        excTrace.setStartedAt(Instant.now().toString());
                                        excTrace.setFinishedAt(Instant.now().toString());
                                        state.getTraces().add(excTrace);
                                    } catch (Exception actErr) {
                                        // logged
                                    }
                                }
                            }
                        }

                        // Trigger Saga Rollback based on mode
                        if (!sagaActions.isEmpty()) {
                            if ("async".equalsIgnoreCase(blueprint.getCompensationMode())) {
                                List<SagaManager.SagaAction> actionsCopy = new ArrayList<>(sagaActions);
                                for (SagaManager.SagaAction action : actionsCopy) {
                                    asyncDispatcher.dispatch(action.getCompensation(), action.getPayload(), jobId, "saga_" + action.getStepId(), activityExecutor);
                                }
                            } else {
                                sagaManager.executeCompensations(sagaActions);
                            }
                            state.setStatus(ExecutionStatus.ROLLED_BACK);
                        }

                        state.setFinishedAt(Instant.now().toString());
                        state.setDurationMs(Math.round((System.nanoTime() - startNanos) / 10000.0) / 100.0);

                        stateStore.saveState(state);
                        stateStore.publishEvent(jobId, new LiveEvent("workflow_failed", state));
                        throw new RuntimeException(ex);
                    }
                }
            }

            // Handle jump to step index
            if (step.getJumpToStepIndex() != null && step.getJumpToStepIndex() >= 0 && step.getJumpToStepIndex() < steps.size()) {
                stepIndex = step.getJumpToStepIndex();
                continue;
            }
            if (step.getRewindToStepIndex() != null && step.getRewindToStepIndex() >= 0 && step.getRewindToStepIndex() < steps.size()) {
                stepIndex = step.getRewindToStepIndex();
                continue;
            }

            // Handle jump_to named step
            if (step.getJumpTo() != null && !step.getJumpTo().isBlank()) {
                int targetIdx = -1;
                for (int i = 0; i < steps.size(); i++) {
                    if (step.getJumpTo().equals(steps.get(i).getId())) {
                        targetIdx = i;
                        break;
                    }
                }
                if (targetIdx >= 0) {
                    stepIndex = targetIdx;
                    continue;
                }
            }

            stepIndex++;
        }

        if (state.getStatus() == ExecutionStatus.RUNNING) {
            state.setStatus(ExecutionStatus.COMPLETED);
        }
        if (state.getOutputData() == null) {
            state.setOutputData(new HashMap<>(state.getData()));
        }
        state.setFinishedAt(Instant.now().toString());
        state.setDurationMs(Math.round((System.nanoTime() - startNanos) / 10000.0) / 100.0);
        idempotencyManager.set(jobId, state);
        stateStore.saveState(state);

        stateStore.publishEvent(jobId, new LiveEvent("workflow_completed", state));

        return state;
    }

    private void dispatchAsyncActivity(WorkflowStep step, WorkflowBlueprint blueprint, ExecutionState state) {
        StepExecutionTrace trace = new StepExecutionTrace();
        trace.setStepId(step.getId());
        trace.setActivityName(step.getActivity());
        trace.setStatus("ASYNC_DISPATCHED");
        trace.setAsync(true);
        trace.setInputPayload(maskPayload(blueprint, step, state.getData()));
        Map<String, Object> out = new HashMap<>();
        out.put("async_dispatched", true);
        out.put("dispatcher", asyncDispatcher.getDispatcherType());
        trace.setOutputPayload(out);
        trace.setStartedAt(Instant.now().toString());
        trace.setFinishedAt(Instant.now().toString());
        state.getTraces().add(trace);

        Map<String, Object> dataCopy = new HashMap<>(state.getData());
        String actName = step.getActivity();
        asyncDispatcher.dispatch(actName, dataCopy, state.getJobId(), step.getId(), activityExecutor);
    }

    @SuppressWarnings("unchecked")
    private void executeLoopStep(WorkflowStep step, WorkflowBlueprint blueprint, ExecutionState state) {
        Map<String, Object> loopCfg = step.getLoop() != null ? step.getLoop() : step.getForEach();
        if (loopCfg == null) return;

        List<Object> items = new ArrayList<>();
        String itemsField = null;
        if (loopCfg.get("items_field") != null) itemsField = loopCfg.get("items_field").toString();
        else if (loopCfg.get("items") instanceof String) itemsField = loopCfg.get("items").toString();

        if (itemsField != null && state.getData().containsKey(itemsField)) {
            Object raw = state.getData().get(itemsField);
            if (raw instanceof List) {
                items.addAll((List<?>) raw);
            }
        } else if (loopCfg.get("items") instanceof List) {
            items.addAll((List<?>) loopCfg.get("items"));
        }

        int maxIter = 100;
        if (loopCfg.get("max_iterations") instanceof Number) {
            maxIter = ((Number) loopCfg.get("max_iterations")).intValue();
        }
        String itemVarName = loopCfg.get("item_variable") != null ? loopCfg.get("item_variable").toString() : "item";
        List<Object> loopResults = new ArrayList<>();

        for (int i = 0; i < Math.min(items.size(), maxIter); i++) {
            Object item = items.get(i);
            Map<String, Object> itemPayload = new HashMap<>(state.getData());
            itemPayload.put("_item", item);
            itemPayload.put("_index", i);
            itemPayload.put(itemVarName, item);

            if (step.getActivity() != null && !step.getActivity().isBlank()) {
                Map<String, Object> out = activityExecutor.executeActivity(step.getActivity(), itemPayload);
                if (out != null) {
                    loopResults.add(out);
                    state.getData().putAll(out);
                }
            }
        }

        state.getData().put(step.getId() + "_results", loopResults);
        StepExecutionTrace trace = new StepExecutionTrace();
        trace.setStepId(step.getId());
        trace.setActivityName(step.getActivity() != null ? step.getActivity() : "loop");
        trace.setStatus("COMPLETED");
        trace.setInputPayload(maskPayload(blueprint, step, state.getData()));
        Map<String, Object> traceOut = new HashMap<>();
        traceOut.put("iterations", loopResults.size());
        traceOut.put("results", loopResults);
        trace.setOutputPayload(maskPayload(blueprint, step, traceOut));
        trace.setStartedAt(Instant.now().toString());
        trace.setFinishedAt(Instant.now().toString());
        state.getTraces().add(trace);
    }

    @Override
    public ExecutionState initiateAsync(String workflowName, Map<String, Object> payload) {
        String jobId = payload != null && payload.get("job_id") != null && !payload.get("job_id").toString().isBlank()
                ? payload.get("job_id").toString()
                : "job_" + UUID.randomUUID().toString().replace("-", "");

        ExecutionState state = new ExecutionState();
        state.setJobId(jobId);
        state.setWorkflowName(workflowName);
        state.setStatus(ExecutionStatus.RUNNING);
        state.setStartedAt(Instant.now().toString());
        state.setData(payload != null ? payload : new HashMap<>());

        stateStore.saveState(state);

        CompletableFuture.runAsync(() -> {
            try {
                executeSync(workflowName, payload);
            } catch (Exception ignored) {}
        });

        return state;
    }

    @Override
    public void streamEvents(String jobId, java.util.function.Consumer<LiveEvent> consumer) {
        stateStore.subscribeEvents(jobId, consumer);
    }

    public IStateStore getStateStore() {
        return stateStore;
    }

    @Override
    public ExecutionState getExecutionState(String jobId) {
        return stateStore.getState(jobId);
    }

    @Override
    public List<ExecutionState> listExecutionStates(String workflowName, int limit) {
        return stateStore.listStates(workflowName, limit);
    }

    private MiddlewareExceptionConfig findExceptionConfig(WorkflowStep step, WorkflowBlueprint blueprint, Exception ex) {
        if (ex == null) {
            return null;
        }
        String exType = ex.getClass().getSimpleName();
        String exMsg = ex.getMessage() != null ? ex.getMessage() : "";

        // 1. Check step-level declared exceptions first
        if (step != null && step.getExceptions() != null && !step.getExceptions().isEmpty()) {
            for (var ec : step.getExceptions()) {
                if (ec.getException() == null || ec.getException().isBlank()) continue;

                if (ec.getException().equalsIgnoreCase(exType) ||
                    exType.toLowerCase().contains(ec.getException().toLowerCase()) ||
                    ec.getException().toLowerCase().contains(exType.toLowerCase())) {
                    return ec;
                }
                if (!exMsg.isBlank() && exMsg.toLowerCase().contains(ec.getException().toLowerCase())) {
                    return ec;
                }
            }
        }

        // 2. Check workflow-level middleware exceptions
        if (blueprint != null && blueprint.getMiddlewares() != null) {
            for (var mw : blueprint.getMiddlewares()) {
                if (mw.getExceptions() == null) continue;
                for (var ec : mw.getExceptions()) {
                    if (ec.getException() == null || ec.getException().isBlank()) continue;

                    if (ec.getException().equalsIgnoreCase(exType) ||
                        exType.toLowerCase().contains(ec.getException().toLowerCase()) ||
                        ec.getException().toLowerCase().contains(exType.toLowerCase())) {
                        return ec;
                    }
                    if (!exMsg.isBlank() && exMsg.toLowerCase().contains(ec.getException().toLowerCase())) {
                        return ec;
                    }
                }
            }

            // 3. Middleware Wildcard '*'
            for (var mw : blueprint.getMiddlewares()) {
                if (mw.getExceptions() == null) continue;
                for (var ec : mw.getExceptions()) {
                    if ("*".equals(ec.getException()) || "exception".equalsIgnoreCase(ec.getException()) || "throwable".equalsIgnoreCase(ec.getException())) {
                        return ec;
                    }
                }
            }
        }

        // 4. Global Exception Handler (Declarative Blueprint safety net)
        if (blueprint != null) {
            var gh = blueprint.getGlobalExceptionHandler();
            if (gh != null && gh.getActivity() != null && !gh.getActivity().isBlank()) {
                MiddlewareExceptionConfig cfg = new MiddlewareExceptionConfig();
                cfg.setException("*");
                cfg.setCode(gh.getCode() != 0 ? gh.getCode() : 500);
                cfg.setMessage(gh.getMessage());
                cfg.setActivity(gh.getActivity());
                cfg.setAsync(gh.isAsync());
                cfg.setHeaders(gh.getHeaders());
                return cfg;
            }
        }

        return null;
    }

    private Map<String, Object> maskPayload(
            WorkflowBlueprint blueprint,
            WorkflowStep step,
            Map<String, Object> payload
    ) {
        if (payload == null) return new HashMap<>();
        WorkflowBlueprint.MaskConfigResult maskCfg = blueprint.getMaskConfig(step);
        return MaskingService.maskPayload(payload, maskCfg.getFields(), maskCfg.getMaskValue());
    }
}
