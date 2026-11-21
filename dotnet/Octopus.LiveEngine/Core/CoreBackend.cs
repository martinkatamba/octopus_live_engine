using System.Runtime.CompilerServices;
using System.Threading.Channels;
using Octopus.LiveEngine.Adapters.Outbound;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using YamlDotNet.Serialization;
using YamlDotNet.Serialization.NamingConventions;

namespace Octopus.LiveEngine.Core;

/// <summary>Which engine runs the workflows: the core (octopus-ffi) or the .NET one.</summary>
public static class EngineChoice
{
    /// <summary>Sets the default when the constructor is given none: "auto", "core" or "dotnet".</summary>
    public const string EnvVar = "OCTOPUS_LIVE_ENGINE";

    /// <summary>"auto", "core" or "dotnet": <paramref name="engine"/>, else <c>OCTOPUS_LIVE_ENGINE</c>, else "dotnet".</summary>
    public static string Resolve(string? engine)
    {
        var choice = (engine ?? Environment.GetEnvironmentVariable(EnvVar) ?? "dotnet").Trim().ToLowerInvariant();
        if (choice is not ("auto" or "core" or "dotnet"))
        {
            throw new ArgumentException($"engine must be 'auto', 'core' or 'dotnet', not '{choice}'", nameof(engine));
        }
        return choice;
    }
}

/// <summary><see cref="PipelineManager"/>'s way of running a declared <c>api:</c> endpoint.</summary>
public interface IEndpointWorkflowUseCase : ILiveWorkflowUseCase
{
    Task<ExecutionState> ExecuteEndpointSyncAsync(
        string workflowName, ApiEndpointConfig? endpoint, string? route, string? method,
        Dictionary<string, object> payload, CancellationToken cancellationToken = default);
}

/// <summary>The workflows run on the core; the counterpart of <see cref="Domain.Services.PipelineManager"/>.</summary>
public sealed class CorePipelineManager : IEndpointWorkflowUseCase, IDisposable
{
    private static readonly string[] FinalEvents = { "workflow_completed", "workflow_failed", "workflow_rolled_back" };

    // No anchors: the core's YAML reader has none, and the repository shares one step
    // list between `steps` and the first endpoint, which YamlDotNet would alias.
    private static readonly Lazy<ISerializer> Yaml = new(() => new SerializerBuilder()
        .DisableAliases()
        .WithNamingConvention(UnderscoredNamingConvention.Instance)
        .ConfigureDefaultValuesHandling(DefaultValuesHandling.OmitNull | DefaultValuesHandling.OmitEmptyCollections)
        .Build());

    private readonly YamlBlueprintRepository _blueprints;
    private readonly Dictionary<string, WorkflowBlueprint> _registered = new();
    private readonly object _lock = new();

    internal CoreEngine Core { get; }

    internal CorePipelineManager(YamlBlueprintRepository blueprints, IActivityExecutor activities)
    {
        _blueprints = blueprints;
        // The routes choose job IDs (X-Job-ID, the payload's job_id, or a new one), as on the .NET engine.
        Core = new CoreEngine(activities, new Dictionary<string, object>
        {
            ["options"] = new Dictionary<string, object> { ["allow_client_job_ids"] = true },
        });
    }

    public void Dispose() => Core.Dispose();

    /// <summary>
    /// Registers <paramref name="bp"/> with the core unless this same object already is:
    /// its YAML as written when it came from a file, else the model written out as YAML.
    /// </summary>
    public void SyncBlueprint(WorkflowBlueprint bp)
    {
        lock (_lock)
        {
            if (_registered.TryGetValue(bp.WorkflowName, out var known) && ReferenceEquals(known, bp)) return;
            var text = _blueprints.GetSource(bp) ?? Yaml.Value.Serialize(bp);
            Core.RegisterBlueprint(text);
            _registered[bp.WorkflowName] = bp;
        }
    }

    /// <summary>Registers every blueprint the repository holds.</summary>
    public void SyncAll()
    {
        foreach (var bp in _blueprints.ListBlueprintsAsync().GetAwaiter().GetResult()) SyncBlueprint(bp);
    }

    private async Task<WorkflowBlueprint> Blueprint(string workflowName, CancellationToken ct)
    {
        var bp = await _blueprints.GetBlueprintAsync(workflowName, ct)
            ?? throw new KeyNotFoundException($"Workflow blueprint '{workflowName}' not found.");
        SyncBlueprint(bp);
        return bp;
    }

    public Task<ExecutionState> ExecuteSyncAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default) =>
        ExecuteEndpointSyncAsync(workflowName, null, null, null, payload, cancellationToken);

    public async Task<ExecutionState> ExecuteEndpointSyncAsync(
        string workflowName, ApiEndpointConfig? endpoint, string? route, string? method,
        Dictionary<string, object> payload, CancellationToken cancellationToken = default)
    {
        var bp = await Blueprint(workflowName, cancellationToken);
        var index = endpoint == null ? -1 : bp.Api.FindIndex(ep => ReferenceEquals(ep, endpoint));
        if (endpoint != null && index < 0)
        {
            index = bp.Api.FindIndex(ep =>
                string.Equals(ep.Route, endpoint.Route, StringComparison.OrdinalIgnoreCase)
                && string.Equals(ep.Method, endpoint.Method, StringComparison.OrdinalIgnoreCase));
        }
        var reqRoute = route ?? endpoint?.Route;
        var reqMethod = method ?? endpoint?.Method;
        var input = new Dictionary<string, object>(payload);
        // Off the caller's thread: the run blocks, and activities may need a free thread
        // (an xunit or UI synchronization context would otherwise deadlock).
        return await Task.Run(() => Core.Execute(workflowName, input, index, reqRoute, reqMethod), CancellationToken.None);
    }

    public async Task<ExecutionState> InitiateAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default)
    {
        await Blueprint(workflowName, cancellationToken);
        var input = new Dictionary<string, object>(payload);
        return await Task.Run(() => Core.InitiateAsync(workflowName, input), CancellationToken.None);
    }

    public Task<ExecutionState?> GetExecutionStateAsync(string jobId, CancellationToken cancellationToken = default) =>
        Task.Run(() => Core.GetState(jobId), CancellationToken.None);

    public Task<IReadOnlyList<ExecutionState>> ListExecutionStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default) =>
        Task.Run<IReadOnlyList<ExecutionState>>(() => Core.ListStates(workflowName, limit), CancellationToken.None);

    /// <summary>The job's live events, ending with its final one. A job that already finished yields that event at once.</summary>
    public async IAsyncEnumerable<LiveEvent> StreamEventsAsync(string jobId, [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        var channel = Channel.CreateUnbounded<LiveEvent>(new UnboundedChannelOptions { SingleReader = true });
        using var sub = Core.Subscribe(jobId, ev => channel.Writer.TryWrite(ev));
        var current = Core.GetState(jobId);
        if (current != null && current.Status is ExecutionStatus.COMPLETED or ExecutionStatus.FAILED or ExecutionStatus.ROLLED_BACK)
        {
            var name = current.Status switch
            {
                ExecutionStatus.COMPLETED => "workflow_completed",
                ExecutionStatus.FAILED => "workflow_failed",
                _ => "workflow_rolled_back",
            };
            // The state as the core's events carry it (snake_case), not the .NET model.
            yield return new LiveEvent(name, CoreJson.ParseObject(CoreJson.WriteState(current)));
            yield break;
        }
        await foreach (var ev in channel.Reader.ReadAllAsync(cancellationToken))
        {
            yield return ev;
            if (FinalEvents.Contains(ev.Event)) yield break;
        }
    }
}

/// <summary>The core's jobs as an <see cref="IStateStore"/>. The core keeps them in its own store (memory, or a blueprint's <c>persistence:</c>).</summary>
public sealed class CoreStateStore : IStateStore
{
    private readonly CorePipelineManager _pipeline;

    public CoreStateStore(CorePipelineManager pipeline) => _pipeline = pipeline;

    public Task SaveStateAsync(ExecutionState state, CancellationToken cancellationToken = default) =>
        Task.Run(() => _pipeline.Core.SaveState(state), CancellationToken.None);

    public Task<ExecutionState?> GetStateAsync(string jobId, CancellationToken cancellationToken = default) =>
        _pipeline.GetExecutionStateAsync(jobId, cancellationToken);

    public Task<IReadOnlyList<ExecutionState>> ListStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default) =>
        _pipeline.ListExecutionStatesAsync(workflowName, limit, cancellationToken);

    /// <summary>Live events come from the core's runs; there is nothing to publish from here.</summary>
    public Task PublishEventAsync(string jobId, LiveEvent liveEvent, CancellationToken cancellationToken = default) => Task.CompletedTask;

    public IAsyncEnumerable<LiveEvent> SubscribeEventsAsync(string jobId, CancellationToken cancellationToken = default) =>
        _pipeline.StreamEventsAsync(jobId, cancellationToken);
}
