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
    private string? _apiKey;

    /// <summary>
    /// The key the engine's routes require (X-API-Key or ?api_key=); none means they are open.
    /// On the core, set it before the engine starts (running a workflow or mounting the routes),
    /// or in the constructor.
    /// </summary>
    public string? ApiKey
    {
        get => _apiKey;
        set
        {
            if (_core != null && value != _core.ApiKey)
            {
                if (_started)
                {
                    throw new NotSupportedException(
                        "On the core, the API key can't change once the engine has started; set it in the constructor: new OctopusLiveEngine(apiKey)");
                }
                // The core takes its key when it is made: make a new one.
                _core.Dispose();
                StartCore(value);
                _core.SyncAll();
                Rewire();
            }
            _apiKey = value;
        }
    }

    /// <summary>How the routes are served (tooling access, trusted proxies). On the core, set them in the constructor.</summary>
    public EngineOptions Options { get; }
    private readonly Dictionary<string, Type> _models = new();
    public Dictionary<string, CustomMiddlewareFactory> CustomMiddlewares { get; } = new(StringComparer.OrdinalIgnoreCase);
    private bool _embeddedWorkerEnabled = true;
    private CorePipelineManager? _core;

    // "auto", "core" or "dotnet", as resolved by the constructor.
    private readonly string _choice;

    // Set once a workflow runs, the routes are mounted or the visualizer draws: from
    // then on the engine can't change, nor can the core's API key.
    private bool _started;

    /// <summary>
    /// <paramref name="engine"/> picks what runs the workflows: "auto" (the default:
    /// the shared Rust core, octopus-ffi, when its library loads, else this SDK's own
    /// engine), "core" (the core, or an error) or "dotnet" (this SDK's own engine).
    /// Without it, the <c>OCTOPUS_LIVE_ENGINE</c> environment variable decides.
    /// On "auto", what only the .NET engine can do (a custom state store or async
    /// dispatcher) moves this instance to the .NET engine, if it hasn't started yet.
    /// <paramref name="options"/> sets how the routes are served; by default the
    /// tooling routes are for this machine only.
    /// </summary>
    public OctopusLiveEngine(string? apiKey = null, IAsyncActivityDispatcher? asyncDispatcher = null, string? engine = null, EngineOptions? options = null)
    {
        Options = options ?? new EngineOptions();
        BlueprintRepo = new YamlBlueprintRepository();
        ActivityExecutor = new CallableActivityExecutor();
        StateStore = new InMemoryStateStore();
        AsyncDispatcher = asyncDispatcher ?? new InMemoryAsyncActivityDispatcher();
        _apiKey = apiKey;

        _choice = EngineChoice.Resolve(engine);
        if (_choice != "dotnet")
        {
            string? reason = null;
            if (asyncDispatcher != null) reason = "a custom async dispatcher runs on the .NET engine only";
            else if (!CoreEngine.IsAvailable(out var why)) reason = why;
            if (reason == null)
            {
                StartCore(apiKey);
            }
            else if (_choice == "core")
            {
                throw new InvalidOperationException($"engine \"core\" was asked for, but {reason}");
            }
        }

        PipelineManager = new PipelineManager(BlueprintRepo, ActivityExecutor, StateStore, AsyncDispatcher);
        VisualizerService = new VisualizerService(BlueprintRepo, StateStore);
    }

    [System.Diagnostics.CodeAnalysis.MemberNotNull(nameof(_core))]
    private void StartCore(string? apiKey)
    {
        _core = new CorePipelineManager(BlueprintRepo, ActivityExecutor, apiKey, Options);
        StateStore = new CoreStateStore(_core);
    }

    private void Rewire()
    {
        PipelineManager = new PipelineManager(BlueprintRepo, ActivityExecutor, StateStore, AsyncDispatcher);
        VisualizerService = new VisualizerService(BlueprintRepo, StateStore);
    }

    /// <summary>"core" or "dotnet": the engine that runs this instance's workflows.</summary>
    public string EngineName => _core != null ? "core" : "dotnet";

    /// <summary>What runs the workflows: the core's pipeline, or <see cref="PipelineManager"/>.</summary>
    public IEndpointWorkflowUseCase Workflows
    {
        get
        {
            _started = true;
            return (IEndpointWorkflowUseCase?)_core ?? PipelineManager;
        }
    }

    /// <summary>
    /// For what only the .NET engine can do: on "auto", before the engine has started,
    /// this instance moves to the .NET engine; else the core refuses it.
    /// </summary>
    private void LeaveCore(string what)
    {
        if (_core == null) return;
        if (_choice != "auto")
        {
            throw new NotSupportedException(
                $"{what} is not supported on the core yet; create the engine with engine: \"dotnet\", or configure the store with a blueprint's persistence: block");
        }
        if (_started)
        {
            throw new NotSupportedException(
                $"{what} runs on the .NET engine only, and this engine has already started on the core; set it before running a workflow or mounting the routes, or create the engine with engine: \"dotnet\"");
        }

        Console.Error.WriteLine($"octopus: {what} runs on the .NET engine only; this engine (\"auto\") now uses the .NET engine instead of the core");
        _core.Dispose();
        _core = null;
        StateStore = new InMemoryStateStore();
        Rewire();
        // On the core the blueprints' persistence: blocks went to the core; give them to .NET.
        foreach (var bp in BlueprintRepo.ListBlueprintsAsync().GetAwaiter().GetResult())
        {
            ConfigurePersistenceIfPresent(bp);
        }
    }

    public void SetStateStore(IStateStore store)
    {
        LeaveCore("A custom state store");
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
        LeaveCore("A custom async dispatcher");
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
            for (var i = 0; i < blueprint.Middlewares.Count; i++)
            {
                var mwCfg = blueprint.Middlewares[i];
                // On the core, a built-in is the core's (D11); a middleware registered
                // in code, or one the core lacks, runs here.
                var mw = _core != null && !CustomMiddlewares.ContainsKey((mwCfg.Name ?? "").Trim())
                    ? _core.BuiltInMiddleware(blueprint, i)
                    : null;
                mw ??= MiddlewareFactory.BuildMiddlewareFromConfig(mwCfg, ActivityExecutor, CustomMiddlewares, AsyncDispatcher);
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

    /// <summary>
    /// Starts the Temporal worker for the blueprints' <c>mode: hybrid</c> async steps. On the
    /// core, the core's own worker (D13), when a blueprint configures Temporal and the core
    /// was built with it; it runs this engine's activities, so <paramref name="executor"/> is
    /// for the .NET engine only.
    /// </summary>
    public async Task StartWorkerAsync(IActivityExecutor? executor = null, CancellationToken cancellationToken = default)
    {
        if (_core is { } core)
        {
            if (executor != null && executor != ActivityExecutor)
            {
                throw new NotSupportedException("On the core, the worker runs the engine's own activities; register them with RegisterActivity");
            }
            // The core starts a worker only for a blueprint that uses Temporal: give it the blueprints first.
            _started = true;
            core.SyncAll();
            await Task.Run(core.Core.StartWorker, cancellationToken);
            return;
        }
        if (AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher)
        {
            await temporalDispatcher.StartWorkerAsync(executor ?? ActivityExecutor, cancellationToken);
        }
    }

    public async Task StopWorkerAsync()
    {
        if (_core is { } core)
        {
            core.Core.StopWorker();
            return;
        }
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

    /// <summary>
    /// The visualizer's graph of a workflow, as <c>GET .../diagram/x6</c> answers it, on
    /// whichever engine runs this instance; null when the core has no such workflow.
    /// With no job, route or method named, it draws the first declared route that has steps.
    /// </summary>
    public async Task<Dictionary<string, object>?> CompileX6DiagramAsync(
        string workflowName,
        string? jobId = null,
        string? route = null,
        string? method = null,
        CancellationToken cancellationToken = default
    )
    {
        _started = true;
        if (_core != null)
        {
            var graph = await _core.DiagramAsync(workflowName, jobId, route, method);
            return graph == null ? null : CoreJson.ParseObject(graph);
        }
        return await VisualizerService.CompileX6DiagramAsync(workflowName, jobId, route, method, cancellationToken);
    }

    public void MountRoutes(IEndpointRouteBuilder endpoints)
    {
        if (_core != null)
        {
            StartEmbeddedCoreWorker();
        }
        else if (_embeddedWorkerEnabled && AsyncDispatcher is TemporalAsyncActivityDispatcher temporalDispatcher && !temporalDispatcher.IsWorkerStarted)
        {
            _ = temporalDispatcher.StartWorkerAsync(ActivityExecutor);
        }
        endpoints.MapOctopusLiveEndpoints(Workflows, VisualizerService, ApiKey, this);
    }

    /// <summary>
    /// On the core, when the routes are mounted: the core's embedded Temporal worker, as the
    /// .NET engine starts its own when a blueprint configures Temporal. The core starts one
    /// only for a blueprint that uses Temporal, and a Temporal that can't be reached leaves
    /// the async steps running in-process (as in the Python SDK), so it doesn't stop the routes.
    /// </summary>
    internal void StartEmbeddedCoreWorker()
    {
        if (!_embeddedWorkerEnabled || _core is not { } core) return;
        try
        {
            core.SyncAll();
            core.Core.StartWorker();
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"octopus: Temporal worker not started: {ex.Message}");
        }
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
