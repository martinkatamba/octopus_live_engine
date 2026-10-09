using Octopus.LiveEngine.Domain.Models;

namespace Octopus.LiveEngine.Domain.Ports;

public interface ILiveWorkflowUseCase
{
    Task<ExecutionState> ExecuteSyncAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default);
    Task<ExecutionState> InitiateAsync(string workflowName, Dictionary<string, object> payload, CancellationToken cancellationToken = default);
    IAsyncEnumerable<LiveEvent> StreamEventsAsync(string jobId, CancellationToken cancellationToken = default);
    Task<ExecutionState?> GetExecutionStateAsync(string jobId, CancellationToken cancellationToken = default);
    Task<IReadOnlyList<ExecutionState>> ListExecutionStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default);
}

public interface IVisualizerUseCase
{
    Task<Dictionary<string, object>> CompileX6DiagramAsync(
        string workflowName,
        string? jobId,
        string? route = null,
        string? method = null,
        CancellationToken cancellationToken = default
    );
    Task<IReadOnlyList<WorkflowBlueprint>> ListBlueprintsAsync(CancellationToken cancellationToken = default);
}

public delegate Task<Dictionary<string, object>> ActivityHandlerFunc(Dictionary<string, object> input, CancellationToken cancellationToken);

public interface IActivityExecutor
{
    Task<Dictionary<string, object>> ExecuteActivityAsync(string activityName, Dictionary<string, object> input, CancellationToken cancellationToken = default);
    void RegisterActivity(string activityName, ActivityHandlerFunc handler);
    bool HasActivity(string activityName);
}

public interface IBlueprintRepository
{
    Task<WorkflowBlueprint?> GetBlueprintAsync(string workflowName, CancellationToken cancellationToken = default);
    Task<IReadOnlyList<WorkflowBlueprint>> ListBlueprintsAsync(CancellationToken cancellationToken = default);
    Task SaveBlueprintAsync(WorkflowBlueprint blueprint, CancellationToken cancellationToken = default);
}

public interface IStateStore
{
    Task SaveStateAsync(ExecutionState state, CancellationToken cancellationToken = default);
    Task<ExecutionState?> GetStateAsync(string jobId, CancellationToken cancellationToken = default);
    Task<IReadOnlyList<ExecutionState>> ListStatesAsync(string? workflowName = null, int limit = 100, CancellationToken cancellationToken = default);
    Task PublishEventAsync(string jobId, LiveEvent liveEvent, CancellationToken cancellationToken = default);
    IAsyncEnumerable<LiveEvent> SubscribeEventsAsync(string jobId, CancellationToken cancellationToken = default);
}

public interface IAsyncActivityDispatcher
{
    void Dispatch(string activityName, Dictionary<string, object> payload, string jobId, string stepId, IActivityExecutor localExecutor);
    string GetDispatcherType();
}

