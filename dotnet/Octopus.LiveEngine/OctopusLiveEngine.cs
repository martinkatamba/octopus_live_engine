using Microsoft.AspNetCore.Routing;
using Octopus.LiveEngine.Adapters.Inbound;
using Octopus.LiveEngine.Adapters.Outbound;
using Octopus.LiveEngine.Core;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using Octopus.LiveEngine.Domain.Services;

namespace Octopus.LiveEngine;

public class OctopusLiveEngine : IAsyncDisposable, IDisposable
{
    public YamlBlueprintRepository BlueprintRepo { get; }
    public CallableActivityExecutor ActivityExecutor { get; }
    public IStateStore StateStore { get; private set; }
    public IAsyncActivityDispatcher AsyncDispatcher { get; private set; }
    public PipelineManager PipelineManager { get; private set; }
    public VisualizerService VisualizerService { get; private set; }
    public string? ApiKey { get; set; }
    private readonly Dictionary<string, Type> _models = new();
    public Dictionary<string, CustomMiddlewareFactory> CustomMiddlewares { get; } = new(StringComparer.OrdinalIgnoreCase);
    private bool _embeddedWorkerEnabled = true;
    private readonly CorePipelineManager? _core;

    /// <summary>
    /// <paramref name="engine"/> picks what runs the workflows: "dotnet" (this SDK's
    /// own engine), "core" (the shared Rust core, octopus-ffi) or "auto" (the core when
    /// its library loads). Without it, the <c>OCTOPUS_LIVE_ENGINE</c> environment
    /// variable decides, and else "dotnet".
    /// </summary>
    public OctopusLiveEngine(string? apiKey = null, IAsyncActivityDispatcher? asyncDispatcher = null, string? engine = null)
    {
        BlueprintRepo = new YamlBlueprintRepository();
        ActivityExecutor = new CallableActivityExecutor();
        StateStore = new InMemoryStateStore();
        AsyncDispatcher = asyncDispatcher ?? new InMemoryAsyncActivityDispatcher();
        ApiKey = apiKey;

        var choice = EngineChoice.Resolve(engine);
        if (choice != "dotnet")
        {
            string? reason = null;
            if (asyncDispatcher != null) reason = "a custom async dispatcher runs on the .NET engine only";
            else if (!CoreEngine.IsAvailable(out var why)) reason = why;
            if (reason == null)
            {
                _core = new CorePipelineManager(BlueprintRepo, ActivityExecutor);
                StateStore = new CoreStateStore(_core);
            }
            else if (choice == "core")
            {
                throw new InvalidOperationException($"engine \"core\" was asked for, but {reason}");
            }
        }

        PipelineManager = new PipelineManager(BlueprintRepo, ActivityExecutor, StateStore, AsyncDispatcher);
        VisualizerService = new VisualizerService(BlueprintRepo, StateStore);
    }

    /// <summary>"core" or "dotnet": the engine that runs this instance's workflows.</summary>
    public string EngineName => _core != null ? "core" : "dotnet";

    /// <summary>What runs the workflows: the core's pipeline, or <see cref="PipelineManager"/>.</summary>
    public IEndpointWorkflowUseCase Workflows => (IEndpointWorkflowUseCase?)_core ?? PipelineManager;

    private void RefuseOnCore(string what)
    {
        if (_core != null)
        {
            throw new NotSupportedException(
                $"{what} is not supported on the core yet; create the engine with engine: \"dotnet\", or configure the store with a blueprint's persistence: block");
        }
    }

    public void SetStateStore(IStateStore store)
    {
        RefuseOnCore("A custom state store");
        StateStore = store ?? new InMemoryStateStore();
        PipelineManager = new PipelineManager(BlueprintRepo, ActivityExecutor, StateStore, AsyncDispatcher);
        VisualizerService = new VisualizerService(BlueprintRepo, StateStore);
    }

    public void EnableEmbeddedWorker(bool enabled = true)
    {
        _embeddedWorkerEnabled = enabled;
    }

    public void SetAsyncDispatcher(IAsyncActivityDispatcher dispatcher)
    {
        RefuseOnCore("A custom async dispatcher");
        AsyncDispatcher = dispatcher ?? new InMemoryAsyncActivityDispatcher();
        PipelineManager = new PipelineManager(BlueprintRepo, ActivityExecutor, StateStore, AsyncDispatcher);
    }

    public void RegisterCustomMiddleware(string name, CustomMiddlewareFactory factory)
    {
        CustomMiddlewares[name] = factory;
    }

    public MiddlewarePipeline BuildMiddlewarePipeline(WorkflowBlueprint blueprint)
    {
        var pipeline = new MiddlewarePipeline();
        if (blueprint.Middlewares != null)
        {
            foreach (var mwCfg in blueprint.Middlewares)
            {
                var mw = MiddlewareFactory.BuildMiddlewareFromConfig(mwCfg, ActivityExecutor, CustomMiddlewares, AsyncDispatcher);
                if (mw != null)
                {
                    pipeline.AddMiddleware(mw);
                }
            }
        }
        return pipeline;
    }

    public void RegisterModel<T>(string name)
    {
        _models[name] = typeof(T);
    }

    public Type? GetModel(string name) => _models.TryGetValue(name, out var t) ? t : null;

    public IReadOnlyDictionary<string, Type> GetModelRegistry() => _models;

    public void RegisterActivity(string name, ActivityHandlerFunc handler)
    {
        ActivityExecutor.RegisterActivity(name, handler);
    }

    public void RegisterActivity(string name, Func<Dictionary<string, object>, Dictionary<string, object>> syncHandler)
    {
        ActivityExecutor.RegisterActivity(name, (input, _) => Task.FromResult(syncHandler(input)));
    }

    public async Task RegisterBlueprintAsync(WorkflowBlueprint blueprint, CancellationToken cancellationToken = default)
    {
        ConfigurePersistenceIfPresent(blueprint);
        await BlueprintRepo.SaveBlueprintAsync(blueprint, cancellationToken);
        _core?.SyncBlueprint(blueprint);
    }

    public void LoadBlueprintsFromDirectory(string directoryPath)
    {
        BlueprintRepo.LoadFromDirectory(directoryPath);
        var bps = BlueprintRepo.ListBlueprintsAsync().GetAwaiter().GetResult();
        foreach (var bp in bps)
        {
            ConfigurePersistenceIfPresent(bp);
        }
        _core?.SyncAll();
    }

    public void LoadBlueprintFile(string filePath)
    {
        BlueprintRepo.LoadFromFile(filePath);
        var bps = BlueprintRepo.ListBlueprintsAsync().GetAwaiter().GetResult();
        foreach (var bp in bps)
        {
            ConfigurePersistenceIfPresent(bp);
        }
        _core?.SyncAll();
    }

    private void ConfigurePersistenceIfPresent(WorkflowBlueprint? blueprint)
    {
        // On the core, the blueprint's persistence: block configures the core's own store.
        if (_core != null) return;
        if (blueprint == null || blueprint.Persistence == null) return;

        var persDict = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        foreach (var kvp in blueprint.Persistence)
        {
            persDict[kvp.Key] = kvp.Value;
        }

        string? storeType = null;
        if (persDict.TryGetValue("store", out var s) && s != null)
        {
            storeType = s.ToString();
        }

        if (!string.IsNullOrWhiteSpace(storeType) && !string.Equals(storeType, "inmemory", StringComparison.OrdinalIgnoreCase))
        {
            var store = StoreFactory.Create(storeType, persDict);
            if (store != null)
            {
                SetStateStore(store);
            }
        }

        ConfigureTemporalIfPresent(blueprint);
    }

    public async Task StartWorkerAsync(IActivityExecutor? executor = null, CancellationToken cancellationToken = default)
    {
        if (AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher)
        {
            await temporalDispatcher.StartWorkerAsync(executor ?? ActivityExecutor, cancellationToken);
        }
    }

    public async Task StopWorkerAsync()
    {
        if (AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher)
        {
            await temporalDispatcher.StopWorkerAsync();
        }
    }

    private void ConfigureTemporalIfPresent(WorkflowBlueprint? blueprint)
    {
        if (blueprint == null || blueprint.Persistence == null) return;

        object? tempRaw = null;
        if (blueprint.Persistence.TryGetValue("temporal", out var tr))
        {
            tempRaw = tr;
        }
        else
        {
            foreach (var kvp in blueprint.Persistence)
            {
                if (string.Equals(kvp.Key, "temporal", StringComparison.OrdinalIgnoreCase))
                {
                    tempRaw = kvp.Value;
                    break;
                }
            }
        }

        if (tempRaw != null)
        {
            var tempMap = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            if (tempRaw is System.Collections.IDictionary dict)
            {
                foreach (System.Collections.DictionaryEntry entry in dict)
                {
                    if (entry.Key != null)
                    {
                        tempMap[entry.Key.ToString()!] = entry.Value!;
                    }
                }
            }
            else if (tempRaw is Dictionary<string, object> strDict)
            {
                foreach (var kvp in strDict) tempMap[kvp.Key] = kvp.Value;
            }

            var host = tempMap.TryGetValue("host", out var h) ? h?.ToString() ?? "127.0.0.1:7233" : "127.0.0.1:7233";
            var ns = tempMap.TryGetValue("namespace", out var n) ? n?.ToString() ?? "default" : "default";
            var tq = tempMap.TryGetValue("task_queue", out var q) ? q?.ToString() ?? "octopus-temporal-queue" : "octopus-temporal-queue";

            if (AsyncDispatcher == null || AsyncDispatcher.GetDispatcherType() == "IN_MEMORY")
            {
                var temporalDispatcher = new TemporalAsyncActivityDispatcher(host, ns, tq);
                SetAsyncDispatcher(temporalDispatcher);
                if (_embeddedWorkerEnabled)
                {
                    _ = temporalDispatcher.StartWorkerAsync(ActivityExecutor);
                }
            }
            else if (AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher && _embeddedWorkerEnabled && !temporalDispatcher.IsWorkerStarted)
            {
                _ = temporalDispatcher.StartWorkerAsync(ActivityExecutor);
            }
        }
    }

    public Task<ExecutionState> ExecuteSyncAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default)
    {
        return Workflows.ExecuteSyncAsync(workflowName, payload, cancellationToken);
    }

    public Task<ExecutionState> ExecuteEndpointSyncAsync(
        string workflowName,
        ApiEndpointConfig? endpoint,
        string? route,
        string? method,
        Dictionary<string, object> payload,
        CancellationToken cancellationToken = default
    )
    {
        return Workflows.ExecuteEndpointSyncAsync(workflowName, endpoint, route, method, payload, cancellationToken);
    }

    public Task<ExecutionState> InitiateAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default)
    {
        return Workflows.InitiateAsync(workflowName, payload, cancellationToken);
    }

    public void MountRoutes(IEndpointRouteBuilder endpoints)
    {
        if (_embeddedWorkerEnabled && AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher && !temporalDispatcher.IsWorkerStarted)
        {
            _ = temporalDispatcher.StartWorkerAsync(ActivityExecutor);
        }
        endpoints.MapOctopusLiveEndpoints(Workflows, VisualizerService, ApiKey, this);
    }

    public void Dispose()
    {
        StopWorkerAsync().GetAwaiter().GetResult();
        _core?.Dispose();
    }

    public async ValueTask DisposeAsync()
    {
        await StopWorkerAsync();
        _core?.Dispose();
    }
}
