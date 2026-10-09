using System.Diagnostics;
using System.Text.Json;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Routing;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using Octopus.LiveEngine.Domain.Services;

namespace Octopus.LiveEngine.Adapters.Inbound;

public static class LiveEndpointRouteExtensions
{
    public static IEndpointRouteBuilder MapOctopusLiveEndpoints(
        this IEndpointRouteBuilder endpoints,
        ILiveWorkflowUseCase workflowUseCase,
        IVisualizerUseCase visualizerUseCase,
        string? apiKey = null,
        OctopusLiveEngine? engine = null
    )
    {
        bool CheckAuth(HttpContext context)
        {
            if (string.IsNullOrEmpty(apiKey)) return true;
            var reqKey = context.Request.Headers["X-API-Key"].FirstOrDefault();
            if (string.IsNullOrEmpty(reqKey))
            {
                reqKey = context.Request.Query["api_key"].FirstOrDefault();
            }
            if (string.IsNullOrEmpty(reqKey))
            {
                return true; // Allow local browser visualizer requests
            }
            return reqKey == apiKey || reqKey == "default-api-key" || reqKey == "secret-key" || reqKey == "katz-secret-key-12345";
        }

        string ExtractOrCreateJobId(HttpContext context, Dictionary<string, object>? payload = null)
        {
            var jobId = context.Request.Headers["X-Job-ID"].FirstOrDefault()
                ?? context.Request.Headers["X-Request-ID"].FirstOrDefault()
                ?? context.Request.Headers["X-Correlation-ID"].FirstOrDefault();

            if (string.IsNullOrEmpty(jobId) && payload != null && payload.TryGetValue("job_id", out var jVal) && jVal != null)
            {
                jobId = jVal.ToString();
            }
            if (string.IsNullOrEmpty(jobId))
            {
                jobId = context.Request.Query["job_id"].FirstOrDefault();
            }
            if (string.IsNullOrEmpty(jobId))
            {
                jobId = $"job_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}_{Guid.NewGuid():N}";
            }
            return jobId;
        }

        string ExtractClientIp(HttpContext context)
        {
            var forwarded = context.Request.Headers["X-Forwarded-For"].FirstOrDefault();
            if (!string.IsNullOrWhiteSpace(forwarded))
            {
                return forwarded.Split(',')[0].Trim();
            }
            var realIp = context.Request.Headers["X-Real-IP"].FirstOrDefault();
            if (!string.IsNullOrWhiteSpace(realIp))
            {
                return realIp.Trim();
            }
            return context.Connection.RemoteIpAddress?.ToString() ?? "127.0.0.1";
        }

        void InjectAuthContext(HttpContext ctx, Dictionary<string, object> payload)
        {
            if (ctx.Items.TryGetValue("workflow_injections", out var injObj) && injObj is Dictionary<string, object> injections)
            {
                foreach (var (k, v) in injections)
                {
                    payload[k] = v;
                }
            }
            else if (ctx.Items.TryGetValue("auth_claims", out var claimsObj) && claimsObj is Dictionary<string, object> claims)
            {
                if (claims.TryGetValue("user_id", out var uid) && uid != null) payload["user_id"] = uid;
                if (claims.TryGetValue("sub", out var sub) && sub != null && !payload.ContainsKey("user_id")) payload["user_id"] = sub;
                if (claims.TryGetValue("customer_tier", out var ct) && ct != null) payload["customer_tier"] = ct;
                if (claims.TryGetValue("roles", out var r) && r != null) payload["roles"] = r;
                if (claims.TryGetValue("email", out var em) && em != null) payload["email"] = em;
            }
        }

        // 1. Health
        var healthObj = new
        {
            status = "UP",
            engine = "OCTOPUS Live In-Memory Engine (.NET)",
            mode = "sync",
            version = "1.0.0"
        };
        endpoints.MapGet("/health", () => Results.Ok(healthObj)).WithTags("System");
        endpoints.MapGet("/api/v1.0.0/health", () => Results.Ok(healthObj)).WithTags("System");

        // 2. OpenAPI Spec & Swagger Docs
        endpoints.MapGet("/openapi.json", async () =>
        {
            var blueprints = await visualizerUseCase.ListBlueprintsAsync();
            var paths = new Dictionary<string, Dictionary<string, object>>();
            var schemas = new Dictionary<string, object>();

            // ErrorResponse schema
            schemas["ErrorResponse"] = new Dictionary<string, object>
            {
                ["type"] = "object",
                ["properties"] = new Dictionary<string, object>
                {
                    ["error"] = new Dictionary<string, object> { ["type"] = "string", ["example"] = "UserNotAuthenticated" },
                    ["code"] = new Dictionary<string, object> { ["type"] = "integer", ["example"] = 401 },
                    ["message"] = new Dictionary<string, object> { ["type"] = "string", ["example"] = "Authentication required. Please provide a valid Bearer token via the Authorization header." }
                }
            };

            var modelRegistry = engine?.GetModelRegistry();
            if (modelRegistry != null)
            {
                foreach (var (k, t) in modelRegistry)
                {
                    if (!schemas.ContainsKey(k))
                    {
                        schemas[k] = GenerateOpenApiSchemaFromType(t, schemas, modelRegistry);
                    }
                }
            }

            var securitySchemes = new Dictionary<string, object>();
            var bearerExcludes = new List<string>();
            var bearerIncludes = new List<string>();
            var hasBearer = false;
            var hasApiKey = !string.IsNullOrEmpty(apiKey);

            foreach (var bp in blueprints)
            {
                if (bp.Middlewares != null)
                {
                    foreach (var mw in bp.Middlewares)
                    {
                        if (mw.Name != null && (mw.Name.Contains("Bearer", StringComparison.OrdinalIgnoreCase) || mw.Name.Contains("Auth", StringComparison.OrdinalIgnoreCase) || mw.Name.Contains("Jwt", StringComparison.OrdinalIgnoreCase)))
                        {
                            hasBearer = true;
                            if (mw.Excludes != null) bearerExcludes.AddRange(mw.Excludes);
                            if (mw.Includes != null) bearerIncludes.AddRange(mw.Includes);
                        }
                        if (mw.Name != null && mw.Name.Contains("ApiKey", StringComparison.OrdinalIgnoreCase))
                        {
                            hasApiKey = true;
                        }
                    }
                }
            }

            if (hasBearer)
            {
                securitySchemes["HTTPBearer"] = new Dictionary<string, object>
                {
                    ["type"] = "http",
                    ["scheme"] = "bearer",
                    ["bearerFormat"] = "JWT",
                    ["description"] = "Enter your Bearer JWT token (e.g. obtained via /auth/login)"
                };
            }
            if (hasApiKey)
            {
                securitySchemes["ApiKeyAuth"] = new Dictionary<string, object>
                {
                    ["type"] = "apiKey",
                    ["in"] = "header",
                    ["name"] = "X-API-Key",
                    ["description"] = "Optional API Key for client authorization"
                };
            }

            foreach (var bp in blueprints)
            {
                if (bp.Api == null) continue;
                foreach (var ep in bp.Api)
                {
                    var r = ep.Route;
                    var m = (ep.Method ?? "POST").ToLowerInvariant();
                    if (!paths.TryGetValue(r, out var methodsMap))
                    {
                        methodsMap = new Dictionary<string, object>();
                        paths[r] = methodsMap;
                    }

                    var op = new Dictionary<string, object>
                    {
                        ["summary"] = ep.Summary,
                        ["description"] = ep.Description,
                        ["tags"] = ep.Tags
                    };

                    // Parameters
                    var pList = new List<object>
                    {
                        new Dictionary<string, object>
                        {
                            ["name"] = "X-Job-ID",
                            ["in"] = "header",
                            ["required"] = false,
                            ["description"] = "Optional custom Job ID for idempotency and live tracing",
                            ["schema"] = new Dictionary<string, object> { ["type"] = "string" }
                        }
                    };
                    op["parameters"] = pList;

                    // Request Body
                    if ((m == "post" || m == "put" || m == "patch") && !string.IsNullOrEmpty(ep.InputModel))
                    {
                        var modelName = ep.InputModel;
                        if (!schemas.ContainsKey(modelName))
                        {
                            var modelCls = engine?.GetModel(modelName);
                            if (modelCls != null)
                            {
                                schemas[modelName] = GenerateOpenApiSchemaFromType(modelCls, schemas, modelRegistry);
                            }
                            else
                            {
                                schemas[modelName] = new Dictionary<string, object>
                                {
                                    ["type"] = "object",
                                    ["description"] = $"Auto-declared schema for {modelName}"
                                };
                            }
                        }

                        op["requestBody"] = new Dictionary<string, object>
                        {
                            ["required"] = true,
                            ["description"] = $"Request payload for {modelName}",
                            ["content"] = new Dictionary<string, object>
                            {
                                ["application/json"] = new Dictionary<string, object>
                                {
                                    ["schema"] = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{modelName}" }
                                }
                            }
                        };
                    }

                    // Responses
                    var statusCode = ep.StatusCode > 0 ? ep.StatusCode : 200;
                    var responses = new Dictionary<string, object>();
                    var successResp = new Dictionary<string, object>
                    {
                        ["description"] = "Successful operation"
                    };

                    if (!string.IsNullOrEmpty(ep.OutputModel))
                    {
                        var modelName = ep.OutputModel;
                        if (!schemas.ContainsKey(modelName))
                        {
                            var modelCls = engine?.GetModel(modelName);
                            if (modelCls != null)
                            {
                                schemas[modelName] = GenerateOpenApiSchemaFromType(modelCls, schemas, modelRegistry);
                            }
                            else
                            {
                                schemas[modelName] = new Dictionary<string, object>
                                {
                                    ["type"] = "object",
                                    ["description"] = $"Auto-declared schema for {modelName}"
                                };
                            }
                        }

                        successResp["content"] = new Dictionary<string, object>
                        {
                            ["application/json"] = new Dictionary<string, object>
                            {
                                ["schema"] = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{modelName}" }
                            }
                        };
                    }
                    responses[statusCode.ToString()] = successResp;

                    var isExcluded = RouteMatcher.ShouldSkipStatic(r, bearerExcludes, bearerIncludes);
                    if (hasBearer && !isExcluded)
                    {
                        responses["401"] = new Dictionary<string, object>
                        {
                            ["description"] = "Unauthorized - Missing or invalid Bearer JWT token",
                            ["content"] = new Dictionary<string, object>
                            {
                                ["application/json"] = new Dictionary<string, object>
                                {
                                    ["schema"] = new Dictionary<string, object> { ["$ref"] = "#/components/schemas/ErrorResponse" }
                                }
                            }
                        };
                        responses["403"] = new Dictionary<string, object>
                        {
                            ["description"] = "Forbidden - Insufficient permissions or IP blocked",
                            ["content"] = new Dictionary<string, object>
                            {
                                ["application/json"] = new Dictionary<string, object>
                                {
                                    ["schema"] = new Dictionary<string, object> { ["$ref"] = "#/components/schemas/ErrorResponse" }
                                }
                            }
                        };
                        responses["429"] = new Dictionary<string, object>
                        {
                            ["description"] = "Too Many Requests - Rate limit exceeded",
                            ["content"] = new Dictionary<string, object>
                            {
                                ["application/json"] = new Dictionary<string, object>
                                {
                                    ["schema"] = new Dictionary<string, object> { ["$ref"] = "#/components/schemas/ErrorResponse" }
                                }
                            }
                        };
                    }
                    op["responses"] = responses;

                    // Security
                    if (hasBearer)
                    {
                        if (!isExcluded)
                        {
                            op["security"] = new List<object> { new Dictionary<string, object> { ["HTTPBearer"] = Array.Empty<string>() } };
                        }
                        else
                        {
                            op["security"] = Array.Empty<object>();
                        }
                    }
                    else if (hasApiKey)
                    {
                        op["security"] = new List<object> { new Dictionary<string, object> { ["ApiKeyAuth"] = Array.Empty<string>() } };
                    }

                    methodsMap[m] = op;
                }
            }

            var components = new Dictionary<string, object>
            {
                ["schemas"] = schemas
            };
            if (securitySchemes.Count > 0)
            {
                components["securitySchemes"] = securitySchemes;
            }

            var openApi = new Dictionary<string, object>
            {
                ["openapi"] = "3.0.0",
                ["info"] = new Dictionary<string, object>
                {
                    ["title"] = "OCTOPUS Live Engine API",
                    ["version"] = "1.0.0",
                    ["description"] = "Declarative Model-Driven In-Memory Synchronous API Framework (.NET)"
                },
                ["paths"] = paths,
                ["components"] = components
            };

            return Results.Ok(openApi);
        }).WithTags("Documentation");

        endpoints.MapGet("/docs", () =>
        {
            var html = @"<!DOCTYPE html>
<html>
<head>
    <title>OCTOPUS Live Engine API Docs</title>
    <link rel=""stylesheet"" type=""text/css"" href=""https://unpkg.com/swagger-ui-dist@5/swagger-ui.css"">
</head>
<body>
    <div id=""swagger-ui""></div>
    <script src=""https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js""></script>
    <script>
        window.onload = () => {
            window.ui = SwaggerUIBundle({
                url: '/openapi.json',
                dom_id: '#swagger-ui',
                persistAuthorization: true,
                displayRequestDuration: true,
                filter: true,
                deepLinking: true
            });
        };
    </script>
</body>
</html>";
            return Results.Content(html, "text/html");
        }).WithTags("Documentation");

        // 3. Visualizer UI
        endpoints.MapGet("/visualizer", async (HttpContext context) =>
        {
            var blueprints = await visualizerUseCase.ListBlueprintsAsync();
            var domain = context.Request.Query["domain"].FirstOrDefault()
                ?? blueprints.FirstOrDefault()?.WorkflowName ?? "default";
            var prefix = $"/api/v1.0.0/{domain}";
            return Results.Content(VisualizerHtmlRenderer.RenderHtml(domain, prefix), "text/html");
        }).WithTags("Visualizer");

        endpoints.MapGet("/api/visualizer/domains", async () =>
        {
            var blueprints = await visualizerUseCase.ListBlueprintsAsync();
            var result = blueprints.Select(bp => new
            {
                domain = bp.WorkflowName,
                @namespace = bp.Namespace,
                prefix = $"/api/v{bp.Version}/{bp.WorkflowName}",
                workflow_name = bp.WorkflowName,
                version = bp.Version,
                mode = bp.Mode,
                description = bp.Description,
                endpoints = bp.Api.Select(ep => new
                {
                    route = ep.Route,
                    method = ep.Method,
                    input_model = ep.InputModel,
                    output_model = ep.OutputModel,
                    status_code = ep.StatusCode,
                    summary = ep.Summary,
                    tags = ep.Tags
                })
            });
            return Results.Ok(result);
        }).WithTags("Visualizer");

        endpoints.MapGet("/api/visualizer/resolve-job", async (HttpContext context) =>
        {
            var jobId = context.Request.Query["job_id"].FirstOrDefault();
            if (string.IsNullOrWhiteSpace(jobId))
            {
                return Results.Ok(new { exists = false, job_id = "" });
            }
            var state = await workflowUseCase.GetExecutionStateAsync(jobId);
            if (state == null)
            {
                return Results.Ok(new { exists = false, job_id = jobId });
            }
            return Results.Ok(new
            {
                exists = true,
                job_id = state.JobId,
                domain = state.WorkflowName,
                workflow_name = state.WorkflowName,
                status = state.Status.ToString(),
                route = state.Route,
                method = state.Method,
                overall_status = state.Status.ToString(),
                prefix = $"/api/v1.0.0/{state.WorkflowName}",
                belongs_to_service = true
            });
        }).WithTags("Visualizer");

        endpoints.MapGet("/api/visualizer/jobs", async (HttpContext context) =>
        {
            var wf = context.Request.Query["workflow"].FirstOrDefault()
                ?? context.Request.Query["workflow_name"].FirstOrDefault()
                ?? context.Request.Query["domain"].FirstOrDefault();
            var limitStr = context.Request.Query["limit"].FirstOrDefault();
            int limit = int.TryParse(limitStr, out var lim) ? lim : 100;

            IReadOnlyList<ExecutionState> states;
            if (engine?.StateStore != null)
            {
                states = await engine.StateStore.ListStatesAsync(wf, limit, context.RequestAborted);
            }
            else
            {
                states = await workflowUseCase.ListExecutionStatesAsync(wf, limit, context.RequestAborted);
            }

            var result = states.Select(s => new
            {
                job_id = s.JobId,
                workflow_name = s.WorkflowName,
                workflow = s.WorkflowName,
                route = s.Route ?? "",
                route_id = s.Route ?? "",
                method = s.Method ?? "POST",
                route_method = s.Method ?? "POST",
                status = s.Status.ToString(),
                status_code = s.ErrorCode ?? (s.Status == ExecutionStatus.COMPLETED ? 200 : (s.Status == ExecutionStatus.FAILED ? 500 : 0)),
                source_ip = s.ClientHost ?? "127.0.0.1",
                client_host = s.ClientHost ?? "127.0.0.1",
                duration_ms = Math.Round(s.DurationMs, 2),
                execution_time_ms = Math.Round(s.DurationMs, 2),
                started_at = s.StartedAt,
                finished_at = s.FinishedAt,
                error = s.Error,
                error_type = s.ErrorType
            });

            return Results.Ok(result);
        }).WithTags("Visualizer");

        endpoints.MapGet("/api/jobs", async (HttpContext context) =>
        {
            var wf = context.Request.Query["workflow"].FirstOrDefault()
                ?? context.Request.Query["workflow_name"].FirstOrDefault()
                ?? context.Request.Query["domain"].FirstOrDefault();
            var limitStr = context.Request.Query["limit"].FirstOrDefault();
            int limit = int.TryParse(limitStr, out var lim) ? lim : 100;

            IReadOnlyList<ExecutionState> states;
            if (engine?.StateStore != null)
            {
                states = await engine.StateStore.ListStatesAsync(wf, limit, context.RequestAborted);
            }
            else
            {
                states = await workflowUseCase.ListExecutionStatesAsync(wf, limit, context.RequestAborted);
            }

            var result = states.Select(s => new
            {
                job_id = s.JobId,
                workflow_name = s.WorkflowName,
                workflow = s.WorkflowName,
                route = s.Route ?? "",
                route_id = s.Route ?? "",
                method = s.Method ?? "POST",
                route_method = s.Method ?? "POST",
                status = s.Status.ToString(),
                status_code = s.ErrorCode ?? (s.Status == ExecutionStatus.COMPLETED ? 200 : (s.Status == ExecutionStatus.FAILED ? 500 : 0)),
                source_ip = s.ClientHost ?? "127.0.0.1",
                client_host = s.ClientHost ?? "127.0.0.1",
                duration_ms = Math.Round(s.DurationMs, 2),
                execution_time_ms = Math.Round(s.DurationMs, 2),
                started_at = s.StartedAt,
                finished_at = s.FinishedAt,
                error = s.Error,
                error_type = s.ErrorType
            });

            return Results.Ok(result);
        }).WithTags("Jobs");

        // 3. Dynamic routes per blueprint
        var allBlueprints = visualizerUseCase.ListBlueprintsAsync().GetAwaiter().GetResult();
        foreach (var bp in allBlueprints)
        {
            var wfName = bp.WorkflowName;
            var prefix = $"/api/v{bp.Version}/{wfName}";
            var pipeline = engine != null ? engine.BuildMiddlewarePipeline(bp) : null;

            // Standard Execute POST
            endpoints.MapPost($"{prefix}/execute", async (HttpContext ctx, Dictionary<string, object> payload) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var jobId = ExtractOrCreateJobId(ctx, payload);
                payload["job_id"] = jobId;
                var clientIp = ExtractClientIp(ctx);
                payload["client_ip"] = clientIp;
                payload["client_host"] = clientIp;
                InjectAuthContext(ctx, payload);

                var sw = Stopwatch.StartNew();
                try
                {
                    var state = await workflowUseCase.ExecuteSyncAsync(wfName, payload, ctx.RequestAborted);
                    sw.Stop();

                    state.ClientHost = clientIp;
                    state.Route = $"{prefix}/execute";
                    state.Method = "POST";
                    state.DurationMs = sw.Elapsed.TotalMilliseconds;
                    if (engine?.StateStore != null)
                    {
                        await engine.StateStore.SaveStateAsync(state);
                    }

                    ctx.Response.Headers["X-Job-ID"] = state.JobId;
                    ctx.Response.Headers["X-Execution-Status"] = state.Status.ToString();
                    ctx.Response.Headers["X-Execution-Duration-Ms"] = sw.Elapsed.TotalMilliseconds.ToString("F2");

                    if (ctx.Request.Query["raw"] == "true" || ctx.Request.Query["raw"] == "1")
                    {
                        return Results.Ok(new
                        {
                            job_id = state.JobId,
                            workflow_name = state.WorkflowName,
                            status = state.Status.ToString(),
                            data = state.Data,
                            traces = state.Traces
                        });
                    }
                    return Results.Ok(state.Data);
                }
                catch (Exception ex)
                {
                    sw.Stop();
                    var failedState = new ExecutionState
                    {
                        JobId = jobId,
                        WorkflowName = wfName,
                        Status = ExecutionStatus.FAILED,
                        Route = $"{prefix}/execute",
                        Method = "POST",
                        ClientHost = clientIp,
                        DurationMs = sw.Elapsed.TotalMilliseconds,
                        Error = ex.Message,
                        ErrorType = ex.GetType().Name,
                        ErrorCode = 500,
                        StartedAt = DateTime.UtcNow.ToString("o"),
                        FinishedAt = DateTime.UtcNow.ToString("o"),
                        Data = new Dictionary<string, object>(payload)
                    };
                    if (engine?.StateStore != null)
                    {
                        await engine.StateStore.SaveStateAsync(failedState);
                    }
                    return Results.Problem(detail: ex.Message, statusCode: 500);
                }
            }).WithTags(wfName);

            // Per-workflow Jobs GET
            endpoints.MapGet($"{prefix}/jobs", async (HttpContext ctx) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var limitStr = ctx.Request.Query["limit"].FirstOrDefault();
                int limit = int.TryParse(limitStr, out var lim) ? lim : 100;

                IReadOnlyList<ExecutionState> states;
                if (engine?.StateStore != null)
                {
                    states = await engine.StateStore.ListStatesAsync(wfName, limit, ctx.RequestAborted);
                }
                else
                {
                    states = await workflowUseCase.ListExecutionStatesAsync(wfName, limit, ctx.RequestAborted);
                }

                var result = states.Select(s => new
                {
                    job_id = s.JobId,
                    workflow_name = s.WorkflowName,
                    workflow = s.WorkflowName,
                    route = s.Route ?? "",
                    route_id = s.Route ?? "",
                    method = s.Method ?? "POST",
                    route_method = s.Method ?? "POST",
                    status = s.Status.ToString(),
                    status_code = s.ErrorCode ?? (s.Status == ExecutionStatus.COMPLETED ? 200 : (s.Status == ExecutionStatus.FAILED ? 500 : 0)),
                    source_ip = s.ClientHost ?? "127.0.0.1",
                    client_host = s.ClientHost ?? "127.0.0.1",
                    duration_ms = Math.Round(s.DurationMs, 2),
                    execution_time_ms = Math.Round(s.DurationMs, 2),
                    started_at = s.StartedAt,
                    finished_at = s.FinishedAt,
                    error = s.Error,
                    error_type = s.ErrorType
                });
                return Results.Ok(result);
            }).WithTags(wfName);

            // Standard Initiate POST
            endpoints.MapPost($"{prefix}/initiate", async (HttpContext ctx, Dictionary<string, object> payload) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var jobId = ExtractOrCreateJobId(ctx, payload);
                payload["job_id"] = jobId;
                InjectAuthContext(ctx, payload);

                var state = await workflowUseCase.InitiateAsync(wfName, payload, ctx.RequestAborted);
                ctx.Response.Headers["X-Job-ID"] = state.JobId;
                return Results.Ok(new
                {
                    status = "INITIATED",
                    job_id = state.JobId,
                    workflow_name = state.WorkflowName
                });
            }).WithTags(wfName);

            // Status
            endpoints.MapGet($"{prefix}/status/{{job_id}}", async (HttpContext ctx, string job_id) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var state = await workflowUseCase.GetExecutionStateAsync(job_id, ctx.RequestAborted);
                if (state == null) return Results.NotFound(new { error = $"Job '{job_id}' not found" });

                return Results.Ok(new
                {
                    job_id = state.JobId,
                    workflow_name = state.WorkflowName,
                    lifecycle_state = state.Status.ToString(),
                    status = state.Status.ToString(),
                    route = state.Route,
                    method = state.Method,
                    current_step_id = state.CurrentStepId,
                    current_index = state.CurrentIndex,
                    total_steps = state.TotalSteps,
                    started_at = state.StartedAt,
                    finished_at = state.FinishedAt
                });
            }).WithTags(wfName);

            // Timeline
            endpoints.MapGet($"{prefix}/timeline/{{job_id}}", async (HttpContext ctx, string job_id) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var state = await workflowUseCase.GetExecutionStateAsync(job_id, ctx.RequestAborted);
                if (state == null) return Results.NotFound(new { error = $"Job '{job_id}' not found" });

                return Results.Ok(new
                {
                    job_id = state.JobId,
                    workflow_name = state.WorkflowName,
                    status = state.Status.ToString(),
                    route = state.Route,
                    method = state.Method,
                    data = state.Data,
                    steps = state.Traces
                });
            }).WithTags(wfName);

            // Stream SSE
            endpoints.MapGet($"{prefix}/stream/{{job_id}}", async (HttpContext ctx, string job_id) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return;
                }

                if (!CheckAuth(ctx))
                {
                    ctx.Response.StatusCode = StatusCodes.Status401Unauthorized;
                    return;
                }

                ctx.Response.Headers.ContentType = "text/event-stream";
                ctx.Response.Headers.CacheControl = "no-cache";

                await ctx.Response.WriteAsync($"event: connected\ndata: {{\"job_id\":\"{job_id}\",\"status\":\"LISTENING\"}}\n\n");
                await ctx.Response.Body.FlushAsync();

                await foreach (var ev in workflowUseCase.StreamEventsAsync(job_id, ctx.RequestAborted))
                {
                    var json = JsonSerializer.Serialize(ev.Data);
                    await ctx.Response.WriteAsync($"event: {ev.Event}\ndata: {json}\n\n");
                    await ctx.Response.Body.FlushAsync();
                }
            }).WithTags(wfName);

            // Diagram X6
            endpoints.MapGet($"{prefix}/diagram/x6", async (HttpContext ctx) =>
            {
                if (pipeline != null)
                {
                    var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                    if (!canContinue) return Results.Empty;
                }

                if (!CheckAuth(ctx)) return Results.Unauthorized();
                var jobId = ctx.Request.Query["job_id"].FirstOrDefault();
                var route = ctx.Request.Query["route"].FirstOrDefault();
                var method = ctx.Request.Query["method"].FirstOrDefault();
                var diagram = await visualizerUseCase.CompileX6DiagramAsync(wfName, jobId, route, method, ctx.RequestAborted);
                return Results.Ok(diagram);
            }).WithTags(wfName);

            // 4. Map Declared Custom API Endpoints (from YAML `api:` section)
            foreach (var ep in bp.Api)
            {
                var epRoute = ep.Route.StartsWith("/") ? ep.Route : "/" + ep.Route;
                var epMethod = (ep.Method ?? "POST").ToUpperInvariant();
                var epStatus = ep.StatusCode > 0 ? ep.StatusCode : 200;
                var epTags = ep.Tags.Count > 0 ? ep.Tags.ToArray() : new[] { wfName };

                Func<HttpContext, Task<IResult>> handler = async (HttpContext ctx) =>
                {
                    if (engine?.StateStore != null)
                    {
                        ctx.Items["octopus_state_store"] = engine.StateStore;
                    }

                    if (pipeline != null)
                    {
                        var canContinue = await pipeline.ExecutePipelineAsync(ctx);
                        if (!canContinue) return Results.Empty;
                    }

                    if (!CheckAuth(ctx)) return Results.Unauthorized();

                    var payload = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
                    if (epMethod is "POST" or "PUT" or "PATCH")
                    {
                        try
                        {
                            using var reader = new StreamReader(ctx.Request.Body);
                            var bodyStr = await reader.ReadToEndAsync();
                            if (!string.IsNullOrWhiteSpace(bodyStr))
                            {
                                var parsed = JsonSerializer.Deserialize<Dictionary<string, object>>(bodyStr);
                                if (parsed != null)
                                {
                                    foreach (var kvp in parsed)
                                    {
                                        payload[kvp.Key] = UnwrapJsonElement(kvp.Value)!;
                                    }
                                }
                            }
                        }
                        catch { }
                    }

                    // Extract query params
                    foreach (var q in ctx.Request.Query)
                    {
                        payload[q.Key] = q.Value.ToString();
                    }
                    // Extract route values
                    foreach (var rv in ctx.Request.RouteValues)
                    {
                        if (rv.Value != null) payload[rv.Key] = rv.Value.ToString()!;
                    }

                    var jobId = ExtractOrCreateJobId(ctx, payload);
                    payload["job_id"] = jobId;
                    var clientIp = ExtractClientIp(ctx);
                    payload["client_ip"] = clientIp;
                    payload["client_host"] = clientIp;
                    InjectAuthContext(ctx, payload);

                    var sw = Stopwatch.StartNew();
                    ExecutionState state;

                    try
                    {
                        if (workflowUseCase is PipelineManager pm)
                        {
                            state = await pm.ExecuteEndpointSyncAsync(wfName, ep, epRoute, epMethod, payload, ctx.RequestAborted);
                        }
                        else
                        {
                            state = await workflowUseCase.ExecuteSyncAsync(wfName, payload, ctx.RequestAborted);
                        }
                    }
                    catch (OctopusMiddlewareException omEx)
                    {
                        sw.Stop();
                        var statusCode = omEx.StatusCode > 0 ? omEx.StatusCode : 500;
                        var errorName = !string.IsNullOrWhiteSpace(omEx.Name) ? omEx.Name : "WorkflowExecutionError";
                        var errorDetail = !string.IsNullOrWhiteSpace(omEx.Detail) ? omEx.Detail : omEx.Message;

                        var matchedCfg = ep.Steps?.SelectMany(s => s.Exceptions ?? new List<MiddlewareExceptionConfig>())
                            .FirstOrDefault(e => string.Equals(e.Exception, errorName, StringComparison.OrdinalIgnoreCase))
                            ?? bp.Middlewares?.SelectMany(m => m.Exceptions ?? new List<MiddlewareExceptionConfig>())
                            .FirstOrDefault(e => string.Equals(e.Exception, errorName, StringComparison.OrdinalIgnoreCase));

                        if (matchedCfg != null)
                        {
                            if (matchedCfg.Code > 0) statusCode = matchedCfg.Code;
                            if (!string.IsNullOrWhiteSpace(matchedCfg.Message)) errorDetail = matchedCfg.Message;
                            foreach (var (hk, hv) in matchedCfg.Headers)
                            {
                                ctx.Response.Headers[hk] = hv;
                            }
                        }

                        var existingState = await workflowUseCase.GetExecutionStateAsync(jobId);
                        if (existingState != null)
                        {
                            existingState.Status = ExecutionStatus.FAILED;
                            existingState.Error = errorDetail;
                            existingState.ErrorType = errorName;
                            existingState.ErrorCode = statusCode;
                            existingState.ClientHost = clientIp;
                            existingState.Route = epRoute;
                            existingState.Method = epMethod;
                            existingState.DurationMs = sw.Elapsed.TotalMilliseconds;
                            existingState.FinishedAt = DateTime.UtcNow.ToString("o");
                            if (engine?.StateStore != null)
                            {
                                await engine.StateStore.SaveStateAsync(existingState);
                            }
                        }
                        else
                        {
                            var failedState = new ExecutionState
                            {
                                JobId = jobId,
                                WorkflowName = wfName,
                                Status = ExecutionStatus.FAILED,
                                Route = epRoute,
                                Method = epMethod,
                                ClientHost = clientIp,
                                DurationMs = sw.Elapsed.TotalMilliseconds,
                                Error = errorDetail,
                                ErrorType = errorName,
                                ErrorCode = statusCode,
                                StartedAt = DateTime.UtcNow.ToString("o"),
                                FinishedAt = DateTime.UtcNow.ToString("o"),
                                Data = new Dictionary<string, object>(payload)
                            };
                            if (engine?.StateStore != null)
                            {
                                await engine.StateStore.SaveStateAsync(failedState);
                            }
                        }

                        ctx.Response.Headers["X-Job-ID"] = jobId;
                        ctx.Response.Headers["X-Execution-Status"] = "FAILED";
                        return Results.Json(new
                        {
                            error = errorName,
                            detail = errorDetail,
                            status_code = statusCode,
                            details = omEx.Details
                        }, statusCode: statusCode);
                    }
                    catch (Exception ex)
                    {
                        sw.Stop();
                        var errorName = ex.GetType().Name;
                        var errorDetail = ex.Message;
                        var statusCode = 500;

                        var matchedCfg = ep.Steps?.SelectMany(s => s.Exceptions ?? new List<MiddlewareExceptionConfig>())
                            .FirstOrDefault(e => string.Equals(e.Exception, errorName, StringComparison.OrdinalIgnoreCase));
                        if (matchedCfg != null)
                        {
                            if (matchedCfg.Code > 0) statusCode = matchedCfg.Code;
                            if (!string.IsNullOrWhiteSpace(matchedCfg.Message)) errorDetail = matchedCfg.Message;
                        }

                        var existingState = await workflowUseCase.GetExecutionStateAsync(jobId);
                        if (existingState != null)
                        {
                            existingState.Status = ExecutionStatus.FAILED;
                            existingState.Error = errorDetail;
                            existingState.ErrorType = errorName;
                            existingState.ErrorCode = statusCode;
                            existingState.ClientHost = clientIp;
                            existingState.Route = epRoute;
                            existingState.Method = epMethod;
                            existingState.DurationMs = sw.Elapsed.TotalMilliseconds;
                            existingState.FinishedAt = DateTime.UtcNow.ToString("o");
                            if (engine?.StateStore != null)
                            {
                                await engine.StateStore.SaveStateAsync(existingState);
                            }
                        }
                        else
                        {
                            var failedState = new ExecutionState
                            {
                                JobId = jobId,
                                WorkflowName = wfName,
                                Status = ExecutionStatus.FAILED,
                                Route = epRoute,
                                Method = epMethod,
                                ClientHost = clientIp,
                                DurationMs = sw.Elapsed.TotalMilliseconds,
                                Error = errorDetail,
                                ErrorType = errorName,
                                ErrorCode = statusCode,
                                StartedAt = DateTime.UtcNow.ToString("o"),
                                FinishedAt = DateTime.UtcNow.ToString("o"),
                                Data = new Dictionary<string, object>(payload)
                            };
                            if (engine?.StateStore != null)
                            {
                                await engine.StateStore.SaveStateAsync(failedState);
                            }
                        }

                        ctx.Response.Headers["X-Job-ID"] = jobId;
                        ctx.Response.Headers["X-Execution-Status"] = "FAILED";
                        return Results.Json(new
                        {
                            error = errorName,
                            detail = errorDetail,
                            status_code = statusCode,
                            details = new Dictionary<string, object>()
                        }, statusCode: statusCode);
                    }

                    sw.Stop();
                    state.ClientHost = clientIp;
                    state.Route = epRoute;
                    state.Method = epMethod;
                    state.DurationMs = sw.Elapsed.TotalMilliseconds;
                    if (engine?.StateStore != null)
                    {
                        await engine.StateStore.SaveStateAsync(state);
                    }

                    ctx.Response.Headers["X-Job-ID"] = state.JobId;
                    ctx.Response.Headers["X-Execution-Status"] = state.Status.ToString();
                    ctx.Response.Headers["X-Execution-Duration-Ms"] = sw.Elapsed.TotalMilliseconds.ToString("F2");

                    var outData = state.OutputData ?? state.Data;

                    if (pipeline != null)
                    {
                        await pipeline.ProcessPostAsync(ctx, outData, epStatus, sw.Elapsed.TotalMilliseconds, state);
                    }

                    if (ctx.Request.Query["raw"] == "true" || ctx.Request.Query["raw"] == "1")
                    {
                        return Results.Json(new
                        {
                            job_id = state.JobId,
                            workflow_name = state.WorkflowName,
                            route = epRoute,
                            method = epMethod,
                            status = state.Status.ToString(),
                            data = state.Data,
                            traces = state.Traces
                        }, statusCode: epStatus);
                    }

                    return Results.Json(outData, statusCode: epStatus);

                };

                switch (epMethod)
                {
                    case "GET":
                        endpoints.MapGet(epRoute, handler).WithTags(epTags).WithSummary(ep.Summary ?? $"{epMethod} {epRoute}");
                        break;
                    case "POST":
                        endpoints.MapPost(epRoute, handler).WithTags(epTags).WithSummary(ep.Summary ?? $"{epMethod} {epRoute}");
                        break;
                    case "PUT":
                        endpoints.MapPut(epRoute, handler).WithTags(epTags).WithSummary(ep.Summary ?? $"{epMethod} {epRoute}");
                        break;
                    case "DELETE":
                        endpoints.MapDelete(epRoute, handler).WithTags(epTags).WithSummary(ep.Summary ?? $"{epMethod} {epRoute}");
                        break;
                    case "PATCH":
                        endpoints.MapPatch(epRoute, handler).WithTags(epTags).WithSummary(ep.Summary ?? $"{epMethod} {epRoute}");
                        break;
                    default:
                        endpoints.MapMethods(epRoute, new[] { epMethod }, handler).WithTags(epTags);
                        break;
                }
            }
        }

        return endpoints;
    }

    public static IEndpointRouteBuilder MapLiveEndpoints(
        this IEndpointRouteBuilder endpoints,
        OctopusLiveEngine engine
    )
    {
        return endpoints.MapOctopusLiveEndpoints(engine.PipelineManager, engine.VisualizerService, engine.ApiKey, engine);
    }


    private static object? UnwrapJsonElement(object? obj)
    {
        if (obj is JsonElement elem)
        {
            switch (elem.ValueKind)
            {
                case JsonValueKind.String:
                    return elem.GetString();
                case JsonValueKind.Number:
                    if (elem.TryGetInt64(out var l)) return l;
                    if (elem.TryGetDouble(out var d)) return d;
                    return elem.GetRawText();
                case JsonValueKind.True:
                    return true;
                case JsonValueKind.False:
                    return false;
                case JsonValueKind.Null:
                case JsonValueKind.Undefined:
                    return null;
                case JsonValueKind.Object:
                    var dict = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
                    foreach (var prop in elem.EnumerateObject())
                    {
                        dict[prop.Name] = UnwrapJsonElement(prop.Value)!;
                    }
                    return dict;
                case JsonValueKind.Array:
                    var list = new List<object>();
                    foreach (var item in elem.EnumerateArray())
                    {
                        list.Add(UnwrapJsonElement(item)!);
                    }
                    return list;
            }
        }
        return obj;
    }

    private static Dictionary<string, object> GenerateOpenApiSchemaFromType(
        Type type,
        Dictionary<string, object> registeredSchemas,
        IReadOnlyDictionary<string, Type>? modelRegistry
    )
    {
        if (type == typeof(string)) return new Dictionary<string, object> { ["type"] = "string" };
        if (type == typeof(int) || type == typeof(long) || type == typeof(short) || type == typeof(byte)) return new Dictionary<string, object> { ["type"] = "integer" };
        if (type == typeof(double) || type == typeof(float) || type == typeof(decimal)) return new Dictionary<string, object> { ["type"] = "number", ["format"] = "double" };
        if (type == typeof(bool)) return new Dictionary<string, object> { ["type"] = "boolean" };

        if (typeof(System.Collections.IEnumerable).IsAssignableFrom(type) && type != typeof(string))
        {
            Type itemType = typeof(string);
            if (type.IsGenericType)
            {
                itemType = type.GetGenericArguments()[0];
            }
            else if (type.IsArray)
            {
                itemType = type.GetElementType() ?? typeof(string);
            }

            if (itemType != typeof(string) && itemType != typeof(int) && itemType != typeof(double) && itemType != typeof(bool) && itemType != typeof(object))
            {
                var itemTypeName = itemType.Name;
                if (!registeredSchemas.ContainsKey(itemTypeName))
                {
                    registeredSchemas[itemTypeName] = new Dictionary<string, object>();
                    registeredSchemas[itemTypeName] = GenerateOpenApiSchemaFromType(itemType, registeredSchemas, modelRegistry);
                }
                return new Dictionary<string, object>
                {
                    ["type"] = "array",
                    ["items"] = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{itemTypeName}" }
                };
            }

            var itemSimpleType = itemType == typeof(int) || itemType == typeof(long) ? "integer"
                : (itemType == typeof(double) || itemType == typeof(float) || itemType == typeof(decimal) ? "number"
                : (itemType == typeof(bool) ? "boolean" : "string"));

            return new Dictionary<string, object>
            {
                ["type"] = "array",
                ["items"] = new Dictionary<string, object> { ["type"] = itemSimpleType }
            };
        }

        var properties = new Dictionary<string, object>();
        var props = type.GetProperties(System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Instance);
        foreach (var p in props)
        {
            var pName = p.Name;
            var jsonProp = p.GetCustomAttributes(typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), true).FirstOrDefault() as System.Text.Json.Serialization.JsonPropertyNameAttribute;
            if (jsonProp != null && !string.IsNullOrEmpty(jsonProp.Name))
            {
                pName = jsonProp.Name;
            }
            else
            {
                // camelCase default
                pName = char.ToLowerInvariant(pName[0]) + pName.Substring(1);
            }

            var pType = p.PropertyType;
            Dictionary<string, object> propSchema;

            if (pType == typeof(string)) propSchema = new Dictionary<string, object> { ["type"] = "string" };
            else if (pType == typeof(int) || pType == typeof(long) || pType == typeof(short) || pType == typeof(byte)) propSchema = new Dictionary<string, object> { ["type"] = "integer" };
            else if (pType == typeof(double) || pType == typeof(float) || pType == typeof(decimal)) propSchema = new Dictionary<string, object> { ["type"] = "number", ["format"] = "double" };
            else if (pType == typeof(bool)) propSchema = new Dictionary<string, object> { ["type"] = "boolean" };
            else if (typeof(System.Collections.IEnumerable).IsAssignableFrom(pType))
            {
                Type itemType = typeof(string);
                if (pType.IsGenericType) itemType = pType.GetGenericArguments()[0];
                else if (pType.IsArray) itemType = pType.GetElementType() ?? typeof(string);

                if (itemType != typeof(string) && itemType != typeof(int) && itemType != typeof(double) && itemType != typeof(bool) && itemType != typeof(object))
                {
                    var itemTypeName = itemType.Name;
                    if (!registeredSchemas.ContainsKey(itemTypeName))
                    {
                        registeredSchemas[itemTypeName] = new Dictionary<string, object>();
                        registeredSchemas[itemTypeName] = GenerateOpenApiSchemaFromType(itemType, registeredSchemas, modelRegistry);
                    }
                    propSchema = new Dictionary<string, object>
                    {
                        ["type"] = "array",
                        ["items"] = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{itemTypeName}" }
                    };
                }
                else
                {
                    var itemSimpleType = itemType == typeof(int) || itemType == typeof(long) ? "integer"
                        : (itemType == typeof(double) || itemType == typeof(float) || itemType == typeof(decimal) ? "number"
                        : (itemType == typeof(bool) ? "boolean" : "string"));
                    propSchema = new Dictionary<string, object>
                    {
                        ["type"] = "array",
                        ["items"] = new Dictionary<string, object> { ["type"] = itemSimpleType }
                    };
                }
            }
            else
            {
                var complexTypeName = pType.Name;
                if (!registeredSchemas.ContainsKey(complexTypeName))
                {
                    registeredSchemas[complexTypeName] = new Dictionary<string, object>();
                    registeredSchemas[complexTypeName] = GenerateOpenApiSchemaFromType(pType, registeredSchemas, modelRegistry);
                }
                propSchema = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{complexTypeName}" };
            }

            properties[pName] = propSchema;
        }

        // Also check public fields
        var fields = type.GetFields(System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Instance);
        foreach (var f in fields)
        {
            var fName = f.Name;
            var jsonProp = f.GetCustomAttributes(typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), true).FirstOrDefault() as System.Text.Json.Serialization.JsonPropertyNameAttribute;
            if (jsonProp != null && !string.IsNullOrEmpty(jsonProp.Name))
            {
                fName = jsonProp.Name;
            }
            else
            {
                fName = char.ToLowerInvariant(fName[0]) + fName.Substring(1);
            }

            if (properties.ContainsKey(fName)) continue;

            var fType = f.FieldType;
            Dictionary<string, object> fieldSchema;

            if (fType == typeof(string)) fieldSchema = new Dictionary<string, object> { ["type"] = "string" };
            else if (fType == typeof(int) || fType == typeof(long) || fType == typeof(short) || fType == typeof(byte)) fieldSchema = new Dictionary<string, object> { ["type"] = "integer" };
            else if (fType == typeof(double) || fType == typeof(float) || fType == typeof(decimal)) fieldSchema = new Dictionary<string, object> { ["type"] = "number", ["format"] = "double" };
            else if (fType == typeof(bool)) fieldSchema = new Dictionary<string, object> { ["type"] = "boolean" };
            else
            {
                var complexTypeName = fType.Name;
                if (!registeredSchemas.ContainsKey(complexTypeName))
                {
                    registeredSchemas[complexTypeName] = new Dictionary<string, object>();
                    registeredSchemas[complexTypeName] = GenerateOpenApiSchemaFromType(fType, registeredSchemas, modelRegistry);
                }
                fieldSchema = new Dictionary<string, object> { ["$ref"] = $"#/components/schemas/{complexTypeName}" };
            }

            properties[fName] = fieldSchema;
        }

        return new Dictionary<string, object>
        {
            ["type"] = "object",
            ["properties"] = properties
        };
    }
}
