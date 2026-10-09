using System.Collections.Concurrent;
using Octopus.LiveEngine.Domain.Ports;
using Temporalio.Activities;
using Temporalio.Client;
using Temporalio.Common;
using Temporalio.Worker;
using Temporalio.Workflows;

namespace Octopus.LiveEngine.Adapters.Outbound;

public class InMemoryAsyncActivityDispatcher : IAsyncActivityDispatcher
{
    public void Dispatch(string activityName, Dictionary<string, object> payload, string jobId, string stepId, IActivityExecutor localExecutor)
    {
        if (string.IsNullOrWhiteSpace(activityName) || localExecutor == null) return;

        var payloadCopy = new Dictionary<string, object>(payload);
        Task.Run(async () =>
        {
            try
            {
                await localExecutor.ExecuteActivityAsync(activityName, payloadCopy);
            }
            catch (Exception ex)
            {
                Console.WriteLine($"⚡ [InMemoryAsyncDispatcher] Background activity '{activityName}' (job={jobId}, step={stepId}) failed: {ex.Message}");
            }
        });
    }

    public string GetDispatcherType() => "IN_MEMORY";
}

[Workflow("OctopusWorkflow")]
public class OctopusWorkflow
{
    [WorkflowRun]
    public async Task<Dictionary<string, object>> RunAsync(Dictionary<string, object> payload)
    {
        var activityName = payload.TryGetValue("activity_name", out var aName) && aName != null
            ? aName.ToString()!
            : "DefaultActivity";

        var options = new ActivityOptions
        {
            StartToCloseTimeout = TimeSpan.FromMinutes(5),
            RetryPolicy = new RetryPolicy
            {
                InitialInterval = TimeSpan.FromMilliseconds(500),
                BackoffCoefficient = 2.0f,
                MaximumAttempts = 5
            }
        };

        return await Workflow.ExecuteActivityAsync(
            (OctopusActivities act) => act.ExecuteActivityAsync(activityName, payload),
            options
        );
    }
}

public class OctopusActivities
{
    private readonly IActivityExecutor _localExecutor;

    public OctopusActivities(IActivityExecutor localExecutor)
    {
        _localExecutor = localExecutor;
    }

    [Activity("ExecuteActivity")]
    public async Task<Dictionary<string, object>> ExecuteActivityAsync(string activityName, Dictionary<string, object> payload)
    {
        if (_localExecutor != null && _localExecutor.HasActivity(activityName))
        {
            var unwrappedPayload = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
            if (payload != null)
            {
                foreach (var (k, v) in payload)
                {
                    unwrappedPayload[k] = UnwrapJson(v)!;
                }
            }
            var result = await _localExecutor.ExecuteActivityAsync(activityName, unwrappedPayload);
            return result ?? new Dictionary<string, object>();
        }
        return new Dictionary<string, object> { ["status"] = "SUCCESS" };
    }

    private static object? UnwrapJson(object? obj)
    {
        if (obj is System.Text.Json.JsonElement elem)
        {
            switch (elem.ValueKind)
            {
                case System.Text.Json.JsonValueKind.String: return elem.GetString();
                case System.Text.Json.JsonValueKind.Number:
                    if (elem.TryGetInt64(out var l)) return l;
                    if (elem.TryGetDouble(out var d)) return d;
                    return elem.GetRawText();
                case System.Text.Json.JsonValueKind.True: return true;
                case System.Text.Json.JsonValueKind.False: return false;
                case System.Text.Json.JsonValueKind.Null: return null;
                case System.Text.Json.JsonValueKind.Array:
                    var list = new List<object>();
                    foreach (var item in elem.EnumerateArray()) list.Add(UnwrapJson(item)!);
                    return list;
                case System.Text.Json.JsonValueKind.Object:
                    var dict = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
                    foreach (var p in elem.EnumerateObject()) dict[p.Name] = UnwrapJson(p.Value)!;
                    return dict;
            }
        }
        return obj;
    }
}

public class TemporalAsyncActivityDispatcher : IAsyncActivityDispatcher, IAsyncDisposable, IDisposable
{
    public string Host { get; }
    public string Namespace { get; }
    public string TaskQueue { get; }
    public int MaxRetries { get; }
    public int InitialBackoffMs { get; }
    public double BackoffCoeff { get; }

    private readonly ConcurrentBag<Dictionary<string, object>> _tasksHistory = new();
    private ITemporalClient? _client;
    private TemporalWorker? _worker;
    private Task? _workerTask;
    private CancellationTokenSource? _workerCts;
    private bool _workerStarted = false;
    private readonly SemaphoreSlim _initLock = new(1, 1);

    public TemporalAsyncActivityDispatcher(
        string host = "127.0.0.1:7233",
        string ns = "default",
        string taskQueue = "octopus-temporal-queue",
        int maxRetries = 5,
        int initialBackoffMs = 500,
        double backoffCoeff = 2.0
    )
    {
        Host = string.IsNullOrWhiteSpace(host) ? "127.0.0.1:7233" : host;
        Namespace = string.IsNullOrWhiteSpace(ns) ? "default" : ns;
        TaskQueue = string.IsNullOrWhiteSpace(taskQueue) ? "octopus-temporal-queue" : taskQueue;
        MaxRetries = maxRetries;
        InitialBackoffMs = initialBackoffMs;
        BackoffCoeff = backoffCoeff;
    }

    private async Task<ITemporalClient?> EnsureClientInternalAsync()
    {
        if (_client != null) return _client;
        try
        {
            var targetHost = Host;
            if (!targetHost.Contains(':'))
            {
                targetHost = $"{targetHost}:7233";
            }
            _client = await TemporalClient.ConnectAsync(new TemporalClientConnectOptions(targetHost)
            {
                Namespace = Namespace
            });
            Console.WriteLine($"🛡️ Temporal .NET SDK Client connected -> Host: {targetHost}, Namespace: {Namespace}, TaskQueue: {TaskQueue}");
            return _client;
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ Could not connect Temporal .NET SDK client to {Host}: {ex.Message}");
            return null;
        }
    }

    public async Task<ITemporalClient?> GetClientAsync()
    {
        if (_client != null) return _client;
        await _initLock.WaitAsync();
        try
        {
            return await EnsureClientInternalAsync();
        }
        finally
        {
            _initLock.Release();
        }
    }

    public async Task StartWorkerAsync(IActivityExecutor activityExecutor, CancellationToken cancellationToken = default)
    {
        if (_workerStarted) return;
        await _initLock.WaitAsync(cancellationToken);
        try
        {
            if (_workerStarted) return;
            var client = await EnsureClientInternalAsync();
            if (client == null) return;

            var workerOptions = new TemporalWorkerOptions(taskQueue: TaskQueue);
            workerOptions.AddWorkflow<OctopusWorkflow>();
            var activities = new OctopusActivities(activityExecutor);
            workerOptions.AddActivity(activities.ExecuteActivityAsync);

            _worker = new TemporalWorker(client, workerOptions);
            _workerCts = new CancellationTokenSource();
            _workerTask = _worker.ExecuteAsync(_workerCts.Token);
            _workerStarted = true;
            Console.WriteLine($"👷 [TEMPORAL WORKER] Worker actively polling TaskQueue '{TaskQueue}' on Temporal Server ({Host})");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"⚠️ Failed to start Temporal Worker on queue {TaskQueue}: {ex.Message}");
        }
        finally
        {
            _initLock.Release();
        }
    }

    public bool IsWorkerStarted => _workerStarted;

    public async Task StopWorkerAsync()
    {
        if (_workerCts != null && !_workerCts.IsCancellationRequested)
        {
            _workerCts.Cancel();
        }
        if (_workerTask != null)
        {
            try
            {
                await _workerTask;
            }
            catch (OperationCanceledException) { }
            catch (Exception ex)
            {
                Console.WriteLine($"⚠️ Error stopping Temporal worker: {ex.Message}");
            }
        }
        _workerStarted = false;
        _worker?.Dispose();
        _worker = null;
    }

    public void Dispatch(string activityName, Dictionary<string, object> payload, string jobId, string stepId, IActivityExecutor localExecutor)
    {
        if (string.IsNullOrWhiteSpace(activityName)) return;

        var workflowId = $"octopus_act_{jobId}_{stepId}_{activityName}";
        var scheduledAt = DateTime.UtcNow.ToString("O");
        var record = new Dictionary<string, object>
        {
            ["temporal_workflow_id"] = workflowId,
            ["task_queue"] = TaskQueue,
            ["activity_name"] = activityName,
            ["job_id"] = jobId,
            ["step_id"] = stepId,
            ["scheduled_at"] = scheduledAt,
            ["status"] = "SCHEDULED"
        };
        _tasksHistory.Add(record);

        Console.WriteLine($"⏳ [TEMPORAL DISPATCH] Queuing durable activity '{activityName}' -> WorkflowID: {workflowId}");

        var payloadCopy = new Dictionary<string, object>(payload)
        {
            ["activity_name"] = activityName,
            ["temporal_workflow_id"] = workflowId,
            ["temporal_task_queue"] = TaskQueue
        };

        Task.Run(async () =>
        {
            bool dispatchedViaSdk = false;
            try
            {
                var client = await GetClientAsync();
                if (client != null)
                {
                    var handle = await client.StartWorkflowAsync(
                        (OctopusWorkflow wf) => wf.RunAsync(payloadCopy),
                        new WorkflowOptions(workflowId, TaskQueue)
                    );
                    dispatchedViaSdk = true;
                    Console.WriteLine($"🚀 [TEMPORAL SDK] Workflow started successfully: {workflowId} (RunId: {handle.RunId})");
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"⚠️ [TEMPORAL SDK DISPATCH] Failed to dispatch via Temporal SDK ({ex.Message}), falling back to in-memory retry worker...");
            }

            if (!dispatchedViaSdk)
            {
                await ExecuteWithRetryAsync(activityName, payloadCopy, jobId, stepId, localExecutor, record, 0, InitialBackoffMs);
            }
        });
    }

    private async Task ExecuteWithRetryAsync(
        string activityName,
        Dictionary<string, object> payload,
        string jobId,
        string stepId,
        IActivityExecutor localExecutor,
        Dictionary<string, object> record,
        int attempt,
        int delayMs
    )
    {
        if (delayMs > 0)
        {
            await Task.Delay(delayMs);
        }

        try
        {
            if (localExecutor != null)
            {
                var result = await localExecutor.ExecuteActivityAsync(activityName, payload);
                record["status"] = "COMPLETED";
                record["completed_at"] = DateTime.UtcNow.ToString("O");
                record["attempts"] = attempt + 1;
                if (result != null)
                {
                    record["result"] = result;
                }
                Console.WriteLine($"✅ [TEMPORAL WORKER] Activity '{activityName}' completed durably (attempt {attempt + 1})");
            }
        }
        catch (Exception ex)
        {
            int nextAttempt = attempt + 1;
            if (nextAttempt < MaxRetries)
            {
                int nextDelay = (int)(delayMs * BackoffCoeff);
                Console.WriteLine($"⚠️ [TEMPORAL WORKER] Activity '{activityName}' failed on attempt {nextAttempt}: {ex.Message}. Retrying in {nextDelay}ms...");
                _ = ExecuteWithRetryAsync(activityName, payload, jobId, stepId, localExecutor, record, nextAttempt, nextDelay);
            }
            else
            {
                record["status"] = "FAILED";
                record["error"] = ex.Message;
                record["failed_at"] = DateTime.UtcNow.ToString("O");
                Console.WriteLine($"❌ [TEMPORAL WORKER] Activity '{activityName}' exhausted all {MaxRetries} retries: {ex.Message}");
            }
        }
    }

    public string GetDispatcherType() => "TEMPORAL";

    public IReadOnlyList<Dictionary<string, object>> GetDispatchedTasksHistory() => _tasksHistory.ToList();

    public void Dispose()
    {
        StopWorkerAsync().GetAwaiter().GetResult();
    }

    public async ValueTask DisposeAsync()
    {
        await StopWorkerAsync();
    }
}
