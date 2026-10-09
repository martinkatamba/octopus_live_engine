package io.octopus.live;

import io.octopus.live.adapters.inbound.rest.LiveHttpServer;
import io.octopus.live.adapters.outbound.CallableActivityExecutor;
import io.octopus.live.adapters.outbound.InMemoryAsyncActivityDispatcher;
import io.octopus.live.adapters.outbound.InMemoryStateStore;
import io.octopus.live.adapters.outbound.TemporalAsyncActivityDispatcher;
import io.octopus.live.adapters.outbound.YamlBlueprintRepository;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.WorkflowBlueprint;
import io.octopus.live.domain.ports.IAsyncActivityDispatcher;
import io.octopus.live.domain.ports.ILiveWorkflowUseCase;
import io.octopus.live.domain.ports.IVisualizerUseCase;
import io.octopus.live.domain.services.PipelineManager;
import io.octopus.live.domain.services.VisualizerService;

import io.octopus.live.adapters.outbound.StoreFactory;
import io.octopus.live.domain.ports.IStateStore;

import java.io.IOException;
import java.util.Collections;
import java.util.Map;
import java.util.function.Function;

public class LiveEngine {

    private final YamlBlueprintRepository blueprintRepo;
    private final CallableActivityExecutor activityExecutor;
    private IStateStore stateStore;
    private IAsyncActivityDispatcher asyncDispatcher;
    private PipelineManager pipelineManager;
    private VisualizerService visualizerService;
    private final java.util.concurrent.ConcurrentHashMap<String, Class<?>> modelRegistry = new java.util.concurrent.ConcurrentHashMap<>();
    private final java.util.List<io.octopus.live.domain.services.middleware.LiveMiddleware> customMiddlewares = new java.util.concurrent.CopyOnWriteArrayList<>();
    private String apiKey;
    private LiveHttpServer httpServer;

    public LiveEngine() {
        this(null, null);
    }

    public LiveEngine(String apiKey) {
        this(apiKey, null);
    }

    public LiveEngine(String apiKey, IAsyncActivityDispatcher asyncDispatcher) {
        this.blueprintRepo = new YamlBlueprintRepository();
        this.activityExecutor = new CallableActivityExecutor();
        this.stateStore = new InMemoryStateStore();
        this.asyncDispatcher = asyncDispatcher != null ? asyncDispatcher : new InMemoryAsyncActivityDispatcher();

        this.pipelineManager = new PipelineManager(blueprintRepo, activityExecutor, stateStore, this.asyncDispatcher);
        this.visualizerService = new VisualizerService(blueprintRepo, stateStore);
        this.apiKey = apiKey;
    }

    public IStateStore getStateStore() {
        return stateStore;
    }

    public void setStateStore(IStateStore stateStore) {
        this.stateStore = stateStore != null ? stateStore : new InMemoryStateStore();
        this.pipelineManager = new PipelineManager(blueprintRepo, activityExecutor, this.stateStore, this.asyncDispatcher);
        this.visualizerService = new VisualizerService(blueprintRepo, this.stateStore);
    }

    public IAsyncActivityDispatcher getAsyncDispatcher() {
        return asyncDispatcher;
    }

    public void setAsyncDispatcher(IAsyncActivityDispatcher asyncDispatcher) {
        this.asyncDispatcher = asyncDispatcher != null ? asyncDispatcher : new InMemoryAsyncActivityDispatcher();
        this.pipelineManager = new PipelineManager(blueprintRepo, activityExecutor, stateStore, this.asyncDispatcher);
    }

    public void registerModel(String name, Class<?> modelClass) {
        if (name != null && modelClass != null) {
            modelRegistry.put(name, modelClass);
        }
    }

    public Class<?> getModel(String name) {
        return modelRegistry.get(name);
    }

    public Map<String, Class<?>> getModelRegistry() {
        return Collections.unmodifiableMap(modelRegistry);
    }

    public void addMiddleware(io.octopus.live.domain.services.middleware.LiveMiddleware middleware) {
        if (middleware != null) {
            customMiddlewares.add(middleware);
        }
    }

    public java.util.List<io.octopus.live.domain.services.middleware.LiveMiddleware> getCustomMiddlewares() {
        return customMiddlewares;
    }

    public CallableActivityExecutor getActivityExecutor() {
        return activityExecutor;
    }

    public void registerActivity(String name, Function<Map<String, Object>, Map<String, Object>> handler) {
        activityExecutor.registerActivity(name, handler);
        if (asyncDispatcher instanceof TemporalAsyncActivityDispatcher temporalDispatcher) {
            temporalDispatcher.registerActivity(name);
        }
    }

    public void registerBlueprint(WorkflowBlueprint blueprint) {
        if (blueprint != null) {
            configurePersistenceIfPresent(blueprint);
            autoRegisterBlueprintAsyncActivities(blueprint);
            blueprintRepo.saveBlueprint(blueprint);
        }
    }

    public void loadBlueprintsFromDirectory(String dirPath) {
        blueprintRepo.loadFromDirectory(dirPath);
        for (WorkflowBlueprint bp : blueprintRepo.listBlueprints()) {
            configurePersistenceIfPresent(bp);
            autoRegisterBlueprintAsyncActivities(bp);
        }
    }

    private void autoRegisterBlueprintAsyncActivities(WorkflowBlueprint blueprint) {
        if (blueprint.getSteps() != null) {
            for (io.octopus.live.domain.models.WorkflowStep step : blueprint.getSteps()) {
                if (step.isAsync()) {
                    ensureActivityRegistered(step.getActivity());
                }
                if (step.getCompensation() != null && !step.getCompensation().isBlank()) {
                    ensureActivityRegistered(step.getCompensation());
                }
                if (step.getExceptions() != null) {
                    for (io.octopus.live.domain.models.MiddlewareExceptionConfig exc : step.getExceptions()) {
                        if (exc.getActivity() != null && !exc.getActivity().isBlank()) {
                            ensureActivityRegistered(exc.getActivity());
                        }
                    }
                }
            }
        }
        // Auto-register standard middleware async activities as well
        ensureActivityRegistered("RecordResponseAuditLog");
        ensureActivityRegistered("LogSecurityAuditViolation");
    }

    private void ensureActivityRegistered(String activityName) {
        if (activityName == null || activityName.isBlank()) {
            return;
        }
        if (!activityExecutor.hasActivity(activityName)) {
            registerActivity(activityName, input -> {
                System.out.println("⚡ [ENGINE AUTO-WORKER] Executing auto-registered async activity: " + activityName + " -> " + input);
                Map<String, Object> res = new java.util.HashMap<>();
                res.put("status", "SUCCESS");
                res.put("activity", activityName);
                res.put("executed_by", "OctopusEngineNativeWorker");
                res.put("timestamp", java.time.Instant.now().toString());
                return res;
            });
        }
    }

    @SuppressWarnings("unchecked")
    private void configurePersistenceIfPresent(WorkflowBlueprint blueprint) {
        if (blueprint.getPersistence() != null) {
            Map<String, Object> p = blueprint.getPersistence();
            if (p.containsKey("store") || p.containsKey("type") || p.containsKey("dsn") || p.containsKey("url") || p.containsKey("connection_string")) {
                IStateStore store = StoreFactory.create(null, p);
                if (store != null) {
                    setStateStore(store);
                }
            }
            if (p.containsKey("temporal")) {
                Object tempObj = p.get("temporal");
                if (tempObj instanceof Map) {
                    Map<String, Object> tempCfg = (Map<String, Object>) tempObj;
                    String host = tempCfg.get("host") != null ? tempCfg.get("host").toString() : "localhost:7233";
                    String ns = tempCfg.get("namespace") != null ? tempCfg.get("namespace").toString() : "default";
                    String tq = tempCfg.get("task_queue") != null ? tempCfg.get("task_queue").toString() : "octopus-temporal-queue";
                    
                    if (this.asyncDispatcher instanceof InMemoryAsyncActivityDispatcher) {
                        TemporalAsyncActivityDispatcher temporalDispatcher = new TemporalAsyncActivityDispatcher(host, ns, tq);
                        setAsyncDispatcher(temporalDispatcher);
                    }
                }
            }
        }
    }

    public ExecutionState executeSync(String workflowName, Map<String, Object> payload) {
        return pipelineManager.executeSync(workflowName, payload);
    }

    public ExecutionState executeEndpointSync(String workflowName, WorkflowBlueprint.ApiEndpointConfig endpoint, String route, String method, Map<String, Object> payload) {
        return pipelineManager.executeEndpointSync(workflowName, endpoint, route, method, payload);
    }

    public ExecutionState initiateAsync(String workflowName, Map<String, Object> payload) {
        return pipelineManager.initiateAsync(workflowName, payload);
    }

    public ILiveWorkflowUseCase getWorkflowUseCase() {
        return pipelineManager;
    }

    public IVisualizerUseCase getVisualizerUseCase() {
        return visualizerService;
    }

    public VisualizerService getVisualizerService() {
        return visualizerService;
    }

    public String getApiKey() {
        return apiKey;
    }

    public void setApiKey(String apiKey) {
        this.apiKey = apiKey;
    }

    private boolean embeddedWorkerEnabled = true;

    public boolean isEmbeddedWorkerEnabled() {
        return embeddedWorkerEnabled;
    }

    public void setEmbeddedWorkerEnabled(boolean embeddedWorkerEnabled) {
        this.embeddedWorkerEnabled = embeddedWorkerEnabled;
    }

    public void startWorker() {
        if (asyncDispatcher instanceof TemporalAsyncActivityDispatcher temporalDispatcher) {
            temporalDispatcher.startWorker(activityExecutor);
        }
    }

    public void startServer(int port) throws IOException {
        if (embeddedWorkerEnabled && asyncDispatcher instanceof TemporalAsyncActivityDispatcher temporalDispatcher) {
            temporalDispatcher.startWorker(activityExecutor);
        }
        httpServer = new LiveHttpServer(this, port);
        httpServer.start();
    }

    public void stopServer() {
        if (httpServer != null) {
            httpServer.stop();
        }
        if (asyncDispatcher instanceof TemporalAsyncActivityDispatcher temporalDispatcher) {
            temporalDispatcher.shutdown();
        }
    }
}
