using System.Diagnostics;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Domain.Services;

public class PipelineManager : ILiveWorkflowUseCase
{
    private readonly IBlueprintRepository _blueprintRepo;
    private readonly IActivityExecutor _activityExecutor;
    private readonly IStateStore _stateStore;
    private readonly IAsyncActivityDispatcher _asyncDispatcher;
    private readonly ConditionEvaluator _conditionEvaluator;
    private readonly SagaManager _sagaManager;
    private readonly IdempotencyManager _idempotencyManager;

    public PipelineManager(
        IBlueprintRepository blueprintRepo,
        IActivityExecutor activityExecutor,
        IStateStore stateStore,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        _blueprintRepo = blueprintRepo;
        _activityExecutor = activityExecutor;
        _stateStore = stateStore;
        _asyncDispatcher = asyncDispatcher ?? new Adapters.Outbound.InMemoryAsyncActivityDispatcher();
        _conditionEvaluator = new ConditionEvaluator();
        _sagaManager = new SagaManager(activityExecutor);
        _idempotencyManager = new IdempotencyManager();
    }

    public Task<ExecutionState> ExecuteSyncAsync(
        string workflowName,
        Dictionary<string, object> payload,
        CancellationToken cancellationToken = default
    )
    {
        return ExecuteEndpointSyncAsync(workflowName, null, null, null, payload, cancellationToken);
    }

    public async Task<ExecutionState> ExecuteEndpointSyncAsync(
        string workflowName,
        ApiEndpointConfig? endpoint,
        string? route,
        string? method,
        Dictionary<string, object> payload,
        CancellationToken cancellationToken = default
    )
    {
        var blueprint = await _blueprintRepo.GetBlueprintAsync(workflowName, cancellationToken)
            ?? throw new KeyNotFoundException($"Workflow blueprint '{workflowName}' not found.");

        var jobId = payload.TryGetValue("job_id", out var jVal) && jVal != null && !string.IsNullOrEmpty(jVal.ToString())
            ? jVal.ToString()!
            : $"job_{Guid.NewGuid():N}";

        if (_idempotencyManager.TryGet(jobId, out var cachedState) && cachedState != null)
        {
            return cachedState;
        }

        var targetEndpoint = endpoint;
        if (targetEndpoint == null && !string.IsNullOrWhiteSpace(route) && blueprint.Api != null)
        {
            foreach (var ep in blueprint.Api)
            {
                if (string.Equals(route, ep.Route, StringComparison.OrdinalIgnoreCase) ||
                    route.EndsWith(ep.Route, StringComparison.OrdinalIgnoreCase) ||
                    ep.Route.EndsWith(route, StringComparison.OrdinalIgnoreCase))
                {
                    if (string.IsNullOrWhiteSpace(method) || string.IsNullOrWhiteSpace(ep.Method) ||
                        string.Equals(method, ep.Method, StringComparison.OrdinalIgnoreCase))
                    {
                        targetEndpoint = ep;
                        break;
                    }
                }
            }
        }

        var stepsToRun = targetEndpoint != null && targetEndpoint.Steps != null && targetEndpoint.Steps.Count > 0
            ? targetEndpoint.Steps
            : ((blueprint.Steps != null && blueprint.Steps.Count > 0)
                ? blueprint.Steps
                : (blueprint.Api != null && blueprint.Api.Count > 0 && blueprint.Api[0].Steps != null
                    ? blueprint.Api[0].Steps
                    : new List<WorkflowStep>()));

        var reqRoute = route ?? targetEndpoint?.Route;
        var reqMethod = method ?? targetEndpoint?.Method;
        var clientHost = payload.TryGetValue("client_host", out var ch) && ch != null
            ? ch.ToString()
            : (payload.TryGetValue("client_ip", out var cip) && cip != null ? cip.ToString() : "127.0.0.1");

        var workflowSw = Stopwatch.StartNew();
        var state = new ExecutionState
        {
            JobId = jobId,
            WorkflowName = workflowName,
            Status = ExecutionStatus.RUNNING,
            Route = reqRoute,
            Method = reqMethod,
            ClientHost = clientHost,
            TotalSteps = stepsToRun.Count,
            Data = new Dictionary<string, object>(payload, StringComparer.OrdinalIgnoreCase),
            StartedAt = DateTime.UtcNow.ToString("o")
        };
        state.Data["job_id"] = jobId;

        await _stateStore.SaveStateAsync(state, cancellationToken);
        await _stateStore.PublishEventAsync(jobId, new LiveEvent("workflow_started", state), cancellationToken);

        var sagaActions = new List<SagaAction>();
        var steps = new List<WorkflowStep>(stepsToRun);
        int stepIndex = 0;

        while (stepIndex < steps.Count)
        {
            var step = steps[stepIndex];
            state.CurrentIndex = stepIndex;
            state.CurrentStepId = step.Id;

            // Pre-execution delay
            if (step.DelaySeconds > 0)
            {
                await Task.Delay(TimeSpan.FromSeconds(step.DelaySeconds), cancellationToken);
            }

            // Early termination / settlement
            if (step.Terminate || step.Settle)
            {
                var reason = !string.IsNullOrWhiteSpace(step.Reason) ? step.Reason : "Workflow settled gracefully";
                var settleTrace = new StepExecutionTrace
                {
                    StepId = step.Id,
                    ActivityName = "terminate_settle",
                    Status = "COMPLETED",
                    OutputPayload = MaskPayload(blueprint, step, new Dictionary<string, object> { ["settled"] = true, ["reason"] = reason }),
                    StartedAt = DateTime.UtcNow.ToString("o"),
                    FinishedAt = DateTime.UtcNow.ToString("o")
                };
                state.Traces.Add(settleTrace);
                state.Status = ExecutionStatus.COMPLETED;
                break;
            }

            // Declarative compensation trigger
            if (step.Compensate || step.Rollback || step.TriggerCompensation)
            {
                var reason = !string.IsNullOrWhiteSpace(step.Reason) ? step.Reason : $"Declarative compensation triggered at step '{step.Id}'";
                state.Error = reason;
                state.Status = ExecutionStatus.ROLLED_BACK;

                if (string.Equals(blueprint.CompensationMode, "async", StringComparison.OrdinalIgnoreCase))
                {
                    var actionsCopy = new List<SagaAction>(sagaActions);
                    _ = Task.Run(async () =>
                    {
                        await _sagaManager.ExecuteCompensationsAsync(actionsCopy, CancellationToken.None);
                    }, CancellationToken.None);
                }
                else
                {
                    await _sagaManager.ExecuteCompensationsAsync(sagaActions, cancellationToken);
                }

                state.FinishedAt = DateTime.UtcNow.ToString("o");
                await _stateStore.SaveStateAsync(state, cancellationToken);
                await _stateStore.PublishEventAsync(jobId, new LiveEvent("workflow_rolled_back", state), cancellationToken);
                throw new InvalidOperationException(reason);
            }

            var thenSteps = step.GetThenSteps();
            var elseSteps = step.GetElseSteps();
            var hasBranches = thenSteps.Count > 0 || elseSteps.Count > 0 || !string.IsNullOrWhiteSpace(step.BranchingStep);

            // Condition evaluation & branching
            if (hasBranches || step.Condition != null || step.Conditions != null)
            {
                var condResult = _conditionEvaluator.EvaluateStep(step, state.Data);
                if (condResult)
                {
                    if (thenSteps.Count > 0)
                    {
                        steps.InsertRange(stepIndex + 1, thenSteps);
                        state.TotalSteps = steps.Count;
                    }
                }
                else
                {
                    if (elseSteps.Count > 0)
                    {
                        steps.InsertRange(stepIndex + 1, elseSteps);
                        state.TotalSteps = steps.Count;
                    }
                }

                if (string.IsNullOrWhiteSpace(step.Activity) && step.Loop == null && step.ForEach == null)
                {
                    stepIndex++;
                    continue;
                }
            }

            // Loop execution
            if (step.Loop != null || step.ForEach != null)
            {
                await ExecuteLoopStepAsync(step, blueprint, state, cancellationToken);
                stepIndex++;
                continue;
            }

            // Activity Execution (Sync or Async background)
            if (!string.IsNullOrWhiteSpace(step.Activity))
            {
                if (step.Async || step.IsAsync)
                {
                    DispatchAsyncActivity(step, blueprint, state);
                }
                else
                {
                    await _stateStore.PublishEventAsync(jobId, new LiveEvent("step_started", new Dictionary<string, object>
                    {
                        ["step_id"] = step.Id,
                        ["activity"] = step.Activity,
                        ["index"] = stepIndex,
                        ["total_steps"] = state.TotalSteps
                    }), cancellationToken);

                    var sw = Stopwatch.StartNew();
                    var startIso = DateTime.UtcNow.ToString("o");

                    try
                    {
                        var outPayload = await _activityExecutor.ExecuteActivityAsync(step.Activity, state.Data, cancellationToken);
                        sw.Stop();

                        var trace = new StepExecutionTrace
                        {
                            StepId = step.Id,
                            ActivityName = step.Activity,
                            Status = "COMPLETED",
                            InputPayload = MaskPayload(blueprint, step, state.Data),
                            OutputPayload = MaskPayload(blueprint, step, outPayload),
                            StartedAt = startIso,
                            FinishedAt = DateTime.UtcNow.ToString("o"),
                            DurationMs = sw.Elapsed.TotalMilliseconds
                        };

                        state.Traces.Add(trace);

                        foreach (var kvp in outPayload)
                        {
                            state.Data[kvp.Key] = kvp.Value;
                        }

                        if (step.IsOutput || step.Output || state.OutputData == null)
                        {
                            state.OutputData = new Dictionary<string, object>(outPayload);
                        }

                        if (!string.IsNullOrWhiteSpace(step.Compensation))
                        {
                            sagaActions.Add(new SagaAction(step.Id, step.Compensation, new Dictionary<string, object>(state.Data)));
                        }

                        await _stateStore.PublishEventAsync(jobId, new LiveEvent("step_completed", trace), cancellationToken);
                    }
                    catch (Exception ex)
                    {
                        sw.Stop();
                        var failTrace = new StepExecutionTrace
                        {
                            StepId = step.Id,
                            ActivityName = step.Activity,
                            Status = "FAILED",
                            ErrorMessage = ex.Message,
                            InputPayload = MaskPayload(blueprint, step, state.Data),
                            StartedAt = startIso,
                            FinishedAt = DateTime.UtcNow.ToString("o"),
                            DurationMs = sw.Elapsed.TotalMilliseconds
                        };

                        state.Traces.Add(failTrace);
                        state.Error = ex.Message;
                        state.Status = ExecutionStatus.FAILED;

                        var excCfg = FindExceptionConfig(step, blueprint, ex);
                        if (excCfg != null)
                        {
                            var msg = !string.IsNullOrWhiteSpace(excCfg.Message) ? excCfg.Message : ex.Message;
                            var code = excCfg.Code != 0 ? excCfg.Code : 400;
                            state.Error = msg;
                            state.ErrorCode = code;
                            state.ErrorType = excCfg.Exception;

                            if (!string.IsNullOrWhiteSpace(excCfg.Activity) && _activityExecutor != null)
                            {
                                var actPayload = new Dictionary<string, object>
                                {
                                    ["exception"] = excCfg.Exception,
                                    ["error"] = excCfg.Exception,
                                    ["error_type"] = ex.GetType().Name,
                                    ["error_message"] = ex.Message,
                                    ["message"] = msg,
                                    ["status_code"] = code,
                                    ["job_id"] = jobId,
                                    ["workflow_name"] = blueprint.WorkflowName,
                                    ["failing_step_id"] = step.Id,
                                    ["route"] = state.Route ?? string.Empty,
                                    ["path"] = state.Route ?? string.Empty,
                                    ["method"] = state.Method ?? "POST",
                                    ["client_host"] = "127.0.0.1",
                                    ["stack_trace"] = ex.ToString(),
                                    ["traceback"] = ex.ToString(),
                                    ["timestamp"] = DateTime.UtcNow.ToString("o"),
                                    ["data"] = new Dictionary<string, object>(state.Data)
                                };

                                if (excCfg.IsAsync || excCfg.Async)
                                {
                                    var excTrace = new StepExecutionTrace
                                    {
                                        StepId = $"exc_{excCfg.Activity.ToLowerInvariant()}",
                                        ActivityName = excCfg.Activity,
                                        Status = "ASYNC_DISPATCHED",
                                        IsAsync = true,
                                        InputPayload = MaskPayload(blueprint, step, actPayload),
                                        OutputPayload = new Dictionary<string, object>
                                        {
                                            ["async_dispatched"] = true,
                                            ["dispatcher"] = _asyncDispatcher.GetDispatcherType()
                                        },
                                        StartedAt = DateTime.UtcNow.ToString("o"),
                                        FinishedAt = DateTime.UtcNow.ToString("o")
                                    };
                                    state.Traces.Add(excTrace);
                                    _asyncDispatcher.Dispatch(excCfg.Activity, actPayload, jobId, $"exc_{excCfg.Activity.ToLowerInvariant()}", _activityExecutor);
                                }
                                else
                                {
                                    var actSw = Stopwatch.StartNew();
                                    try
                                    {
                                        var actRes = await _activityExecutor.ExecuteActivityAsync(excCfg.Activity, actPayload, cancellationToken);
                                        actSw.Stop();
                                        state.Traces.Add(new StepExecutionTrace
                                        {
                                            StepId = $"exc_{excCfg.Activity.ToLowerInvariant()}",
                                            ActivityName = excCfg.Activity,
                                            Status = "COMPLETED",
                                            InputPayload = MaskPayload(blueprint, step, actPayload),
                                            OutputPayload = MaskPayload(blueprint, step, actRes ?? new Dictionary<string, object>()),
                                            DurationMs = actSw.Elapsed.TotalMilliseconds,
                                            StartedAt = DateTime.UtcNow.ToString("o"),
                                            FinishedAt = DateTime.UtcNow.ToString("o")
                                        });
                                    }
                                    catch (Exception actEx)
                                    {
                                        actSw.Stop();
                                    }
                                }
                            }
                        }

                        // Trigger Saga Rollback based on mode
                        if (sagaActions.Count > 0)
                        {
                            if (string.Equals(blueprint.CompensationMode, "async", StringComparison.OrdinalIgnoreCase))
                            {
                                var actionsCopy = new List<SagaAction>(sagaActions);
                                foreach (var action in actionsCopy)
                                {
                                    _asyncDispatcher.Dispatch(action.Compensation, action.Payload, jobId, $"saga_{action.StepId}", _activityExecutor);
                                }
                            }
                            else
                            {
                                await _sagaManager.ExecuteCompensationsAsync(sagaActions, cancellationToken);
                            }
                            state.Status = ExecutionStatus.ROLLED_BACK;
                        }

                        state.FinishedAt = DateTime.UtcNow.ToString("o");
                        state.DurationMs = workflowSw.Elapsed.TotalMilliseconds;

                        await _stateStore.SaveStateAsync(state, cancellationToken);
                        await _stateStore.PublishEventAsync(jobId, new LiveEvent("workflow_failed", state), cancellationToken);
                        throw;
                    }
                }
            }

            // Handle jump to step index
            if (step.JumpToStepIndex.HasValue && step.JumpToStepIndex.Value >= 0 && step.JumpToStepIndex.Value < steps.Count)
            {
                stepIndex = step.JumpToStepIndex.Value;
                continue;
            }
            if (step.RewindToStepIndex.HasValue && step.RewindToStepIndex.Value >= 0 && step.RewindToStepIndex.Value < steps.Count)
            {
                stepIndex = step.RewindToStepIndex.Value;
                continue;
            }

            // Handle jump_to named step
            if (!string.IsNullOrWhiteSpace(step.JumpTo))
            {
                var targetIdx = steps.FindIndex(s => s.Id == step.JumpTo);
                if (targetIdx >= 0)
                {
                    stepIndex = targetIdx;
                    continue;
                }
            }

            stepIndex++;
        }

        if (state.Status == ExecutionStatus.RUNNING)
        {
            state.Status = ExecutionStatus.COMPLETED;
        }
        if (state.OutputData == null)
        {
            state.OutputData = new Dictionary<string, object>(state.Data);
        }
        workflowSw.Stop();
        state.DurationMs = workflowSw.Elapsed.TotalMilliseconds;
        state.FinishedAt = DateTime.UtcNow.ToString("o");
        _idempotencyManager.Set(jobId, state);
        await _stateStore.SaveStateAsync(state, cancellationToken);

        await _stateStore.PublishEventAsync(jobId, new LiveEvent("workflow_completed", state), cancellationToken);

        return state;
    }

    private void DispatchAsyncActivity(WorkflowStep step, WorkflowBlueprint blueprint, ExecutionState state)
    {
        var trace = new StepExecutionTrace
        {
            StepId = step.Id,
            ActivityName = step.Activity,
            Status = "ASYNC_DISPATCHED",
            IsAsync = true,
            InputPayload = MaskPayload(blueprint, step, state.Data),
            OutputPayload = new Dictionary<string, object>
            {
                ["async_dispatched"] = true,
                ["dispatcher"] = _asyncDispatcher.GetDispatcherType()
            },
            StartedAt = DateTime.UtcNow.ToString("o"),
            FinishedAt = DateTime.UtcNow.ToString("o")
        };
        state.Traces.Add(trace);

        var dataCopy = new Dictionary<string, object>(state.Data);
        var actName = step.Activity!;
        _asyncDispatcher.Dispatch(actName, dataCopy, state.JobId, step.Id, _activityExecutor);
    }

    private async Task ExecuteLoopStepAsync(WorkflowStep step, WorkflowBlueprint blueprint, ExecutionState state, CancellationToken cancellationToken)
    {
        var loopCfg = step.Loop ?? step.ForEach;
        if (loopCfg == null) return;

        var items = new List<object>();
        string? itemsField = null;
        if (loopCfg.TryGetValue("items_field", out var fObj) && fObj != null) itemsField = fObj.ToString();
        else if (loopCfg.TryGetValue("items", out var iObj) && iObj is string str) itemsField = str;

        if (!string.IsNullOrWhiteSpace(itemsField) && state.Data.TryGetValue(itemsField, out var rawList))
        {
            if (rawList is System.Collections.IEnumerable enumerable && rawList is not string)
            {
                foreach (var it in enumerable) items.Add(it);
            }
        }
        else if (loopCfg.TryGetValue("items", out var directItems) && directItems is System.Collections.IEnumerable list)
        {
            foreach (var it in list) items.Add(it);
        }

        int maxIter = 100;
        if (loopCfg.TryGetValue("max_iterations", out var maxObj) && int.TryParse(maxObj?.ToString(), out var parsedMax) && parsedMax > 0)
        {
            maxIter = parsedMax;
        }

        string itemVarName = loopCfg.TryGetValue("item_variable", out var ivObj) && ivObj != null ? ivObj.ToString()! : "item";
        var loopResults = new List<object>();

        for (int i = 0; i < Math.Min(items.Count, maxIter); i++)
        {
            var item = items[i];
            var itemPayload = new Dictionary<string, object>(state.Data)
            {
                ["_item"] = item,
                ["_index"] = i,
                [itemVarName] = item
            };

            if (!string.IsNullOrWhiteSpace(step.Activity))
            {
                var outPayload = await _activityExecutor.ExecuteActivityAsync(step.Activity, itemPayload, cancellationToken);
                loopResults.Add(outPayload);
                foreach (var kvp in outPayload)
                {
                    state.Data[kvp.Key] = kvp.Value;
                }
            }
        }

        state.Data[$"{step.Id}_results"] = loopResults;
        var trace = new StepExecutionTrace
        {
            StepId = step.Id,
            ActivityName = step.Activity ?? "loop",
            Status = "COMPLETED",
            InputPayload = MaskPayload(blueprint, step, state.Data),
            OutputPayload = MaskPayload(blueprint, step, new Dictionary<string, object>
            {
                ["iterations"] = loopResults.Count,
                ["results"] = loopResults
            }),
            StartedAt = DateTime.UtcNow.ToString("o"),
            FinishedAt = DateTime.UtcNow.ToString("o")
        };
        state.Traces.Add(trace);
    }

    public async Task<ExecutionState> InitiateAsync(
        string workflowName,
        Dictionary<string, object> payload,
        CancellationToken cancellationToken = default
    )
    {
        var jobId = payload.TryGetValue("job_id", out var jVal) && jVal != null && !string.IsNullOrEmpty(jVal.ToString())
            ? jVal.ToString()!
            : $"job_{Guid.NewGuid():N}";

        var state = new ExecutionState
        {
            JobId = jobId,
            WorkflowName = workflowName,
            Status = ExecutionStatus.RUNNING,
            StartedAt = DateTime.UtcNow.ToString("o"),
            Data = payload
        };
        await _stateStore.SaveStateAsync(state, cancellationToken);

        _ = Task.Run(async () =>
        {
            try
            {
                await ExecuteSyncAsync(workflowName, payload, CancellationToken.None);
            }
            catch { }
        }, CancellationToken.None);

        return state;
    }

    public IAsyncEnumerable<LiveEvent> StreamEventsAsync(string jobId, CancellationToken cancellationToken = default)
    {
        return _stateStore.SubscribeEventsAsync(jobId, cancellationToken);
    }

    public IStateStore StateStore => _stateStore;

    public Task<ExecutionState?> GetExecutionStateAsync(string jobId, CancellationToken cancellationToken = default)
    {
        return _stateStore.GetStateAsync(jobId, cancellationToken);
    }

    public Task<IReadOnlyList<ExecutionState>> ListExecutionStatesAsync(
        string? workflowName = null,
        int limit = 100,
        CancellationToken cancellationToken = default
    )
    {
        return _stateStore.ListStatesAsync(workflowName, limit, cancellationToken);
    }

    private MiddlewareExceptionConfig? FindExceptionConfig(WorkflowStep? step, WorkflowBlueprint? blueprint, Exception ex)
    {
        if (ex == null) return null;
        var exType = ex.GetType().Name;
        var exMsg = ex.Message;

        // 1. Check step-level declared exceptions first
        if (step?.Exceptions != null && step.Exceptions.Count > 0)
        {
            foreach (var ec in step.Exceptions)
            {
                if (string.IsNullOrWhiteSpace(ec.Exception)) continue;

                if (string.Equals(ec.Exception, exType, StringComparison.OrdinalIgnoreCase) ||
                    exType.Contains(ec.Exception, StringComparison.OrdinalIgnoreCase) ||
                    ec.Exception.Contains(exType, StringComparison.OrdinalIgnoreCase))
                {
                    return ec;
                }

                if (!string.IsNullOrWhiteSpace(exMsg) && exMsg.Contains(ec.Exception, StringComparison.OrdinalIgnoreCase))
                {
                    return ec;
                }
            }
        }

        // 2. Check workflow-level middleware exceptions
        if (blueprint?.Middlewares != null)
        {
            foreach (var mw in blueprint.Middlewares)
            {
                if (mw.Exceptions == null) continue;
                foreach (var ec in mw.Exceptions)
                {
                    if (string.IsNullOrWhiteSpace(ec.Exception)) continue;

                    if (string.Equals(ec.Exception, exType, StringComparison.OrdinalIgnoreCase) ||
                        exType.Contains(ec.Exception, StringComparison.OrdinalIgnoreCase) ||
                        ec.Exception.Contains(exType, StringComparison.OrdinalIgnoreCase))
                    {
                        return ec;
                    }

                    if (!string.IsNullOrWhiteSpace(exMsg) && exMsg.Contains(ec.Exception, StringComparison.OrdinalIgnoreCase))
                    {
                        return ec;
                    }
                }
            }

            // 3. Middleware Wildcard '*'
            foreach (var mw in blueprint.Middlewares)
            {
                if (mw.Exceptions == null) continue;
                foreach (var ec in mw.Exceptions)
                {
                    if (ec.Exception == "*" || string.Equals(ec.Exception, "exception", StringComparison.OrdinalIgnoreCase))
                    {
                        return ec;
                    }
                }
            }
        }

        // 4. Global Exception Handler (Declarative Blueprint safety net)
        var gh = blueprint?.ErrorHandling?.GlobalHandler ?? blueprint?.GlobalExceptionHandler;
        if (gh != null && !string.IsNullOrWhiteSpace(gh.Activity))
        {
            return new MiddlewareExceptionConfig
            {
                Exception = "*",
                Code = gh.Code != 0 ? gh.Code : 500,
                Message = gh.Message,
                Activity = gh.Activity,
                IsAsync = gh.GetIsAsync(),
                Headers = gh.Headers
            };
        }

        return null;
    }

    private Dictionary<string, object> MaskPayload(
        WorkflowBlueprint blueprint,
        WorkflowStep? step,
        Dictionary<string, object>? payload
    )
    {
        if (payload == null) return new Dictionary<string, object>();
        var (fields, maskVal) = blueprint.GetMaskConfig(step);
        return MaskingService.MaskPayload(payload, fields, maskVal);
    }
}
