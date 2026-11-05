namespace Octopus.LiveEngine.Domain.Models;

public enum ExecutionStatus
{
    IDLE,
    RUNNING,
    COMPLETED,
    FAILED,
    ROLLED_BACK,
    CANCELLED
}

public class SingleCondition
{
    public string? Variable { get; set; }
    public string? Field { get; set; }
    public string? Operator { get; set; }
    public object? Value { get; set; }
}

public class ConditionsBlock
{
    public string Match { get; set; } = "all";
    public List<SingleCondition> Conditions { get; set; } = new();
}

public class StepCondition
{
    public string? Rule { get; set; }
    public string? Variable { get; set; }
    public string? Field { get; set; }
    public string? Operator { get; set; }
    public object? Value { get; set; }
    public List<WorkflowStep> ThenSteps { get; set; } = new();
    public List<WorkflowStep> ElseSteps { get; set; } = new();
}

public class WorkflowStep
{
    public string Id { get; set; } = string.Empty;
    public string? Activity { get; set; }
    public string? Description { get; set; }
    public int TimeoutSeconds { get; set; } = 30;
    public double DelaySeconds { get; set; } = 0.0;
    public string? Compensation { get; set; }
    public string? BranchingStep { get; set; }
    public string? HumanStep { get; set; }
    public StepCondition? Condition { get; set; }
    public ConditionsBlock? Conditions { get; set; }
    public List<WorkflowStep> ThenSteps { get; set; } = new();
    public List<WorkflowStep> ElseSteps { get; set; } = new();
    public Dictionary<string, object>? Loop { get; set; }
    public Dictionary<string, object>? ForEach { get; set; }
    public Dictionary<string, object>? RetryPolicy { get; set; }
    public string? JumpTo { get; set; }
    public int? JumpToStepIndex { get; set; }
    public int? RewindToStepIndex { get; set; }
    public bool Async { get; set; }
    public bool IsAsync { get; set; }
    public bool Compensate { get; set; }
    public bool Rollback { get; set; }
    public bool TriggerCompensation { get; set; }
    public bool Terminate { get; set; }
    public bool Settle { get; set; }
    public string? Reason { get; set; }
    public Dictionary<string, object>? DynamicBranching { get; set; }
    public bool IsOutput { get; set; }
    public bool Output { get; set; }
    public List<MiddlewareExceptionConfig> Exceptions { get; set; } = new();
    public List<string>? MaskFields { get; set; }
    public List<string>? Mask { get; set; }
    public MaskingConfig? Masking { get; set; }

    public List<WorkflowStep> GetThenSteps()
    {
        if (ThenSteps != null && ThenSteps.Count > 0) return ThenSteps;
        if (Condition?.ThenSteps != null && Condition.ThenSteps.Count > 0) return Condition.ThenSteps;
        return new List<WorkflowStep>();
    }

    public List<WorkflowStep> GetElseSteps()
    {
        if (ElseSteps != null && ElseSteps.Count > 0) return ElseSteps;
        if (Condition?.ElseSteps != null && Condition.ElseSteps.Count > 0) return Condition.ElseSteps;
        return new List<WorkflowStep>();
    }
}

public class MaskingConfig
{
    public bool Enabled { get; set; } = true;
    public List<string> Fields { get; set; } = new();
    public string MaskValue { get; set; } = "********";
}

public class TelemetryConfig
{
    public MaskingConfig? Masking { get; set; }
    public List<string>? MaskFields { get; set; }
}

public class ApiEndpointConfig
{
    public string Route { get; set; } = string.Empty;
    public string Method { get; set; } = "POST";
    public string? InputModel { get; set; }
    public string? OutputModel { get; set; }
    public int StatusCode { get; set; } = 200;
    public string? Summary { get; set; }
    public string? Description { get; set; }
    public List<string> Tags { get; set; } = new();
    public List<WorkflowStep> Steps { get; set; } = new();
}

public class MiddlewareExceptionConfig
{
    public string Exception { get; set; } = string.Empty;
    public int Code { get; set; }
    public string? Message { get; set; }
    public string? Activity { get; set; }
    public bool Async { get; set; }
    public bool IsAsync { get; set; }
    public Dictionary<string, string> Headers { get; set; } = new();
}

public class MiddlewareConfig
{
    public string Name { get; set; } = string.Empty;
    public string Type { get; set; } = "pre-request";
    public string? Activity { get; set; }
    public bool Async { get; set; }
    public bool IsAsync { get; set; }
    public List<string> Excludes { get; set; } = new();
    public List<string> Includes { get; set; } = new();
    public List<MiddlewareExceptionConfig> Exceptions { get; set; } = new();
    public Dictionary<string, object> Config { get; set; } = new();

    // Top-level properties commonly provided in YAML definitions:
    public int? Limit { get; set; }
    public int? WindowSeconds { get; set; }
    public string? KeyBy { get; set; }
    public string? TierClaim { get; set; }
    public string? TierHeader { get; set; }
    public Dictionary<string, int>? TierLimits { get; set; }
    public List<string>? AllowedIps { get; set; }
    public List<string>? BlockedIps { get; set; }
    public int? TtlSeconds { get; set; }
    public List<string>? KeyStrategy { get; set; }
    public List<string>? Methods { get; set; }
    public string? HeaderName { get; set; }
    public List<string>? ValidKeys { get; set; }
    public List<string>? ApiKeys { get; set; }
    public string? JwtSecret { get; set; }
    public string? Prefix { get; set; }
    public string? RedisUrl { get; set; }
    public bool? CacheControlHeaders { get; set; }
    public long? MaxBodySizeBytes { get; set; }
    public bool? RequireJsonBody { get; set; }
    public List<string>? RequiredHeaders { get; set; }
    public object? RequiredFields { get; set; }
}

public class GlobalExceptionHandlerConfig
{
    public string? Activity { get; set; }
    public int Code { get; set; } = 500;
    public string? Message { get; set; }
    public bool Async { get; set; }
    public bool IsAsync { get; set; }
    public bool IncludeStackTrace { get; set; } = true;
    public Dictionary<string, string> Headers { get; set; } = new();

    public bool GetIsAsync() => IsAsync || Async;
}

public class ErrorHandlingConfig
{
    public GlobalExceptionHandlerConfig? GlobalHandler { get; set; }
}

public class WorkflowBlueprint
{
    public string WorkflowName { get; set; } = string.Empty;
    public string Version { get; set; } = "1.0.0";
    public string Namespace { get; set; } = "default";
    public string Description { get; set; } = string.Empty;
    public string Mode { get; set; } = "sync";
    public string CompensationMode { get; set; } = "sync";
    public List<string> AllowedInitiatorRoles { get; set; } = new();
    public List<MiddlewareConfig> Middlewares { get; set; } = new();
    public List<WorkflowStep> Steps { get; set; } = new();
    public List<ApiEndpointConfig> Api { get; set; } = new();
    public Dictionary<string, object> Persistence { get; set; } = new();
    public Dictionary<string, object> Metadata { get; set; } = new();
    public MaskingConfig? Masking { get; set; }
    public TelemetryConfig? Telemetry { get; set; }
    public MaskingConfig? Redaction { get; set; }
    public ErrorHandlingConfig? ErrorHandling { get; set; }
    public GlobalExceptionHandlerConfig? GlobalExceptionHandler { get; set; }

    public (List<string> Fields, string MaskValue) GetMaskConfig(WorkflowStep? step = null)
    {
        var fields = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var maskVal = "********";

        // 1. Global blueprint configs
        if (Masking != null && Masking.Enabled)
        {
            if (Masking.Fields != null)
            {
                foreach (var f in Masking.Fields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
            }
            if (!string.IsNullOrWhiteSpace(Masking.MaskValue)) maskVal = Masking.MaskValue;
        }

        if (Telemetry != null)
        {
            if (Telemetry.Masking != null && Telemetry.Masking.Enabled)
            {
                if (Telemetry.Masking.Fields != null)
                {
                    foreach (var f in Telemetry.Masking.Fields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
                }
                if (!string.IsNullOrWhiteSpace(Telemetry.Masking.MaskValue)) maskVal = Telemetry.Masking.MaskValue;
            }
            if (Telemetry.MaskFields != null)
            {
                foreach (var f in Telemetry.MaskFields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
            }
        }

        if (Redaction != null && Redaction.Enabled)
        {
            if (Redaction.Fields != null)
            {
                foreach (var f in Redaction.Fields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
            }
            if (!string.IsNullOrWhiteSpace(Redaction.MaskValue)) maskVal = Redaction.MaskValue;
        }

        // 2. Step-level override
        if (step != null)
        {
            if (step.MaskFields != null)
            {
                foreach (var f in step.MaskFields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
            }
            if (step.Mask != null)
            {
                foreach (var f in step.Mask) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
            }
            if (step.Masking != null && step.Masking.Enabled)
            {
                if (step.Masking.Fields != null)
                {
                    foreach (var f in step.Masking.Fields) if (!string.IsNullOrWhiteSpace(f)) fields.Add(f.Trim());
                }
                if (!string.IsNullOrWhiteSpace(step.Masking.MaskValue)) maskVal = step.Masking.MaskValue;
            }
        }

        return (fields.ToList(), maskVal);
    }

    public void NormalizeEndpoints()
    {
        if (Api == null || Api.Count == 0) return;
        var wfName = (WorkflowName ?? "").Trim('/', ' ');
        var ver = (Version ?? "").Trim('/', ' ');
        if (string.IsNullOrEmpty(ver)) ver = "1.0.0";
        var verStr = ver.StartsWith("v") || ver.StartsWith("/") ? ver : "v" + ver;
        var prefix = $"/api/{verStr.Trim('/')}/{wfName}";

        foreach (var ep in Api)
        {
            if (!string.IsNullOrWhiteSpace(ep.Route))
            {
                var r = ep.Route.Trim();
                if (!r.StartsWith("/api/") && !r.StartsWith(prefix))
                {
                    ep.Route = $"{prefix}/{r.TrimStart('/')}";
                }
            }
        }
    }
}

public class StepExecutionTrace
{
    public string StepId { get; set; } = string.Empty;
    public string? ActivityName { get; set; }
    public string Status { get; set; } = "COMPLETED";
    public bool IsAsync { get; set; }
    public Dictionary<string, object> InputPayload { get; set; } = new();
    public Dictionary<string, object> OutputPayload { get; set; } = new();
    public string? ErrorMessage { get; set; }
    public string StartedAt { get; set; } = DateTime.UtcNow.ToString("o");
    public string FinishedAt { get; set; } = DateTime.UtcNow.ToString("o");
    public double DurationMs { get; set; }
}

public class ExecutionState
{
    public string JobId { get; set; } = string.Empty;
    public string WorkflowName { get; set; } = string.Empty;
    public ExecutionStatus Status { get; set; } = ExecutionStatus.IDLE;
    public string? Route { get; set; }
    public string? Method { get; set; }
    public string? CurrentStepId { get; set; }
    public int CurrentIndex { get; set; }
    public int TotalSteps { get; set; }
    public Dictionary<string, object> Data { get; set; } = new();
    public Dictionary<string, object>? OutputData { get; set; }
    public List<StepExecutionTrace> Traces { get; set; } = new();
    public string StartedAt { get; set; } = DateTime.UtcNow.ToString("o");
    public string? FinishedAt { get; set; }
    public string? ClientHost { get; set; }
    public double DurationMs { get; set; }
    public string? Error { get; set; }
    public int? ErrorCode { get; set; }
    public string? ErrorType { get; set; }
}

public record LiveEvent(string Event, object Data);

