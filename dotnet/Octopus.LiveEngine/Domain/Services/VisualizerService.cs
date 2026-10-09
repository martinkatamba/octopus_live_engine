using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Domain.Services;

public class VisualizerService : IVisualizerUseCase
{
    private readonly IBlueprintRepository _blueprintRepo;
    private readonly IStateStore _stateStore;

    private record PrevItem(string Id, string Label = "", string Color = "#64748b", bool Executed = false);

    private record ExcConfigItem(
        string Activity,
        string Exception,
        int Code,
        bool IsAsync,
        bool IsMiddleware,
        string? SourceStepId = null,
        string? SourceName = null,
        string? Message = null
    );

    public VisualizerService(IBlueprintRepository blueprintRepo, IStateStore stateStore)
    {
        _blueprintRepo = blueprintRepo;
        _stateStore = stateStore;
    }

    public Task<IReadOnlyList<WorkflowBlueprint>> ListBlueprintsAsync(CancellationToken cancellationToken = default)
    {
        return _blueprintRepo.ListBlueprintsAsync(cancellationToken);
    }

    public async Task<Dictionary<string, object>> CompileX6DiagramAsync(
        string workflowName,
        string? jobId,
        string? route = null,
        string? method = null,
        CancellationToken cancellationToken = default
    )
    {
        var blueprint = await _blueprintRepo.GetBlueprintAsync(workflowName, cancellationToken)
            ?? throw new KeyNotFoundException($"Blueprint '{workflowName}' not found.");

        ExecutionState? state = null;
        if (!string.IsNullOrWhiteSpace(jobId))
        {
            state = await _stateStore.GetStateAsync(jobId, cancellationToken);
            if (state == null)
            {
                // Fallback for Temporal Workflow IDs: e.g. "wf-customer_auth_portal-12345" -> "12345"
                if (jobId.StartsWith($"wf-{workflowName}-", StringComparison.OrdinalIgnoreCase))
                {
                    var cleanId = jobId.Substring($"wf-{workflowName}-".Length);
                    state = await _stateStore.GetStateAsync(cleanId, cancellationToken);
                }
                else if (jobId.StartsWith("wf-", StringComparison.OrdinalIgnoreCase))
                {
                    var parts = jobId.Split('-', 3);
                    if (parts.Length == 3)
                    {
                        state = await _stateStore.GetStateAsync(parts[2], cancellationToken);
                    }
                }
            }
        }

        var statusMap = new Dictionary<string, string>();
        var durationMap = new Dictionary<string, double>();
        var inputMap = new Dictionary<string, object>();
        var outputMap = new Dictionary<string, object>();
        var errorMap = new Dictionary<string, string?>();
        if (state != null && state.Traces != null)
        {
            foreach (var trace in state.Traces)
            {
                if (!string.IsNullOrWhiteSpace(trace.StepId))
                {
                    statusMap[trace.StepId] = trace.Status;
                    durationMap[trace.StepId] = trace.DurationMs;
                    if (trace.InputPayload != null) inputMap[trace.StepId] = trace.InputPayload;
                    if (trace.OutputPayload != null) outputMap[trace.StepId] = trace.OutputPayload;
                    if (!string.IsNullOrWhiteSpace(trace.ErrorMessage)) errorMap[trace.StepId] = trace.ErrorMessage;
                }
                if (!string.IsNullOrWhiteSpace(trace.ActivityName))
                {
                    statusMap[trace.ActivityName] = trace.Status;
                    if (trace.InputPayload != null) inputMap[trace.ActivityName] = trace.InputPayload;
                    if (trace.OutputPayload != null) outputMap[trace.ActivityName] = trace.OutputPayload;
                }
            }
        }

        var routesList = new List<Dictionary<string, object>>();
        if (blueprint.Api != null && blueprint.Api.Count > 0)
        {
            foreach (var ep in blueprint.Api)
            {
                routesList.Add(new Dictionary<string, object>
                {
                    ["route"] = ep.Route,
                    ["method"] = (ep.Method ?? "POST").ToUpperInvariant(),
                    ["summary"] = ep.Summary ?? "",
                    ["description"] = ep.Description ?? "",
                    ["step_count"] = ep.Steps?.Count ?? 0
                });
            }
        }

        ApiEndpointConfig? selectedEp = null;
        string? selectedRoute = null;
        string? selectedMethod = null;

        // 1. If we have a live job execution state, match by state.Route / Method or trace step IDs
        if (state != null)
        {
            if (!string.IsNullOrWhiteSpace(state.Route) && blueprint.Api != null && blueprint.Api.Count > 0)
            {
                selectedEp = blueprint.Api.FirstOrDefault(ep =>
                    !string.IsNullOrWhiteSpace(ep.Route) && (
                        string.Equals(ep.Route, state.Route, StringComparison.OrdinalIgnoreCase) ||
                        state.Route.EndsWith(ep.Route, StringComparison.OrdinalIgnoreCase) ||
                        ep.Route.EndsWith(state.Route, StringComparison.OrdinalIgnoreCase)));
            }

            if (selectedEp == null && blueprint.Api != null && state.Traces != null && state.Traces.Count > 0)
            {
                var traceStepIds = state.Traces
                    .SelectMany(t => new[] { t.StepId, t.ActivityName })
                    .Where(s => !string.IsNullOrWhiteSpace(s))
                    .ToHashSet(StringComparer.OrdinalIgnoreCase);

                foreach (var ep in blueprint.Api)
                {
                    if (ep.Steps != null && ep.Steps.Any(s =>
                        (!string.IsNullOrWhiteSpace(s.Id) && traceStepIds.Contains(s.Id)) ||
                        (!string.IsNullOrWhiteSpace(s.Activity) && traceStepIds.Contains(s.Activity))))
                    {
                        selectedEp = ep;
                        break;
                    }
                }
            }
        }

        // 2. If no job match, check explicit requested route / method from query parameter
        if (selectedEp == null && !string.IsNullOrWhiteSpace(route) && blueprint.Api != null)
        {
            foreach (var ep in blueprint.Api)
            {
                if (string.IsNullOrWhiteSpace(ep.Route)) continue;
                bool matchRoute = string.Equals(ep.Route, route, StringComparison.OrdinalIgnoreCase) ||
                                  route.EndsWith(ep.Route, StringComparison.OrdinalIgnoreCase) ||
                                  ep.Route.EndsWith(route, StringComparison.OrdinalIgnoreCase);
                bool matchMethod = string.IsNullOrWhiteSpace(method) ||
                                   string.Equals(ep.Method, method, StringComparison.OrdinalIgnoreCase);
                if (matchRoute && matchMethod)
                {
                    selectedEp = ep;
                    break;
                }
            }
        }

        // 3. Resolve the step list to render
        List<WorkflowStep> stepsToVisualize;
        if (selectedEp != null && selectedEp.Steps != null && selectedEp.Steps.Count > 0)
        {
            stepsToVisualize = selectedEp.Steps;
            selectedRoute = selectedEp.Route;
            selectedMethod = (selectedEp.Method ?? "POST").ToUpperInvariant();
        }
        else if (blueprint.Steps != null && blueprint.Steps.Count > 0)
        {
            stepsToVisualize = blueprint.Steps;
            selectedRoute = "default";
            selectedMethod = "EXECUTE";
        }
        else if (blueprint.Api != null && blueprint.Api.Count > 0 && blueprint.Api[0].Steps != null)
        {
            selectedEp = blueprint.Api[0];
            stepsToVisualize = selectedEp.Steps;
            selectedRoute = selectedEp.Route;
            selectedMethod = (selectedEp.Method ?? "POST").ToUpperInvariant();
        }
        else
        {
            stepsToVisualize = new List<WorkflowStep>();
        }

        bool hasLiveExecution = state != null;
        string startStatus = hasLiveExecution ? "COMPLETED" : "PENDING";

        var nodes = new List<Dictionary<string, object>>();
        var edges = new List<Dictionary<string, object>>();

        // 1. Start Node
        var startNode = new Dictionary<string, object>
        {
            ["id"] = "node__start",
            ["shape"] = "octopus-start-node",
            ["label"] = "START",
            ["width"] = 120,
            ["height"] = 50,
            ["data"] = new Dictionary<string, object>
            {
                ["step_id"] = "start",
                ["type"] = "start",
                ["title"] = "Workflow Start",
                ["status"] = startStatus,
                ["executed"] = hasLiveExecution
            }
        };
        if (hasLiveExecution)
        {
            startNode["attrs"] = new Dictionary<string, object>
            {
                ["body"] = new Dictionary<string, object>
                {
                    ["rx"] = 25,
                    ["ry"] = 25,
                    ["fill"] = "#064e3b",
                    ["stroke"] = "#10b981",
                    ["strokeWidth"] = 3,
                    ["filter"] = "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    ["opacity"] = 1.0
                },
                ["label"] = new Dictionary<string, object>
                {
                    ["fill"] = "#6ee7b7",
                    ["fontSize"] = 13,
                    ["fontWeight"] = 700
                }
            };
        }
        nodes.Add(startNode);

        // 2. Ingress Middleware Guards Node (Filtered by route excludes / includes)
        var preMiddlewares = new List<MiddlewareConfig>();
        var postMiddlewares = new List<MiddlewareConfig>();
        if (blueprint.Middlewares != null)
        {
            foreach (var m in blueprint.Middlewares)
            {
                if (IsMiddlewareActiveForRoute(m, selectedRoute))
                {
                    if (string.Equals(m.Type, "post-request", StringComparison.OrdinalIgnoreCase))
                    {
                        postMiddlewares.Add(m);
                    }
                    else
                    {
                        preMiddlewares.Add(m);
                    }
                }
            }
        }

        bool hasIngressGuards = preMiddlewares.Count > 0;
        bool hasEgressGuards = postMiddlewares.Count > 0;
        List<PrevItem> initialPrev;

        bool isIngressFailed = false;
        if (hasIngressGuards)
        {
            var mwExcNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var m in preMiddlewares)
            {
                if (m.Exceptions != null)
                {
                    foreach (var e in m.Exceptions)
                    {
                        if (!string.IsNullOrWhiteSpace(e.Exception))
                        {
                            mwExcNames.Add(e.Exception);
                        }
                    }
                }
            }

            if (state != null && state.Status == ExecutionStatus.FAILED)
            {
                var stateErr = (state.Error ?? "").ToLowerInvariant();
                foreach (var me in mwExcNames)
                {
                    if (stateErr.Contains(me.ToLowerInvariant()))
                    {
                        isIngressFailed = true;
                        break;
                    }
                }
                if (!isIngressFailed && state.Traces != null)
                {
                    foreach (var t in state.Traces)
                    {
                        var tErr = (t.ErrorMessage ?? "").ToLowerInvariant();
                        foreach (var me in mwExcNames)
                        {
                            if (tErr.Contains(me.ToLowerInvariant()))
                            {
                                isIngressFailed = true;
                                break;
                            }
                        }
                        if (isIngressFailed) break;
                    }
                }
            }

            string ingressStatus = isIngressFailed ? "FAILED" : (hasLiveExecution ? "COMPLETED" : "PENDING");

            var mwNamesList = preMiddlewares.Where(m => !string.IsNullOrWhiteSpace(m.Name)).Select(m => m.Name).ToList();

            var ingressNode = new Dictionary<string, object>
            {
                ["id"] = "node__ingress_guards",
                ["shape"] = "octopus-activity-node",
                ["label"] = "🛡️ Ingress Middleware Guards",
                ["width"] = 220,
                ["height"] = 55,
                ["data"] = new Dictionary<string, object>
                {
                    ["step_id"] = "ingress_guards",
                    ["type"] = "ingress_guards",
                    ["title"] = "🛡️ Ingress Middleware Guards",
                    ["description"] = "Pre-execution security filters, JWT authentication, IP whitelisting, and rate-limiting guards executed prior to workflow pipeline dispatch.",
                    ["status"] = ingressStatus,
                    ["executed"] = hasLiveExecution,
                    ["middlewares"] = mwNamesList
                }
            };

            if (isIngressFailed)
            {
                ingressNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#ef4444",
                        ["strokeWidth"] = 3,
                        ["fill"] = "#2d1215",
                        ["filter"] = "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                        ["opacity"] = 1.0
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#fca5a5",
                        ["fontWeight"] = 700
                    }
                };
            }
            else if (hasLiveExecution)
            {
                ingressNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#10b981",
                        ["strokeWidth"] = 3,
                        ["fill"] = "#0f2e22",
                        ["filter"] = "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        ["opacity"] = 1.0
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#6ee7b7",
                        ["fontWeight"] = 600
                    }
                };
            }
            nodes.Add(ingressNode);

            var startToIngressStroke = isIngressFailed ? "#ef4444" : (hasLiveExecution ? "#10b981" : "#6366f1");
            var startToIngressEdge = new Dictionary<string, object>
            {
                ["id"] = "edge__node__start__to__node__ingress_guards",
                ["source"] = "node__start",
                ["target"] = "node__ingress_guards",
                ["attrs"] = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = startToIngressStroke,
                        ["strokeWidth"] = 2,
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = startToIngressStroke }
                    }
                }
            };
            edges.Add(startToIngressEdge);

            initialPrev = new List<PrevItem> { new("node__ingress_guards", "✅ Quota & Security OK", "#10b981", hasLiveExecution && !isIngressFailed) };
        }
        else
        {
            initialPrev = new List<PrevItem> { new("node__start", "", "#6366f1", hasLiveExecution) };
        }

        List<PrevItem> ProcessStepList(
            List<WorkflowStep> stepList,
            List<PrevItem> prevHeads,
            string scopePrefix,
            bool isBranchActive
        )
        {
            var currPrev = prevHeads;

            for (int idx = 0; idx < stepList.Count; idx++)
            {
                var step = stepList[idx];
                var rawStepId = !string.IsNullOrWhiteSpace(step.Id) ? step.Id : $"step_{idx}";
                var nodeId = string.IsNullOrWhiteSpace(scopePrefix)
                    ? $"node__{rawStepId}"
                    : $"node__{scopePrefix}__{rawStepId}";

                bool executed = false;
                string stepStatus = "PENDING";

                if (statusMap.TryGetValue(rawStepId, out var st))
                {
                    if (st is "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")
                    {
                        stepStatus = "COMPLETED";
                        executed = true;
                    }
                    else
                    {
                        stepStatus = st;
                        executed = st is "COMPLETED" or "RUNNING" or "FAILED" or "ROLLED_BACK";
                    }
                }
                else if (!string.IsNullOrWhiteSpace(step.Activity) && statusMap.TryGetValue(step.Activity, out var ast))
                {
                    if (ast is "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")
                    {
                        stepStatus = "COMPLETED";
                        executed = true;
                    }
                    else
                    {
                        stepStatus = ast;
                        executed = ast is "COMPLETED" or "RUNNING" or "FAILED" or "ROLLED_BACK";
                    }
                }
                else if (state != null && state.CurrentStepId == rawStepId)
                {
                    stepStatus = "RUNNING";
                    executed = true;
                }
                else if (hasLiveExecution)
                {
                    stepStatus = "SKIPPED";
                }

                bool isLoop = step.Loop != null || step.ForEach != null;
                bool isAsync = step.Async || step.IsAsync;
                var thenSteps = step.GetThenSteps();
                var elseSteps = step.GetElseSteps();
                bool hasBranches = thenSteps.Count > 0 || elseSteps.Count > 0 || !string.IsNullOrWhiteSpace(step.BranchingStep) || step.Condition != null || step.Conditions != null;

                string shape = "octopus-activity-node";
                string typeStr = "activity";
                string nodeLabel = rawStepId;

                bool thenExecuted = false;
                bool elseExecuted = false;

                if (isLoop)
                {
                    shape = "octopus-loop-node";
                    typeStr = "loop";
                    nodeLabel = $"⚏ {rawStepId}\n[Loop Iteration]";
                }
                else if (isAsync)
                {
                    shape = "octopus-activity-node";
                    typeStr = "async";
                    nodeLabel = $"⚡ {rawStepId}\n[Async Background]";
                }
                else if (hasBranches)
                {
                    shape = "octopus-branch-node";
                    typeStr = "branch";
                    nodeLabel = $"⚏ {rawStepId}";

                    foreach (var ts in thenSteps)
                    {
                        if (statusMap.TryGetValue(ts.Id, out var tst) && (tst is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED") ||
                            (!string.IsNullOrWhiteSpace(ts.Activity) && statusMap.TryGetValue(ts.Activity, out var tast) && (tast is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")))
                        {
                            thenExecuted = true;
                            executed = true;
                            stepStatus = "COMPLETED";
                            break;
                        }
                    }
                    foreach (var es in elseSteps)
                    {
                        if (statusMap.TryGetValue(es.Id, out var est) && (est is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED") ||
                            (!string.IsNullOrWhiteSpace(es.Activity) && statusMap.TryGetValue(es.Activity, out var east) && (east is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")))
                        {
                            elseExecuted = true;
                            executed = true;
                            stepStatus = "COMPLETED";
                            break;
                        }
                    }
                }
                else if (!string.IsNullOrWhiteSpace(step.HumanStep))
                {
                    shape = "octopus-human-node";
                    typeStr = "human";
                }

                var nodeData = new Dictionary<string, object?>
                {
                    ["step_id"] = rawStepId,
                    ["title"] = rawStepId,
                    ["activity"] = step.Activity,
                    ["status"] = stepStatus,
                    ["executed"] = executed,
                    ["type"] = typeStr,
                    ["is_loop"] = isLoop,
                    ["loop"] = step.Loop,
                    ["for_each"] = step.ForEach,
                    ["is_async"] = isAsync,
                    ["condition"] = step.Condition,
                    ["conditions"] = step.Conditions,
                    ["compensation"] = step.Compensation,
                    ["description"] = step.Description,
                    ["timeout_seconds"] = step.TimeoutSeconds,
                    ["index"] = idx + 1,
                    ["duration_ms"] = durationMap.TryGetValue(rawStepId, out var dur) ? dur : 0.0,
                    ["input_payload"] = inputMap.TryGetValue(rawStepId, out var inp) ? inp : (!string.IsNullOrWhiteSpace(step.Activity) && inputMap.TryGetValue(step.Activity, out var ainp) ? ainp : null),
                    ["output_payload"] = outputMap.TryGetValue(rawStepId, out var outp) ? outp : (!string.IsNullOrWhiteSpace(step.Activity) && outputMap.TryGetValue(step.Activity, out var aoutp) ? aoutp : null),
                    ["error"] = errorMap.TryGetValue(rawStepId, out var err) ? err : null
                };

                if (hasBranches)
                {
                    var thenStepIds = thenSteps.Select(s => !string.IsNullOrWhiteSpace(s.Id) ? s.Id : s.Activity).ToList();
                    var elseStepIds = elseSteps.Select(s => !string.IsNullOrWhiteSpace(s.Id) ? s.Id : s.Activity).ToList();

                    var branchInfo = new Dictionary<string, object?>
                    {
                        ["has_branches"] = true,
                        ["condition"] = step.Condition,
                        ["conditions"] = step.Conditions,
                        ["then_step_ids"] = thenStepIds,
                        ["else_step_ids"] = elseStepIds
                    };

                    if (hasLiveExecution && (thenExecuted || elseExecuted || executed))
                    {
                        var evalInfo = new Dictionary<string, object?>
                        {
                            ["evaluated"] = true,
                            ["verdict"] = thenExecuted ? "TRUE" : (elseExecuted ? "FALSE" : "UNKNOWN"),
                            ["verdict_label"] = thenExecuted ? "TRUE (Match)" : (elseExecuted ? "FALSE (Else)" : "EXECUTED"),
                            ["branch_taken"] = thenExecuted ? "THEN" : (elseExecuted ? "ELSE" : "NONE")
                        };

                        if (step.Condition != null)
                        {
                            var varName = !string.IsNullOrWhiteSpace(step.Condition.Variable) ? step.Condition.Variable : step.Condition.Field;
                            if (!string.IsNullOrWhiteSpace(varName))
                            {
                                evalInfo["variable_name"] = varName;
                                evalInfo["runtime_value"] = ExtractRuntimeVariable(state, varName);
                                evalInfo["expected_value"] = step.Condition.Value;
                                evalInfo["operator"] = !string.IsNullOrWhiteSpace(step.Condition.Operator) ? step.Condition.Operator : "==";
                            }
                            if (!string.IsNullOrWhiteSpace(step.Condition.Rule))
                            {
                                evalInfo["rule"] = step.Condition.Rule;
                            }
                        }
                        branchInfo["evaluation"] = evalInfo;
                    }
                    nodeData["branch_info"] = branchInfo;
                }

                var node = new Dictionary<string, object>
                {
                    ["id"] = nodeId,
                    ["shape"] = shape,
                    ["width"] = 220,
                    ["height"] = 75,
                    ["label"] = nodeLabel,
                    ["data"] = nodeData
                };

                // Apply Node Path Highlighting
                if (stepStatus is "FAILED" or "ROLLED_BACK")
                {
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#ef4444",
                            ["strokeWidth"] = 3,
                            ["fill"] = "#2d1215",
                            ["filter"] = "drop-shadow(0 0 16px rgba(239, 68, 68, 0.75))",
                            ["opacity"] = 1.0
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#fca5a5",
                            ["fontWeight"] = 700
                        }
                    };
                }
                else if (stepStatus == "TIMED_OUT")
                {
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#f97316",
                            ["strokeWidth"] = 3,
                            ["fill"] = "#2b180d",
                            ["filter"] = "drop-shadow(0 0 16px rgba(249, 115, 22, 0.75))",
                            ["opacity"] = 1.0
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#fdba74",
                            ["fontWeight"] = 700
                        }
                    };
                }
                else if (stepStatus is "AWAITING_APPROVAL" or "RUNNING" or "AWAITING_SIGNAL")
                {
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#f59e0b",
                            ["strokeWidth"] = 3,
                            ["fill"] = "#2a1d08",
                            ["filter"] = "drop-shadow(0 0 14px rgba(245, 158, 11, 0.85))",
                            ["opacity"] = 1.0
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#fde68a",
                            ["fontWeight"] = 700
                        }
                    };
                }
                else if (executed)
                {
                    var bodyAttrs = new Dictionary<string, object>
                    {
                        ["stroke"] = "#10b981",
                        ["strokeWidth"] = 3,
                        ["fill"] = "#0f2e22",
                        ["filter"] = "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        ["opacity"] = 1.0
                    };
                    if (isLoop)
                    {
                        bodyAttrs["strokeDasharray"] = "5 4";
                    }
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = bodyAttrs,
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#6ee7b7",
                            ["fontWeight"] = 600
                        }
                    };
                }
                else if (isLoop)
                {
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#38bdf8",
                            ["strokeWidth"] = 2,
                            ["strokeDasharray"] = "5 4",
                            ["fill"] = "#0c2333",
                            ["filter"] = "drop-shadow(0 0 12px rgba(56, 189, 248, 0.45))",
                            ["opacity"] = hasLiveExecution ? 0.35 : 1.0
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#7dd3fc",
                            ["fontWeight"] = 600,
                            ["opacity"] = hasLiveExecution ? 0.45 : 1.0
                        }
                    };
                }
                else if (hasLiveExecution)
                {
                    node["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#334155",
                            ["strokeWidth"] = 1.5,
                            ["fill"] = "#111827",
                            ["opacity"] = 0.35
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#64748b",
                            ["opacity"] = 0.45
                        }
                    };
                }

                nodes.Add(node);

                // Connect predecessors
                foreach (var p in currPrev)
                {
                    Dictionary<string, object> edgeAttrs;
                    if (p.Executed && executed)
                    {
                        edgeAttrs = new Dictionary<string, object>
                        {
                            ["line"] = new Dictionary<string, object>
                            {
                                ["stroke"] = "#10b981",
                                ["strokeWidth"] = 3.5,
                                ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 10, ["height"] = 8, ["fill"] = "#10b981" },
                                ["filter"] = "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                                ["opacity"] = 1.0
                            }
                        };
                    }
                    else if (p.Executed && stepStatus is "RUNNING" or "AWAITING_APPROVAL")
                    {
                        edgeAttrs = new Dictionary<string, object>
                        {
                            ["line"] = new Dictionary<string, object>
                            {
                                ["stroke"] = "#f59e0b",
                                ["strokeWidth"] = 3.5,
                                ["strokeDasharray"] = "6 4",
                                ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 10, ["height"] = 8, ["fill"] = "#f59e0b" },
                                ["filter"] = "drop-shadow(0 0 10px rgba(245, 158, 11, 0.75))",
                                ["opacity"] = 1.0
                            }
                        };
                    }
                    else if (hasLiveExecution)
                    {
                        edgeAttrs = new Dictionary<string, object>
                        {
                            ["line"] = new Dictionary<string, object>
                            {
                                ["stroke"] = "#1e293b",
                                ["strokeWidth"] = 1.5,
                                ["strokeDasharray"] = "4 4",
                                ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 6, ["height"] = 4, ["fill"] = "#334155" },
                                ["opacity"] = 0.25
                            }
                        };
                    }
                    else
                    {
                        edgeAttrs = new Dictionary<string, object>
                        {
                            ["line"] = new Dictionary<string, object>
                            {
                                ["stroke"] = "#64748b",
                                ["strokeWidth"] = 2,
                                ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = "#64748b" },
                                ["opacity"] = 1.0
                            }
                        };
                    }

                    var edgeObj = new Dictionary<string, object>
                    {
                        ["id"] = $"edge_{p.Id}_to_{nodeId}",
                        ["source"] = p.Id,
                        ["target"] = nodeId,
                        ["attrs"] = edgeAttrs
                    };

                    if (!string.IsNullOrWhiteSpace(p.Label))
                    {
                        var labelColor = p.Color;
                        if (p.Executed && executed) labelColor = "#10b981";
                        else if (hasLiveExecution) labelColor = "#475569";

                        edgeObj["labels"] = new List<Dictionary<string, object>>
                        {
                            new()
                            {
                                ["attrs"] = new Dictionary<string, object>
                                {
                                    ["text"] = new Dictionary<string, object>
                                    {
                                        ["text"] = p.Label,
                                        ["fill"] = labelColor,
                                        ["fontSize"] = 11,
                                        ["fontWeight"] = "bold"
                                    },
                                    ["rect"] = new Dictionary<string, object>
                                    {
                                        ["fill"] = "#0f172a",
                                        ["rx"] = 4,
                                        ["ry"] = 4,
                                        ["stroke"] = labelColor,
                                        ["strokeWidth"] = 1
                                    }
                                }
                            }
                        };
                    }

                    edges.Add(edgeObj);
                }

                if (hasBranches)
                {
                    var branchOuts = new List<PrevItem>();
                    if (thenSteps.Count > 0)
                    {
                        bool thenExec = false;
                        foreach (var ts in thenSteps)
                        {
                            if (statusMap.TryGetValue(ts.Id, out var tst) && (tst is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED") ||
                                (!string.IsNullOrWhiteSpace(ts.Activity) && statusMap.TryGetValue(ts.Activity, out var tast) && (tast is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")))
                            {
                                thenExec = true;
                                break;
                            }
                        }
                        var thenHeads = new List<PrevItem> { new(nodeId, "TRUE (Match)", "#10b981", executed && thenExec) };
                        var thenEnds = ProcessStepList(thenSteps, thenHeads, $"{rawStepId}_then", isBranchActive);
                        branchOuts.AddRange(thenEnds);
                    }
                    if (elseSteps.Count > 0)
                    {
                        bool elseExec = false;
                        foreach (var es in elseSteps)
                        {
                            if (statusMap.TryGetValue(es.Id, out var est) && (est is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED") ||
                                (!string.IsNullOrWhiteSpace(es.Activity) && statusMap.TryGetValue(es.Activity, out var east) && (east is "COMPLETED" or "ASYNC_DISPATCHED" or "BACKGROUND_DISPATCHED" or "DISPATCHED")))
                            {
                                elseExec = true;
                                break;
                            }
                        }
                        var elseHeads = new List<PrevItem> { new(nodeId, "FALSE (Else)", "#38bdf8", executed && elseExec) };
                        var elseEnds = ProcessStepList(elseSteps, elseHeads, $"{rawStepId}_else", isBranchActive);
                        branchOuts.AddRange(elseEnds);
                    }
                    currPrev = branchOuts;
                }
                else
                {
                    currPrev = new List<PrevItem> { new(nodeId, "", "#64748b", executed) };
                }
            }

            return currPrev;
        }

        var finalHeads = ProcessStepList(stepsToVisualize, initialPrev, "", true);

        // 3. Exception Handlers & Failure Terminal Nodes Collection
        var excConfigs = new List<ExcConfigItem>();
        if (blueprint.ErrorHandling?.GlobalHandler != null && !string.IsNullOrWhiteSpace(blueprint.ErrorHandling.GlobalHandler.Activity))
        {
            var gh = blueprint.ErrorHandling.GlobalHandler;
            excConfigs.Add(new ExcConfigItem(
                gh.Activity,
                "*",
                gh.Code > 0 ? gh.Code : 500,
                gh.IsAsync || gh.Async,
                false,
                null,
                "GlobalExceptionHandler",
                gh.Message ?? "Internal Server Error"
            ));
        }

        if (preMiddlewares != null)
        {
            foreach (var mw in preMiddlewares)
            {
                if (mw.Exceptions != null)
                {
                    foreach (var exc in mw.Exceptions)
                    {
                        if (!string.IsNullOrWhiteSpace(exc.Activity))
                        {
                            excConfigs.Add(new ExcConfigItem(
                                exc.Activity,
                                !string.IsNullOrWhiteSpace(exc.Exception) ? exc.Exception : "MiddlewareException",
                                exc.Code,
                                exc.IsAsync || exc.Async,
                                true,
                                null,
                                mw.Name,
                                exc.Message
                            ));
                        }
                    }
                }
            }
        }

        CollectStepExceptions(stepsToVisualize, excConfigs);

        if (state != null && state.Traces != null)
        {
            foreach (var trace in state.Traces)
            {
                var sId = trace.StepId ?? "";
                var aName = trace.ActivityName ?? "";
                if ((sId.StartsWith("exc_") || aName.Contains("audit", StringComparison.OrdinalIgnoreCase)) &&
                    !excConfigs.Any(e => string.Equals(e.Activity, aName, StringComparison.OrdinalIgnoreCase)))
                {
                    excConfigs.Add(new ExcConfigItem(
                        aName,
                        !string.IsNullOrWhiteSpace(trace.ErrorMessage) ? trace.ErrorMessage : "Exception",
                        400,
                        true,
                        false,
                        null
                    ));
                }
            }
        }

        var processedExcIds = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var exc in excConfigs)
        {
            var excName = !string.IsNullOrWhiteSpace(exc.Exception) ? exc.Exception : "MiddlewareException";
            var excCode = exc.Code > 0 ? exc.Code : 400;
            var excActName = exc.Activity;
            var isGlobalExc = excName == "*" || excCode == 500 || excActName.Contains("global", StringComparison.OrdinalIgnoreCase);
            var excNodeId = isGlobalExc ? $"node__exc_{excActName.ToLowerInvariant()}" : $"node__exc_{excName.ToLowerInvariant()}";

            var srcId = !string.IsNullOrWhiteSpace(exc.SourceStepId)
                ? $"node__{exc.SourceStepId}"
                : (isGlobalExc && state?.Traces?.FirstOrDefault(t => t.Status == "FAILED" && !(t.StepId ?? "").StartsWith("exc_")) != null
                    ? $"node__{state.Traces.First(t => t.Status == "FAILED" && !(t.StepId ?? "").StartsWith("exc_")).StepId}"
                    : (isGlobalExc && stepsToVisualize.Count > 0 ? $"node__{stepsToVisualize[0].Id}" : (hasIngressGuards ? "node__ingress_guards" : "node__start")));

            if (!nodes.Any(n => (string)n["id"] == srcId))
            {
                srcId = hasIngressGuards ? "node__ingress_guards" : "node__start";
            }

            if (processedExcIds.Contains(excNodeId))
            {
                continue;
            }
            processedExcIds.Add(excNodeId);

            bool isExcExecuted = false;
            if (state != null)
            {
                var sErr = (state.Error ?? "").ToLowerInvariant();
                var excSrcStep = (exc.SourceStepId ?? "").ToLowerInvariant();

                if (isGlobalExc)
                {
                    if (state.Status == ExecutionStatus.FAILED)
                    {
                        if (state.Traces?.Any(t => string.Equals(t.ActivityName, excActName, StringComparison.OrdinalIgnoreCase) || string.Equals(t.StepId, $"exc_{excActName}", StringComparison.OrdinalIgnoreCase)) == true)
                        {
                            isExcExecuted = true;
                        }
                    }
                }
                else
                {
                    if (state.Traces != null)
                    {
                        foreach (var t in state.Traces)
                        {
                            var tErr = (t.ErrorMessage ?? "").ToLowerInvariant();
                            var tStep = (t.StepId ?? "").ToLowerInvariant();
                            var aName = t.ActivityName ?? "";

                            string inpExc = "";
                            string inpErr = "";
                            string inpStep = "";
                            int inpCode = 0;
                            if (t.InputPayload != null)
                            {
                                if (t.InputPayload.TryGetValue("exception", out var ie)) inpExc = ie?.ToString()?.ToLowerInvariant() ?? "";
                                if (t.InputPayload.TryGetValue("error", out var ir)) inpErr = ir?.ToString()?.ToLowerInvariant() ?? "";
                                if (t.InputPayload.TryGetValue("step_id", out var is_)) inpStep = is_?.ToString()?.ToLowerInvariant() ?? "";
                                else if (t.InputPayload.TryGetValue("failing_step_id", out var ifs)) inpStep = ifs?.ToString()?.ToLowerInvariant() ?? "";
                                if (t.InputPayload.TryGetValue("status_code", out var isc) && isc is int c) inpCode = c;
                            }

                            if (!string.IsNullOrEmpty(excSrcStep))
                            {
                                if (string.Equals(aName, excActName, StringComparison.OrdinalIgnoreCase) || tStep == $"exc_{excSrcStep}_{excName.ToLowerInvariant()}" || tStep == $"exc_{excActName.ToLowerInvariant()}")
                                {
                                    if (inpStep == excSrcStep || inpExc == excName.ToLowerInvariant() || inpErr == excName.ToLowerInvariant() || inpExc.Contains(excName.ToLowerInvariant()) || (inpCode > 0 && inpCode == excCode))
                                    {
                                        isExcExecuted = true;
                                        break;
                                    }
                                }
                            }
                            else
                            {
                                if (string.Equals(aName, excActName, StringComparison.OrdinalIgnoreCase) || tStep == $"exc_{excName.ToLowerInvariant()}" || tStep == $"exc_{excActName.ToLowerInvariant()}")
                                {
                                    if (!string.IsNullOrEmpty(inpStep) && inpStep != "ingress_guards" && inpStep != "egress_guards" && inpStep != "middleware")
                                    {
                                        continue;
                                    }
                                    if (inpExc == excName.ToLowerInvariant() || inpErr == excName.ToLowerInvariant() || tErr.Contains(excName.ToLowerInvariant()) || tStep == $"exc_{excName.ToLowerInvariant()}")
                                    {
                                        isExcExecuted = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if (!isExcExecuted)
                    {
                        if (state.Data != null)
                        {
                            if (state.Data.TryGetValue("exception", out var dExc) && string.Equals(dExc?.ToString(), excName, StringComparison.OrdinalIgnoreCase))
                            {
                                isExcExecuted = true;
                            }
                            else if (state.Data.TryGetValue("error", out var dErr) && string.Equals(dErr?.ToString(), excName, StringComparison.OrdinalIgnoreCase))
                            {
                                isExcExecuted = true;
                            }
                        }
                        if (!isExcExecuted && sErr.Contains(excName.ToLowerInvariant()))
                        {
                            isExcExecuted = true;
                        }
                    }
                }
            }

            string excStatus = isExcExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

            string excEdgeLbl = $"On {excName} ({excCode})";
            string termLbl = $"🛑 FAILED ({excCode})";
            string termId = $"node__failed_term_{excCode}";

            string excLower = excName.ToLowerInvariant();
            if (isGlobalExc)
            {
                excEdgeLbl = "💥 500 Unhandled Exception\n(Catch-All / Runtime Crash)";
                termLbl = "🛑 500 INTERNAL SERVER ERROR";
                termId = "node__failed_term_500";
            }
            else if (excLower.Contains("rate"))
            {
                excEdgeLbl = "⚠️ 429 RateLimitExceeded\n(Quota > 60 req/min)";
                termLbl = "🛑 429 TOO MANY REQUESTS";
                termId = "node__failed_term_429";
            }
            else if (excLower.Contains("ip") || excLower.Contains("denied"))
            {
                excEdgeLbl = "🚫 403 IpAccessDenied\n(Blocked IP / Untrusted Subnet)";
                termLbl = "🛑 403 FORBIDDEN";
                termId = "node__failed_term_403";
            }
            else if (excLower.Contains("authorized"))
            {
                excEdgeLbl = "🚫 403 UserNotAuthorized\n(Forbidden Role/Permissions)";
                termLbl = "🛑 403 FORBIDDEN";
                termId = "node__failed_term_403";
            }
            else if (excLower.Contains("invalidcredentials") || excLower.Contains("credentials"))
            {
                excEdgeLbl = "🔒 401 InvalidCredentials\n(Invalid Username/Password)";
                termLbl = "🛑 401 UNAUTHORIZED";
                termId = "node__failed_term_401";
            }
            else if (excLower.Contains("authenticated"))
            {
                excEdgeLbl = "🔒 401 UserNotAuthenticated\n(Missing/Invalid Token)";
                termLbl = "🛑 401 UNAUTHORIZED";
                termId = "node__failed_term_401";
            }
            else if (excCode == 400 || excLower.Contains("badrequest") || excLower.Contains("validation") || excLower.Contains("malformed") || excLower.Contains("field"))
            {
                excEdgeLbl = $"🛑 400 {excName}\n(Bad Request / Schema Violation)";
                termLbl = "🛑 400 BAD REQUEST";
                termId = "node__failed_term_400";
            }
            else if (excCode == 413 || excLower.Contains("toolarge") || excLower.Contains("payload"))
            {
                excEdgeLbl = "🛑 413 PayloadTooLarge\n(Payload Size Exceeded)";
                termLbl = "🛑 413 PAYLOAD TOO LARGE";
                termId = "node__failed_term_413";
            }

            string originName = isGlobalExc
                ? $"Global Engine Safety Net ({excActName})"
                : (exc.IsMiddleware
                    ? $"Middleware Pipeline ({exc.SourceName ?? "Ingress Guards"})"
                    : (!string.IsNullOrWhiteSpace(exc.SourceStepId) ? $"Workflow Step ({exc.SourceStepId})" : "Workflow Exception Handler"));

            string desc = exc.IsMiddleware
                ? $"Invoked {(exc.IsAsync ? "asynchronously " : "")}by {exc.SourceName ?? "Ingress Guard"} upon {excName} ({excCode})."
                : (!string.IsNullOrWhiteSpace(exc.SourceStepId) ? $"Invoked {(exc.IsAsync ? "asynchronously " : "")}when step '{exc.SourceStepId}' encounters exception {excName} ({excCode})." : $"Invoked upon exception {excName} ({excCode}).");

            if (!string.IsNullOrWhiteSpace(exc.Message))
            {
                desc += " Policy: " + exc.Message;
            }

            var excNode = new Dictionary<string, object>
            {
                ["id"] = excNodeId,
                ["shape"] = "octopus-activity-node",
                ["label"] = "🚨 " + excActName,
                ["width"] = 220,
                ["height"] = 55,
                ["data"] = new Dictionary<string, object?>
                {
                    ["step_id"] = "exc_" + excActName.ToLowerInvariant(),
                    ["activity"] = excActName,
                    ["type"] = "exception_handler",
                    ["is_async"] = exc.IsAsync,
                    ["exception_type"] = excName,
                    ["status_code"] = excCode,
                    ["source_type"] = exc.IsMiddleware ? "Middleware Exception Pipeline" : "Workflow Step Exception Handler",
                    ["source_location"] = originName,
                    ["source_name"] = exc.SourceName,
                    ["source_step_id"] = exc.SourceStepId,
                    ["is_middleware"] = exc.IsMiddleware,
                    ["middleware_name"] = exc.IsMiddleware ? exc.SourceName : null,
                    ["title"] = "🚨 Exception Handler: " + excActName,
                    ["description"] = desc,
                    ["status"] = excStatus,
                    ["executed"] = isExcExecuted
                }
            };

            if (isExcExecuted)
            {
                excNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#10b981",
                        ["strokeWidth"] = 3,
                        ["fill"] = "#0f2e22",
                        ["filter"] = "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        ["opacity"] = 1.0
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#6ee7b7",
                        ["fontWeight"] = 600
                    }
                };
            }
            else if (hasLiveExecution)
            {
                excNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#334155",
                        ["strokeWidth"] = 1.5,
                        ["fill"] = "#111827",
                        ["opacity"] = 0.35
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#64748b",
                        ["opacity"] = 0.45
                    }
                };
            }
            nodes.Add(excNode);

            var strokeColor = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#1e293b" : "#f59e0b");
            var strokeWidth = isExcExecuted ? 2.5 : 1.5;
            var strokeDash = "5 5";
            var opacity = isExcExecuted ? 1.0 : (hasLiveExecution ? 0.25 : 0.8);
            var markerColor = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#334155" : "#f59e0b");

            var textFill = isExcExecuted ? "#f87171" : (hasLiveExecution ? "#475569" : "#fbbf24");
            var rectFill = isExcExecuted ? "#2d1215" : (hasLiveExecution ? "#090d16" : "#0f172a");
            var rectStroke = isExcExecuted ? "#ef4444" : (hasLiveExecution ? "#1e293b" : "#f59e0b");
            var labelOpacity = hasLiveExecution && !isExcExecuted ? 0.3 : 1.0;

            var excEdge = new Dictionary<string, object>
            {
                ["id"] = $"edge__{srcId}__to__{excNodeId}",
                ["source"] = srcId,
                ["target"] = excNodeId,
                ["attrs"] = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = strokeColor,
                        ["strokeWidth"] = strokeWidth,
                        ["strokeDasharray"] = strokeDash,
                        ["opacity"] = opacity,
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = markerColor }
                    }
                },
                ["labels"] = new List<Dictionary<string, object>>
                {
                    new()
                    {
                        ["position"] = 0.4,
                        ["attrs"] = new Dictionary<string, object>
                        {
                            ["text"] = new Dictionary<string, object>
                            {
                                ["text"] = excEdgeLbl,
                                ["fill"] = textFill,
                                ["fontSize"] = 10,
                                ["fontWeight"] = isExcExecuted ? "bold" : "normal",
                                ["opacity"] = labelOpacity
                            },
                            ["rect"] = new Dictionary<string, object>
                            {
                                ["fill"] = rectFill,
                                ["rx"] = 4,
                                ["ry"] = 4,
                                ["stroke"] = rectStroke,
                                ["strokeWidth"] = 1,
                                ["opacity"] = labelOpacity
                            }
                        }
                    }
                }
            };
            edges.Add(excEdge);

            // Terminal Failure Node
            var existingTermNode = nodes.FirstOrDefault(n => (string)n["id"] == termId);
            if (existingTermNode == null)
            {
                string failedTermStatus = isExcExecuted ? "FAILED" : (hasLiveExecution ? "SKIPPED" : "PENDING");
                int termWidth = Math.Max(160, termLbl.Length * 10 + 40);

                var termNode = new Dictionary<string, object>
                {
                    ["id"] = termId,
                    ["shape"] = "octopus-end-node",
                    ["label"] = termLbl,
                    ["width"] = termWidth,
                    ["height"] = 50,
                    ["data"] = new Dictionary<string, object>
                    {
                        ["step_id"] = "failed",
                        ["type"] = "end_failed",
                        ["title"] = "Failed: " + excName,
                        ["status"] = failedTermStatus,
                        ["executed"] = isExcExecuted
                    }
                };

                if (isExcExecuted)
                {
                    termNode["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["rx"] = 25,
                            ["ry"] = 25,
                            ["fill"] = "#450a0a",
                            ["stroke"] = "#ef4444",
                            ["strokeWidth"] = 3,
                            ["filter"] = "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                            ["opacity"] = 1.0
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#fca5a5",
                            ["fontSize"] = 13,
                            ["fontWeight"] = 700
                        }
                    };
                }
                else if (hasLiveExecution)
                {
                    termNode["attrs"] = new Dictionary<string, object>
                    {
                        ["body"] = new Dictionary<string, object>
                        {
                            ["rx"] = 25,
                            ["ry"] = 25,
                            ["fill"] = "#0f172a",
                            ["stroke"] = "#334155",
                            ["strokeWidth"] = 1.5,
                            ["opacity"] = 0.35
                        },
                        ["label"] = new Dictionary<string, object>
                        {
                            ["fill"] = "#64748b",
                            ["fontSize"] = 13,
                            ["fontWeight"] = 600,
                            ["opacity"] = 0.45
                        }
                    };
                }
                nodes.Add(termNode);
            }
            else if (isExcExecuted)
            {
                if (existingTermNode["data"] is Dictionary<string, object> tData)
                {
                    tData["status"] = "FAILED";
                    tData["executed"] = true;
                }
                existingTermNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["rx"] = 25,
                        ["ry"] = 25,
                        ["fill"] = "#450a0a",
                        ["stroke"] = "#ef4444",
                        ["strokeWidth"] = 3,
                        ["filter"] = "drop-shadow(0 0 14px rgba(239, 68, 68, 0.75))",
                        ["opacity"] = 1.0
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#fca5a5",
                        ["fontSize"] = 13,
                        ["fontWeight"] = 700
                    }
                };
            }

            var excToTermEdge = new Dictionary<string, object>
            {
                ["id"] = $"edge__{excNodeId}__to__{termId}",
                ["source"] = excNodeId,
                ["target"] = termId,
                ["attrs"] = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = strokeColor,
                        ["strokeWidth"] = strokeWidth,
                        ["strokeDasharray"] = "5 5",
                        ["opacity"] = opacity,
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = markerColor }
                    }
                }
            };
            edges.Add(excToTermEdge);
        }

        // 4. Egress Post-Execution Middleware Guards Node
        if (hasEgressGuards)
        {
            bool hasPredExec = finalHeads.Any(p => p.Executed);
            bool egressExecuted = state != null && state.Status == ExecutionStatus.COMPLETED && hasPredExec;
            string egressStatus = egressExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

            var postMwNamesList = postMiddlewares.Where(m => !string.IsNullOrWhiteSpace(m.Name)).Select(m => m.Name).ToList();

            var egressNode = new Dictionary<string, object>
            {
                ["id"] = "node__egress_guards",
                ["shape"] = "octopus-activity-node",
                ["label"] = "📊 Egress Middleware Guards",
                ["width"] = 220,
                ["height"] = 55,
                ["data"] = new Dictionary<string, object>
                {
                    ["step_id"] = "egress_guards",
                    ["type"] = "egress_guards",
                    ["title"] = "📊 Egress Middleware Guards",
                    ["description"] = "Post-execution response auditing, latency logging, and metrics dispatch executed following workflow pipeline completion.",
                    ["status"] = egressStatus,
                    ["executed"] = egressExecuted,
                    ["middlewares"] = postMwNamesList
                }
            };

            if (egressExecuted)
            {
                egressNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#10b981",
                        ["strokeWidth"] = 3,
                        ["fill"] = "#0f2e22",
                        ["filter"] = "drop-shadow(0 0 16px rgba(16, 185, 129, 0.75))",
                        ["opacity"] = 1.0
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = "#6ee7b7",
                        ["fontWeight"] = 600
                    }
                };
            }
            else
            {
                double opacityVal = hasLiveExecution ? 0.35 : 0.6;
                string labelFill = hasLiveExecution ? "#475569" : "#64748b";
                egressNode["attrs"] = new Dictionary<string, object>
                {
                    ["body"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#334155",
                        ["strokeWidth"] = 1.5,
                        ["fill"] = "#111827",
                        ["opacity"] = opacityVal
                    },
                    ["label"] = new Dictionary<string, object>
                    {
                        ["fill"] = labelFill,
                        ["fontWeight"] = 600
                    }
                };
            }
            nodes.Add(egressNode);

            foreach (var p in finalHeads)
            {
                Dictionary<string, object> edgeAttrs;
                if (p.Executed && egressExecuted)
                {
                    edgeAttrs = new Dictionary<string, object>
                    {
                        ["line"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#10b981",
                            ["strokeWidth"] = 3.5,
                            ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 10, ["height"] = 8, ["fill"] = "#10b981" },
                            ["filter"] = "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                            ["opacity"] = 1.0
                        }
                    };
                }
                else if (hasLiveExecution)
                {
                    edgeAttrs = new Dictionary<string, object>
                    {
                        ["line"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#334155",
                            ["strokeWidth"] = 1.5,
                            ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 6, ["height"] = 5, ["fill"] = "#334155" },
                            ["opacity"] = 0.35
                        }
                    };
                }
                else
                {
                    edgeAttrs = new Dictionary<string, object>
                    {
                        ["line"] = new Dictionary<string, object>
                        {
                            ["stroke"] = "#6366f1",
                            ["strokeWidth"] = 2,
                            ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = "#6366f1" }
                        }
                    };
                }

                edges.Add(new Dictionary<string, object>
                {
                    ["id"] = $"edge__{p.Id}__to__node__egress_guards",
                    ["source"] = p.Id,
                    ["target"] = "node__egress_guards",
                    ["attrs"] = edgeAttrs
                });
            }

            finalHeads = new List<PrevItem> { new("node__egress_guards", "", "#10b981", egressExecuted) };
        }

        // 5. End Node
        bool hasExecPred = finalHeads.Any(p => p.Executed);
        bool endExecuted = state != null && state.Status == ExecutionStatus.COMPLETED && hasExecPred;
        string endStatus = endExecuted ? "COMPLETED" : (hasLiveExecution ? "SKIPPED" : "PENDING");

        int? successStatusCode = null;
        if (selectedEp != null && selectedEp.StatusCode > 0)
        {
            successStatusCode = selectedEp.StatusCode;
        }
        else if (blueprint.Api != null && blueprint.Api.Count > 0)
        {
            successStatusCode = 200;
        }

        string endLabel = "✅ COMPLETED";
        string endTitle = "Workflow Completed";
        if (successStatusCode.HasValue)
        {
            endLabel = successStatusCode.Value switch
            {
                200 => "✅ 200 OK",
                201 => "✅ 201 CREATED",
                202 => "✅ 202 ACCEPTED",
                204 => "✅ 204 NO CONTENT",
                _ => $"✅ {successStatusCode.Value} OK"
            };
            endTitle = $"Completed: {successStatusCode.Value}";
        }

        int endWidth = Math.Max(140, endLabel.Length * 10 + 36);

        var endData = new Dictionary<string, object>
        {
            ["step_id"] = "end",
            ["type"] = "end",
            ["title"] = endTitle,
            ["status"] = endStatus,
            ["executed"] = endExecuted
        };
        if (successStatusCode.HasValue)
        {
            endData["status_code"] = successStatusCode.Value;
        }

        var endNode = new Dictionary<string, object>
        {
            ["id"] = "node__end",
            ["shape"] = "octopus-end-node",
            ["width"] = endWidth,
            ["height"] = 50,
            ["label"] = endLabel,
            ["data"] = endData
        };
        if (endExecuted)
        {
            endNode["attrs"] = new Dictionary<string, object>
            {
                ["body"] = new Dictionary<string, object>
                {
                    ["rx"] = 25,
                    ["ry"] = 25,
                    ["fill"] = "#064e3b",
                    ["stroke"] = "#10b981",
                    ["strokeWidth"] = 3,
                    ["filter"] = "drop-shadow(0 0 14px rgba(16, 185, 129, 0.75))",
                    ["opacity"] = 1.0
                },
                ["label"] = new Dictionary<string, object>
                {
                    ["fill"] = "#6ee7b7",
                    ["fontSize"] = 13,
                    ["fontWeight"] = 700
                }
            };
        }
        else
        {
            endNode["attrs"] = new Dictionary<string, object>
            {
                ["body"] = new Dictionary<string, object>
                {
                    ["rx"] = 25,
                    ["ry"] = 25,
                    ["fill"] = "#0f172a",
                    ["stroke"] = "#334155",
                    ["strokeWidth"] = 1.5,
                    ["opacity"] = hasLiveExecution ? 0.35 : 0.6
                },
                ["label"] = new Dictionary<string, object>
                {
                    ["fill"] = hasLiveExecution ? "#475569" : "#64748b",
                    ["fontSize"] = 13,
                    ["fontWeight"] = 600
                }
            };
        }
        nodes.Add(endNode);

        foreach (var p in finalHeads)
        {
            Dictionary<string, object> edgeAttrs;
            if (p.Executed && endStatus == "COMPLETED")
            {
                edgeAttrs = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#10b981",
                        ["strokeWidth"] = 3.5,
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 10, ["height"] = 8, ["fill"] = "#10b981" },
                        ["filter"] = "drop-shadow(0 0 8px rgba(16, 185, 129, 0.53))",
                        ["opacity"] = 1.0
                    }
                };
            }
            else if (hasLiveExecution)
            {
                edgeAttrs = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#1e293b",
                        ["strokeWidth"] = 1.5,
                        ["strokeDasharray"] = "4 4",
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 6, ["height"] = 4, ["fill"] = "#334155" },
                        ["opacity"] = 0.25
                    }
                };
            }
            else
            {
                edgeAttrs = new Dictionary<string, object>
                {
                    ["line"] = new Dictionary<string, object>
                    {
                        ["stroke"] = "#64748b",
                        ["strokeWidth"] = 2,
                        ["targetMarker"] = new Dictionary<string, object> { ["name"] = "block", ["width"] = 8, ["height"] = 6, ["fill"] = "#64748b" },
                        ["opacity"] = 1.0
                    }
                };
            }

            edges.Add(new Dictionary<string, object>
            {
                ["id"] = $"edge_{p.Id}_to_node__end",
                ["source"] = p.Id,
                ["target"] = "node__end",
                ["attrs"] = edgeAttrs
            });
        }

        string statusStr = state != null ? state.Status.ToString() : "IDLE";
        string overallStatusStr = state != null ? state.Status.ToString() : "";
        bool jobExists = string.IsNullOrWhiteSpace(jobId) || state != null;

        var apiList = new List<Dictionary<string, object>>();
        if (blueprint.Api != null)
        {
            foreach (var ep in blueprint.Api)
            {
                apiList.Add(new Dictionary<string, object>
                {
                    ["route"] = ep.Route,
                    ["method"] = ep.Method,
                    ["input_model"] = ep.InputModel ?? "",
                    ["output_model"] = ep.OutputModel ?? "",
                    ["status_code"] = ep.StatusCode,
                    ["summary"] = ep.Summary ?? "",
                    ["tags"] = ep.Tags
                });
            }
        }

        var result = new Dictionary<string, object>
        {
            ["workflow_name"] = workflowName,
            ["version"] = blueprint.Version,
            ["description"] = blueprint.Description,
            ["namespace"] = blueprint.Namespace,
            ["mode"] = blueprint.Mode,
            ["job_id"] = jobId ?? "",
            ["job_exists"] = jobExists,
            ["status"] = statusStr,
            ["overall_status"] = overallStatusStr,
            ["nodes"] = nodes,
            ["edges"] = edges,
            ["api"] = apiList,
            ["api_endpoints"] = apiList,
            ["routes"] = routesList,
            ["middlewares"] = blueprint.Middlewares,
            ["persistence"] = blueprint.Persistence
        };

        if (!string.IsNullOrWhiteSpace(selectedRoute))
        {
            result["selected_route"] = selectedRoute;
        }
        if (!string.IsNullOrWhiteSpace(selectedMethod))
        {
            result["selected_method"] = selectedMethod;
        }

        return result;
    }

    private static void CollectStepExceptions(List<WorkflowStep> steps, List<ExcConfigItem> excConfigs)
    {
        if (steps == null || steps.Count == 0) return;
        foreach (var step in steps)
        {
            var sId = !string.IsNullOrWhiteSpace(step.Id) ? step.Id : step.Activity ?? "";
            if (step.Exceptions != null)
            {
                foreach (var exc in step.Exceptions)
                {
                    if (!string.IsNullOrWhiteSpace(exc.Activity))
                    {
                        excConfigs.Add(new ExcConfigItem(
                            exc.Activity,
                            !string.IsNullOrWhiteSpace(exc.Exception) ? exc.Exception : "Exception",
                            exc.Code,
                            exc.IsAsync || exc.Async,
                            false,
                            sId,
                            sId,
                            exc.Message
                        ));
                    }
                }
            }
            var thenSteps = step.GetThenSteps();
            if (thenSteps != null && thenSteps.Count > 0)
            {
                CollectStepExceptions(thenSteps, excConfigs);
            }
            var elseSteps = step.GetElseSteps();
            if (elseSteps != null && elseSteps.Count > 0)
            {
                CollectStepExceptions(elseSteps, excConfigs);
            }
        }
    }

    private static bool IsPathExcluded(string route, List<string>? excludes)
    {
        if (string.IsNullOrWhiteSpace(route) || excludes == null || excludes.Count == 0)
        {
            return false;
        }
        var cleanRoute = route.TrimEnd('/');
        foreach (var raw in excludes)
        {
            if (string.IsNullOrWhiteSpace(raw)) continue;
            var pattern = raw.Trim();
            var cleanPattern = pattern.TrimEnd('/');

            if (string.Equals(cleanRoute, cleanPattern, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }

            if (pattern.EndsWith("*"))
            {
                var prefix = pattern[..^1];
                if (route.StartsWith(prefix, StringComparison.OrdinalIgnoreCase) || route.Contains(prefix, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }

            if (string.Equals(route, pattern, StringComparison.OrdinalIgnoreCase) ||
                route.EndsWith(pattern, StringComparison.OrdinalIgnoreCase) ||
                pattern.EndsWith(route, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }
        return false;
    }

    private static bool IsMiddlewareActiveForRoute(MiddlewareConfig m, string? route)
    {
        if (m == null) return false;
        if (string.IsNullOrWhiteSpace(route) || string.Equals(route, "default", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        if (IsPathExcluded(route, m.Excludes))
        {
            return false;
        }
        if (m.Includes != null && m.Includes.Count > 0)
        {
            foreach (var inc in m.Includes)
            {
                if (string.IsNullOrWhiteSpace(inc)) continue;
                var pattern = inc.Trim();
                if (pattern.EndsWith("*"))
                {
                    var prefix = pattern[..^1];
                    if (route.StartsWith(prefix, StringComparison.OrdinalIgnoreCase) || route.Contains(prefix, StringComparison.OrdinalIgnoreCase))
                    {
                        return true;
                    }
                }
                if (string.Equals(route, pattern, StringComparison.OrdinalIgnoreCase) ||
                    route.EndsWith(pattern, StringComparison.OrdinalIgnoreCase) ||
                    pattern.EndsWith(route, StringComparison.OrdinalIgnoreCase))
                {
                    return true;
                }
            }
            return false;
        }
        return true;
    }

    private object? ExtractRuntimeVariable(ExecutionState? state, string varName)
    {
        if (state == null || string.IsNullOrWhiteSpace(varName)) return null;
        var eval = new ConditionEvaluator();
        if (state.Data != null)
        {
            var val = eval.GetNestedField(state.Data, varName);
            if (val != null) return val;
        }
        if (state.Traces != null)
        {
            for (int i = state.Traces.Count - 1; i >= 0; i--)
            {
                var t = state.Traces[i];
                if (t.OutputPayload != null)
                {
                    var val = eval.GetNestedField(t.OutputPayload, varName);
                    if (val != null) return val;
                }
                if (t.InputPayload != null)
                {
                    var val = eval.GetNestedField(t.InputPayload, varName);
                    if (val != null) return val;
                }
            }
        }
        return null;
    }
}
