using System.Collections.Concurrent;
using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.AspNetCore.Http;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using StackExchange.Redis;

namespace Octopus.LiveEngine.Domain.Services;

// ============================================================================
// 1. Standard Built-in Middleware Error Hierarchy
// ============================================================================

public class OctopusMiddlewareException : Exception
{
    public string Name { get; set; }
    public int StatusCode { get; set; }
    public string Detail { get; set; }
    public Dictionary<string, object> Details { get; set; }
    public Dictionary<string, string> Headers { get; set; }

    public OctopusMiddlewareException(
        string name,
        int statusCode,
        string detail,
        Dictionary<string, object>? details = null,
        Dictionary<string, string>? headers = null
    ) : base(detail)
    {
        Name = name;
        StatusCode = statusCode;
        Detail = detail;
        Details = details ?? new Dictionary<string, object>();
        Headers = headers ?? new Dictionary<string, string>();
    }
}

public class UserNotAuthenticatedException : OctopusMiddlewareException
{
    public UserNotAuthenticatedException(string? detail = null)
        : base(
            "UserNotAuthenticated",
            StatusCodes.Status401Unauthorized,
            detail ?? "User is not authenticated or token is missing/invalid"
        )
    {
    }
}

public class InvalidCredentialsException : OctopusMiddlewareException
{
    public InvalidCredentialsException(string? detail = null)
        : base(
            "InvalidCredentials",
            StatusCodes.Status401Unauthorized,
            detail ?? "Invalid username or password. Please try again."
        )
    {
    }
}

public class UserNotAuthorizedException : OctopusMiddlewareException
{
    public UserNotAuthorizedException(string? detail = null)
        : base(
            "UserNotAuthorized",
            StatusCodes.Status403Forbidden,
            detail ?? "User lacks authorization or required permissions"
        )
    {
    }
}

public class InvalidApiKeyException : OctopusMiddlewareException
{
    public InvalidApiKeyException(string? detail = null)
        : base(
            "InvalidApiKeyException",
            StatusCodes.Status401Unauthorized,
            detail ?? "Invalid or missing API key"
        )
    {
    }
}

public class RateLimitExceededException : OctopusMiddlewareException
{
    public RateLimitExceededException(
        string? detail,
        int limit,
        int windowSeconds,
        int retryAfter,
        string key,
        Dictionary<string, string>? headers = null
    ) : base(
        "RateLimitExceeded",
        StatusCodes.Status429TooManyRequests,
        detail ?? $"Rate limit exceeded. Quota: {limit} req/{windowSeconds}s. Retry in {retryAfter}s.",
        new Dictionary<string, object>
        {
            ["limit"] = limit,
            ["window_seconds"] = windowSeconds,
            ["retry_after_seconds"] = retryAfter,
            ["rate_limited_key"] = key
        },
        headers
    )
    {
    }
}

public class IpAccessDeniedException : OctopusMiddlewareException
{
    public IpAccessDeniedException(string? detail, string clientIp, string path, string method)
        : base(
            "IpAccessDenied",
            StatusCodes.Status403Forbidden,
            detail ?? $"Access denied: Client IP '{clientIp}' is not in the allowed IP whitelist",
            new Dictionary<string, object>
            {
                ["client_ip"] = clientIp,
                ["path"] = path,
                ["method"] = method
            }
        )
    {
    }
}

public class BadRequestException : OctopusMiddlewareException
{
    public BadRequestException(string? detail = null, Dictionary<string, object>? details = null, Dictionary<string, string>? headers = null)
        : base(
            "BadRequestException",
            StatusCodes.Status400BadRequest,
            detail ?? "Invalid or malformed request payload. Please verify syntax and parameters.",
            details,
            headers
        )
    {
    }
}

public class MalformedJsonException : BadRequestException
{
    public MalformedJsonException(string? detail = null)
        : base(detail ?? "Malformed JSON syntax in request body.")
    {
        Name = "MalformedJsonException";
    }
}

public class ValidationException : BadRequestException
{
    public ValidationException(string? detail = null, Dictionary<string, object>? details = null)
        : base(detail ?? "Request validation rules failed.", details)
    {
        Name = "ValidationException";
    }
}

public class MissingRequiredFieldException : ValidationException
{
    public MissingRequiredFieldException(string? fieldOrMsg = null)
        : base(
            (fieldOrMsg != null && !fieldOrMsg.Contains(' '))
                ? $"Missing required field '{fieldOrMsg}' in request payload."
                : (fieldOrMsg ?? "Missing required field in request payload."),
            fieldOrMsg != null ? new Dictionary<string, object> { ["field"] = fieldOrMsg } : null
        )
    {
        Name = "MissingRequiredFieldException";
    }
}

public class PayloadTooLargeException : OctopusMiddlewareException
{
    public PayloadTooLargeException(string? detail, long maxSize, long actualSize)
        : base(
            "PayloadTooLargeException",
            StatusCodes.Status413PayloadTooLarge,
            detail ?? $"Request payload size {actualSize} bytes exceeds maximum limit of {maxSize} bytes.",
            new Dictionary<string, object>
            {
                ["max_size_bytes"] = maxSize,
                ["actual_size_bytes"] = actualSize
            }
        )
    {
    }
}

// ============================================================================
// 2. Route Matching Utility
// ============================================================================

public class RouteMatcher
{
    public static readonly string[] SystemExcludes = new[]
    {
        "/favicon.ico",
        "/favicon.png",
        "/robots.txt",
        "/health*",
        "/visualizer*",
        "/api/visualizer*",
        "*/diagram*",
        "*/diagram/x6*",
        "/docs*",
        "/openapi.json",
        "/redoc*"
    };

    public bool Matches(string path, string pattern) => MatchesStatic(path, pattern);
    public bool ShouldSkip(string path, IEnumerable<string>? excludes, IEnumerable<string>? includes) => ShouldSkipStatic(path, excludes, includes);

    public static bool MatchesStatic(string path, string pattern)
    {
        if (string.IsNullOrEmpty(path) || string.IsNullOrEmpty(pattern))
        {
            return false;
        }

        var cleanPath = path.TrimEnd('/');
        var cleanPat = pattern.TrimEnd('/');

        if (string.Equals(cleanPath, cleanPat, StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }

        if (pattern.EndsWith("/*") || pattern.EndsWith("*"))
        {
            var prefix = pattern.TrimEnd('*').TrimEnd('/');
            if (path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        if (pattern.Contains('*') || pattern.Contains('?'))
        {
            var regexPattern = "^" + Regex.Escape(pattern).Replace("\\*", ".*").Replace("\\?", ".") + "$";
            if (Regex.IsMatch(path, regexPattern, RegexOptions.IgnoreCase))
            {
                return true;
            }
        }

        return false;
    }

    public static bool ShouldSkipStatic(string path, IEnumerable<string>? excludes, IEnumerable<string>? includes)
    {
        foreach (var sysExc in SystemExcludes)
        {
            if (MatchesStatic(path, sysExc))
            {
                return true;
            }
        }

        if (includes != null)
        {
            var incList = includes.Where(s => !string.IsNullOrWhiteSpace(s)).ToList();
            if (incList.Count > 0)
            {
                var included = incList.Any(inc => MatchesStatic(path, inc));
                if (!included)
                {
                    return true;
                }
            }
        }

        if (excludes != null)
        {
            var excList = excludes.Where(s => !string.IsNullOrWhiteSpace(s)).ToList();
            if (excList.Count > 0)
            {
                if (excList.Any(exc => MatchesStatic(path, exc)))
                {
                    return true;
                }
            }
        }

        return false;
    }
}

// ============================================================================
// 3. Middleware Exception Responder Helper
// ============================================================================

public static class MiddlewareErrorResponder
{
    public static async Task RespondMiddlewareErrorAsync(
        HttpContext context,
        OctopusMiddlewareException error,
        IEnumerable<MiddlewareExceptionConfig>? exceptions,
        IActivityExecutor? executor,
        IAsyncActivityDispatcher? asyncDispatcher = null,
        string? jobId = null
    )
    {
        var statusCode = error.StatusCode;
        var message = error.Detail;
        var details = new Dictionary<string, object>(error.Details, StringComparer.OrdinalIgnoreCase);
        var headers = new Dictionary<string, string>(error.Headers, StringComparer.OrdinalIgnoreCase);

        MiddlewareExceptionConfig? matchedCfg = null;
        if (exceptions != null)
        {
            foreach (var ec in exceptions)
            {
                if (string.Equals(ec.Exception, error.Name, StringComparison.OrdinalIgnoreCase))
                {
                    matchedCfg = ec;
                    break;
                }
            }
        }

        if (matchedCfg != null)
        {
            if (matchedCfg.Code > 0)
            {
                statusCode = matchedCfg.Code;
            }
            if (!string.IsNullOrEmpty(matchedCfg.Message))
            {
                message = matchedCfg.Message;
            }
            foreach (var (k, v) in matchedCfg.Headers)
            {
                headers[k] = v;
            }

            if (!string.IsNullOrEmpty(matchedCfg.Activity) && executor != null)
            {
                var clientIp = context.Connection.RemoteIpAddress?.ToString() ?? "127.0.0.1";
                if (context.Request.Headers.TryGetValue("X-Forwarded-For", out var fwd))
                {
                    clientIp = fwd.FirstOrDefault()?.Split(',').FirstOrDefault()?.Trim() ?? clientIp;
                }

                var effJobId = jobId ?? context.Request.Headers["X-Job-ID"].FirstOrDefault()
                                     ?? context.Request.Headers["X-Request-ID"].FirstOrDefault()
                                     ?? $"job_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}_{Guid.NewGuid():N}";

                var actPayload = new Dictionary<string, object>
                {
                    ["exception"] = error.Name,
                    ["error"] = error.Name,
                    ["message"] = message,
                    ["original_error"] = error.Message,
                    ["status_code"] = statusCode,
                    ["job_id"] = effJobId,
                    ["path"] = context.Request.Path.Value ?? "",
                    ["method"] = context.Request.Method,
                    ["client_host"] = clientIp,
                    ["details"] = details
                };

                if (context.Items.TryGetValue("auth_claims", out var claims) && claims != null)
                {
                    actPayload["auth_claims"] = claims;
                }

                if (matchedCfg.IsAsync || matchedCfg.Async)
                {
                    var actName = matchedCfg.Activity;
                    var stepId = $"mw_exc_{actName.ToLowerInvariant()}";
                    if (asyncDispatcher != null)
                    {
                        asyncDispatcher.Dispatch(actName, actPayload, effJobId, stepId, executor);
                    }
                    else
                    {
                        _ = Task.Run(async () =>
                        {
                            try
                            {
                                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
                                await executor.ExecuteActivityAsync(actName, actPayload, cts.Token);
                            }
                            catch { }
                        });
                    }
                }
                else
                {
                    try
                    {
                        using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(5));
                        var actRes = await executor.ExecuteActivityAsync(matchedCfg.Activity, actPayload, cts.Token);
                        if (actRes != null)
                        {
                            foreach (var (k, v) in actRes)
                            {
                                if (!string.Equals(k, "status_code", StringComparison.OrdinalIgnoreCase) &&
                                    !string.Equals(k, "code", StringComparison.OrdinalIgnoreCase) &&
                                    !string.Equals(k, "message", StringComparison.OrdinalIgnoreCase) &&
                                    !string.Equals(k, "detail", StringComparison.OrdinalIgnoreCase))
                                {
                                    details[k] = v;
                                }
                            }
                        }
                    }
                    catch { }
                }
            }
        }

        foreach (var (k, v) in headers)
        {
            context.Response.Headers[k] = v;
        }

        var effJobIdOut = jobId ?? context.Request.Headers["X-Job-ID"].FirstOrDefault()
                                ?? context.Request.Headers["X-Request-ID"].FirstOrDefault()
                                ?? $"job_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}_{Guid.NewGuid():N}";

        context.Response.Headers["X-Job-ID"] = effJobIdOut;

        if (context.Items.TryGetValue("octopus_state_store", out var storeObj) && storeObj is IStateStore stateStore)
        {
            var pVal = context.Request.Path.Value ?? "";
            var parts = pVal.Split('/', StringSplitOptions.RemoveEmptyEntries);
            var wf = parts.Length >= 3 ? parts[2] : (parts.Length >= 2 ? parts[1] : "customer_auth_portal");
            var clientIp = context.Connection.RemoteIpAddress?.ToString() ?? "127.0.0.1";
            if (context.Request.Headers.TryGetValue("X-Forwarded-For", out var fwd))
            {
                clientIp = fwd.FirstOrDefault()?.Split(',').FirstOrDefault()?.Trim() ?? clientIp;
            }

            var failedState = new ExecutionState
            {
                JobId = effJobIdOut,
                WorkflowName = wf,
                Status = ExecutionStatus.FAILED,
                Route = pVal,
                Method = context.Request.Method,
                ClientHost = clientIp,
                DurationMs = 1.0,
                Error = message,
                ErrorType = error.Name,
                ErrorCode = statusCode,
                StartedAt = DateTime.UtcNow.ToString("o"),
                FinishedAt = DateTime.UtcNow.ToString("o"),
                Data = new Dictionary<string, object>
                {
                    ["error"] = error.Name,
                    ["detail"] = message,
                    ["client_ip"] = clientIp
                }
            };

            if (matchedCfg != null && !string.IsNullOrEmpty(matchedCfg.Activity))
            {
                failedState.Traces.Add(new StepExecutionTrace
                {
                    StepId = $"mw_exc_{matchedCfg.Activity.ToLowerInvariant()}",
                    ActivityName = matchedCfg.Activity,
                    Status = "COMPLETED",
                    IsAsync = matchedCfg.IsAsync || matchedCfg.Async,
                    StartedAt = DateTime.UtcNow.ToString("o"),
                    FinishedAt = DateTime.UtcNow.ToString("o"),
                    DurationMs = 1.0
                });
            }

            _ = Task.Run(async () =>
            {
                try { await stateStore.SaveStateAsync(failedState); } catch { }
            });
        }

        context.Response.StatusCode = statusCode;
        context.Response.ContentType = "application/json";

        var responseObj = new
        {
            error = error.Name,
            detail = message,
            status_code = statusCode,
            details
        };

        var json = JsonSerializer.Serialize(responseObj, new JsonSerializerOptions { WriteIndented = false });
        await context.Response.WriteAsync(json);
    }
}

// ============================================================================
// 4. Concrete Middleware Implementations
// ============================================================================

public interface IOctopusMiddleware
{
    Task<bool> InvokeAsync(HttpContext context);
}

// --- A. Bearer Auth & Claims Middleware ---
public delegate Task<Dictionary<string, object>> TokenValidatorFuncAsync(string token);
public delegate Dictionary<string, object> ClaimsMapperFunc(Dictionary<string, object> claims);

public static class Hs256Jwt
{
    /// <summary>Returns the claims of an HS256 JWT signed with secret; throws for a bad signature, another algorithm, an expired or not-yet-valid token.</summary>
    public static Dictionary<string, object> Verify(string token, string secret)
    {
        var parts = (token ?? "").Split('.');
        if (parts.Length != 3) throw new InvalidOperationException("malformed JWT");
        JsonElement header, claims;
        byte[] signature;
        try
        {
            header = JsonDocument.Parse(Decode(parts[0])).RootElement;
            claims = JsonDocument.Parse(Decode(parts[1])).RootElement;
            signature = Decode(parts[2]);
        }
        catch (Exception ex) when (ex is FormatException or JsonException)
        {
            throw new InvalidOperationException("malformed JWT");
        }
        if (header.ValueKind != JsonValueKind.Object || claims.ValueKind != JsonValueKind.Object
            || !header.TryGetProperty("alg", out var alg) || alg.ValueKind != JsonValueKind.String || alg.GetString() != "HS256")
            throw new InvalidOperationException("JWT must be HS256");
        using var hmac = new System.Security.Cryptography.HMACSHA256(Encoding.UTF8.GetBytes(secret));
        var expected = hmac.ComputeHash(Encoding.UTF8.GetBytes($"{parts[0]}.{parts[1]}"));
        if (!System.Security.Cryptography.CryptographicOperations.FixedTimeEquals(expected, signature))
            throw new InvalidOperationException("invalid JWT signature");
        var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        if (claims.TryGetProperty("exp", out var exp) && exp.ValueKind == JsonValueKind.Number && now >= exp.GetDouble())
            throw new InvalidOperationException("JWT has expired");
        if (claims.TryGetProperty("nbf", out var nbf) && nbf.ValueKind == JsonValueKind.Number && now < nbf.GetDouble())
            throw new InvalidOperationException("JWT is not valid yet");
        return (Dictionary<string, object>)ToPlain(claims)!;
    }

    private static byte[] Decode(string part)
    {
        var s = part.Replace('-', '+').Replace('_', '/');
        return Convert.FromBase64String(s.PadRight(s.Length + (4 - s.Length % 4) % 4, '='));
    }

    private static object? ToPlain(JsonElement e) => e.ValueKind switch
    {
        JsonValueKind.Object => e.EnumerateObject().ToDictionary(p => p.Name, p => ToPlain(p.Value)!),
        JsonValueKind.Array => e.EnumerateArray().Select(ToPlain).ToList(),
        JsonValueKind.String => e.GetString(),
        JsonValueKind.Number => e.TryGetInt64(out var l) ? l : e.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        _ => null,
    };
}

public class BearerAuthAndClaimsMiddleware : IOctopusMiddleware
{
    private readonly TokenValidatorFuncAsync _validator;
    private readonly ClaimsMapperFunc _mapper;
    private readonly string _tokenHeader;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<MiddlewareExceptionConfig> _exceptions;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;
    private readonly RouteMatcher _matcher = new();

    public BearerAuthAndClaimsMiddleware(
        TokenValidatorFuncAsync? validator,
        ClaimsMapperFunc? mapper,
        IEnumerable<string>? excludes,
        IEnumerable<string>? includes,
        IEnumerable<MiddlewareExceptionConfig>? exceptions,
        IActivityExecutor? executor,
        string tokenHeader = "Authorization",
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        // No validator and no secret: every token is refused.
        _validator = validator ?? (_ => throw new InvalidOperationException("no token validator or jwt_secret is configured"));

        _mapper = mapper ?? (claims => claims);
        _tokenHeader = tokenHeader;
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _exceptions = exceptions?.ToList() ?? new List<MiddlewareExceptionConfig>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes))
        {
            return true;
        }

        var authHeader = context.Request.Headers[_tokenHeader].FirstOrDefault();
        if (string.IsNullOrWhiteSpace(authHeader))
        {
            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                context,
                new UserNotAuthenticatedException($"Missing '{_tokenHeader}' header"),
                _exceptions,
                _executor,
                _asyncDispatcher
            );
            return false;
        }

        var token = authHeader.Trim();
        if (token.StartsWith("Bearer ", StringComparison.OrdinalIgnoreCase))
        {
            token = token.Substring(7).Trim();
        }

        Dictionary<string, object> claims;
        try
        {
            claims = await _validator(token);
        }
        catch (Exception ex)
        {
            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                context,
                new UserNotAuthenticatedException($"Token validation error: {ex.Message}"),
                _exceptions,
                _executor,
                _asyncDispatcher
            );
            return false;
        }

        var injections = _mapper(claims);
        context.Items["auth_claims"] = claims;
        context.Items["workflow_injections"] = injections;

        return true;
    }
}

// --- B. API Key Auth Middleware ---
public class ApiKeyAuthMiddleware : IOctopusMiddleware
{
    private readonly HashSet<string> _validKeys;
    private readonly string _headerName;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<MiddlewareExceptionConfig> _exceptions;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;
    private readonly RouteMatcher _matcher = new();

    public ApiKeyAuthMiddleware(
        IEnumerable<string>? validKeys,
        IEnumerable<string>? excludes,
        IEnumerable<string>? includes,
        IEnumerable<MiddlewareExceptionConfig>? exceptions,
        IActivityExecutor? executor,
        string headerName = "X-API-Key",
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        // No configured keys means no request passes: there are no built-in keys to fall back on.
        _validKeys = new HashSet<string>((validKeys ?? Enumerable.Empty<string>()).Where(k => !string.IsNullOrEmpty(k)), StringComparer.Ordinal);

        _headerName = headerName;
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _exceptions = exceptions?.ToList() ?? new List<MiddlewareExceptionConfig>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes))
        {
            return true;
        }

        var reqKey = context.Request.Headers[_headerName].FirstOrDefault();
        if (string.IsNullOrEmpty(reqKey))
        {
            reqKey = context.Request.Query["api_key"].FirstOrDefault();
        }

        if (string.IsNullOrEmpty(reqKey) || !_validKeys.Contains(reqKey))
        {
            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                context,
                new InvalidApiKeyException($"Invalid or missing '{_headerName}' header"),
                _exceptions,
                _executor,
                _asyncDispatcher
            );
            return false;
        }

        return true;
    }
}

// --- C. Correlation ID Middleware ---
public class CorrelationIdMiddleware : IOctopusMiddleware
{
    private readonly string _headerName;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly RouteMatcher _matcher = new();

    public CorrelationIdMiddleware(
        string? headerName = null,
        IEnumerable<string>? excludes = null,
        IEnumerable<string>? includes = null
    )
    {
        _headerName = !string.IsNullOrWhiteSpace(headerName) ? headerName : "X-Correlation-ID";
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
    }

    public Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes))
        {
            return Task.FromResult(true);
        }

        var corrId = context.Request.Headers[_headerName].FirstOrDefault()
            ?? context.Request.Headers["X-Request-ID"].FirstOrDefault()
            ?? $"req_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";

        context.Items["correlation_id"] = corrId;
        context.Response.Headers[_headerName] = corrId;
        return Task.FromResult(true);
    }
}

// --- D. Rate Limiting Middleware ---
public class RateLimitMiddleware : IOctopusMiddleware
{
    public int DefaultLimit { get; }
    public int WindowSeconds { get; }
    public string KeyBy { get; }
    public string TierClaim { get; }
    public string TierHeader { get; }
    public Dictionary<string, int> TierLimits { get; }
    public bool IncludeHeaders { get; }

    private readonly ConcurrentDictionary<string, List<long>> _requests = new();
    private readonly object _lock = new();
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<MiddlewareExceptionConfig> _exceptions;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;
    private readonly RouteMatcher _matcher = new();

    public RateLimitMiddleware(
        int limit = 60,
        int windowSeconds = 60,
        string keyBy = "client_ip",
        string tierClaim = "customer_tier",
        string tierHeader = "X-User-Tier",
        Dictionary<string, int>? tierLimits = null,
        IEnumerable<string>? excludes = null,
        IEnumerable<string>? includes = null,
        IEnumerable<MiddlewareExceptionConfig>? exceptions = null,
        IActivityExecutor? executor = null,
        bool includeHeaders = true,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        DefaultLimit = limit > 0 ? limit : 60;
        WindowSeconds = windowSeconds > 0 ? windowSeconds : 60;
        KeyBy = string.IsNullOrWhiteSpace(keyBy) ? "client_ip" : keyBy.ToLowerInvariant();
        TierClaim = tierClaim ?? "customer_tier";
        TierHeader = tierHeader ?? "X-User-Tier";
        TierLimits = tierLimits != null ? new Dictionary<string, int>(tierLimits, StringComparer.OrdinalIgnoreCase) : new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        IncludeHeaders = includeHeaders;

        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _exceptions = exceptions?.ToList() ?? new List<MiddlewareExceptionConfig>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;
    }

    private string ResolveKey(HttpContext context)
    {
        switch (KeyBy)
        {
            case "user_id":
            case "user":
            case "sub":
                var uid = context.Items.TryGetValue("user_id", out var u) ? u?.ToString() : null;
                if (string.IsNullOrEmpty(uid) && context.Items.TryGetValue("auth_claims", out var cObj) && cObj is Dictionary<string, object> cl)
                {
                    if (cl.TryGetValue("user_id", out var idVal)) uid = idVal?.ToString();
                    else if (cl.TryGetValue("sub", out var subVal)) uid = subVal?.ToString();
                }
                return !string.IsNullOrEmpty(uid) ? $"user:{uid}" : $"ip:{GetClientIp(context)}";

            case "api_key":
            case "key":
                var key = context.Request.Headers["X-API-Key"].FirstOrDefault() ?? "anonymous_key";
                return $"apikey:{key}";

            case "tenant_id":
            case "tenant":
                var tenant = context.Request.Headers["X-Tenant-ID"].FirstOrDefault() ?? "default_tenant";
                return $"tenant:{tenant}";

            case "path":
            case "route":
                return $"path:{context.Request.Path.Value}";

            default:
                return $"ip:{GetClientIp(context)}";
        }
    }

    private int ResolveLimit(HttpContext context)
    {
        if (TierLimits.Count > 0)
        {
            string? tier = null;

            // 1. Check Claims injected by BearerAuthAndClaimsMiddleware
            if (context.Items.TryGetValue("auth_claims", out var claimsObj) && claimsObj is Dictionary<string, object> claims)
            {
                if (!string.IsNullOrEmpty(TierClaim) && claims.TryGetValue(TierClaim, out var tcVal) && tcVal != null)
                {
                    tier = tcVal.ToString();
                }

                if (string.IsNullOrEmpty(tier))
                {
                    if (claims.TryGetValue("customer_tier", out var cVal) && cVal != null) tier = cVal.ToString();
                    else if (claims.TryGetValue("tier", out var tVal) && tVal != null) tier = tVal.ToString();
                    else if (claims.TryGetValue("subscription_plan", out var sVal) && sVal != null) tier = sVal.ToString();
                    else if (claims.TryGetValue("role", out var rVal) && rVal != null) tier = rVal.ToString();
                }
            }

            // 2. Check HTTP Header
            if (string.IsNullOrEmpty(tier))
            {
                var hdr = !string.IsNullOrEmpty(TierHeader) ? TierHeader : "X-User-Tier";
                tier = context.Request.Headers[hdr].FirstOrDefault();
            }

            // 3. Fallback to unauthenticated ANONYMOUS
            if (string.IsNullOrEmpty(tier))
            {
                tier = "ANONYMOUS";
            }

            if (!string.IsNullOrEmpty(tier))
            {
                if (TierLimits.TryGetValue(tier, out var lim)) return lim;
                if (TierLimits.TryGetValue(tier.ToUpperInvariant(), out var upperLim)) return upperLim;
            }
        }

        return DefaultLimit;
    }

    private static string GetClientIp(HttpContext context)
    {
        if (context.Request.Headers.TryGetValue("X-Forwarded-For", out var fwd))
        {
            var ip = fwd.FirstOrDefault()?.Split(',').FirstOrDefault()?.Trim();
            if (!string.IsNullOrEmpty(ip)) return ip;
        }
        return context.Connection.RemoteIpAddress?.ToString() ?? "127.0.0.1";
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes))
        {
            return true;
        }

        var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var key = ResolveKey(context);
        var maxLimit = ResolveLimit(context);
        var cutoff = now - WindowSeconds;

        List<long> valid;
        long retryAfter = 1;
        long resetTs = now + WindowSeconds;
        bool exceeded = false;

        lock (_lock)
        {
            var timestamps = _requests.GetOrAdd(key, _ => new List<long>());
            valid = timestamps.Where(ts => ts > cutoff).ToList();

            if (valid.Count >= maxLimit)
            {
                var oldest = valid.Count > 0 ? valid[0] : now;
                retryAfter = WindowSeconds - (now - oldest);
                if (retryAfter < 1) retryAfter = 1;
                resetTs = now + retryAfter;
                _requests[key] = valid;
                exceeded = true;
            }
            else
            {
                valid.Add(now);
                _requests[key] = valid;
                var remaining = maxLimit - valid.Count;
                if (remaining < 0) remaining = 0;
                resetTs = valid[0] + WindowSeconds;

                if (IncludeHeaders)
                {
                    context.Response.Headers["X-RateLimit-Limit"] = maxLimit.ToString();
                    context.Response.Headers["X-RateLimit-Remaining"] = remaining.ToString();
                    context.Response.Headers["X-RateLimit-Reset"] = resetTs.ToString();
                }
            }
        }

        if (exceeded)
        {
            var hdrs = new Dictionary<string, string>
            {
                ["Retry-After"] = retryAfter.ToString(),
                ["X-RateLimit-Limit"] = maxLimit.ToString(),
                ["X-RateLimit-Remaining"] = "0",
                ["X-RateLimit-Reset"] = resetTs.ToString()
            };

            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                context,
                new RateLimitExceededException(null, maxLimit, WindowSeconds, (int)retryAfter, key, hdrs),
                _exceptions,
                _executor,
                _asyncDispatcher
            );
            return false;
        }

        return true;
    }
}

// --- E. IP Whitelist Middleware ---
public delegate Task<(bool Allowed, string? Error)> IpValidatorCallbackAsync(string ip, HttpContext context);

public class IpWhitelistMiddleware : IOctopusMiddleware
{
    private readonly List<IPNetwork> _allowedNetworks = new();
    private readonly List<IPAddress> _allowedExactIps = new();
    private readonly List<IPNetwork> _blockedNetworks = new();
    private readonly List<IPAddress> _blockedExactIps = new();
    private readonly bool _allowAll;
    private readonly IpValidatorCallbackAsync? _customValidator;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<MiddlewareExceptionConfig> _exceptions;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;
    private readonly RouteMatcher _matcher = new();

    public IpWhitelistMiddleware(
        IEnumerable<string>? allowedIps = null,
        IEnumerable<string>? blockedIps = null,
        IpValidatorCallbackAsync? customValidator = null,
        IEnumerable<string>? excludes = null,
        IEnumerable<string>? includes = null,
        IEnumerable<MiddlewareExceptionConfig>? exceptions = null,
        IActivityExecutor? executor = null,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        _customValidator = customValidator;
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _exceptions = exceptions?.ToList() ?? new List<MiddlewareExceptionConfig>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;

        if (allowedIps != null)
        {
            foreach (var raw in allowedIps)
            {
                var s = raw.Trim();
                if (s == "*")
                {
                    _allowAll = true;
                }
                else if (s.Contains('/'))
                {
                    if (IPNetwork.TryParse(s, out var net)) _allowedNetworks.Add(net);
                }
                else if (IPAddress.TryParse(s, out var ip))
                {
                    _allowedExactIps.Add(ip);
                }
            }
        }

        if (blockedIps != null)
        {
            foreach (var raw in blockedIps)
            {
                var s = raw.Trim();
                if (s.Contains('/'))
                {
                    if (IPNetwork.TryParse(s, out var net)) _blockedNetworks.Add(net);
                }
                else if (IPAddress.TryParse(s, out var ip))
                {
                    _blockedExactIps.Add(ip);
                }
            }
        }
    }

    private bool IsIpAllowed(string clientIpStr, HttpContext context)
    {
        if (!IPAddress.TryParse(clientIpStr, out var ip))
        {
            return false;
        }

        if (ip.IsIPv4MappedToIPv6)
        {
            ip = ip.MapToIPv4();
        }

        // Blocked check
        foreach (var blockedIp in _blockedExactIps)
        {
            if (blockedIp.Equals(ip)) return false;
        }
        foreach (var blockedNet in _blockedNetworks)
        {
            if (blockedNet.Contains(ip)) return false;
        }

        if (_allowAll) return true;

        foreach (var allowedIp in _allowedExactIps)
        {
            if (allowedIp.Equals(ip)) return true;
        }
        foreach (var allowedNet in _allowedNetworks)
        {
            if (allowedNet.Contains(ip)) return true;
        }

        return false;
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes))
        {
            return true;
        }

        var clientIp = context.Connection.RemoteIpAddress?.ToString() ?? "127.0.0.1";
        if (context.Request.Headers.TryGetValue("X-Forwarded-For", out var fwd))
        {
            var raw = fwd.FirstOrDefault()?.Split(',').FirstOrDefault()?.Trim();
            if (!string.IsNullOrEmpty(raw)) clientIp = raw;
        }

        if (!IsIpAllowed(clientIp, context))
        {
            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                context,
                new IpAccessDeniedException(null, clientIp, path, context.Request.Method),
                _exceptions,
                _executor,
                _asyncDispatcher
            );
            return false;
        }

        return true;
    }
}

// --- F. Response Audit Middleware (Post-Request Lifecycle Auditing) ---
public class ResponseAuditMiddleware : IOctopusMiddleware
{
    private readonly string _activity;
    private readonly bool _isAsync;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;

    public ResponseAuditMiddleware(
        string activity,
        bool isAsync,
        List<string>? excludes,
        List<string>? includes,
        IActivityExecutor? executor,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        _activity = activity;
        _isAsync = isAsync;
        _excludes = excludes ?? new List<string>();
        _includes = includes ?? new List<string>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;
    }

    public Task<bool> InvokeAsync(HttpContext context)
    {
        return Task.FromResult(true);
    }

    public async Task ProcessPostAsync(HttpContext context, object? responseData, int statusCode, double durationMs, ExecutionState state)
    {
        var path = context.Request.Path.Value ?? "";
        if (RouteMatcher.ShouldSkipStatic(path, _excludes, _includes)) return;

        if (!string.IsNullOrEmpty(_activity) && _executor != null)
        {
            var jobId = state.JobId;
            var userId = context.Items.TryGetValue("user_id", out var uid) ? uid?.ToString() ?? "anonymous" : "anonymous";
            var tier = context.Items.TryGetValue("customer_tier", out var tr) ? tr?.ToString() ?? "STANDARD" : "STANDARD";

            var auditPayload = new Dictionary<string, object>
            {
                ["job_id"] = jobId,
                ["route"] = path,
                ["method"] = context.Request.Method,
                ["status_code"] = statusCode,
                ["duration_ms"] = durationMs,
                ["user_id"] = userId,
                ["customer_tier"] = tier
            };

            if (_isAsync)
            {
                if (_asyncDispatcher != null)
                {
                    _asyncDispatcher.Dispatch(_activity, auditPayload, jobId, $"mw_post_{_activity.ToLowerInvariant()}", _executor);
                }
                else
                {
                    _ = Task.Run(async () =>
                    {
                        try
                        {
                            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(5));
                            await _executor.ExecuteActivityAsync(_activity, auditPayload, cts.Token);
                        }
                        catch { }
                    });
                }
            }
            else
            {
                try
                {
                    using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(5));
                    await _executor.ExecuteActivityAsync(_activity, auditPayload, cts.Token);
                }
                catch { }
            }
        }
    }
}

// --- G. Redis Cache Middleware (Declarative Response Caching with Sub-Millisecond Short-Circuiting) ---
public class RedisCacheMiddleware : IOctopusMiddleware
{
    private readonly int _ttlSeconds;
    private readonly List<string> _keyStrategy;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<string> _methods;
    private readonly string _headerName;
    private readonly string _prefix;
    private readonly bool _cacheControlHeaders;
    private readonly IConnectionMultiplexer? _redis;
    private readonly IDatabase? _db;
    private readonly ConcurrentDictionary<string, (string Content, DateTime Expiry)> _fallbackCache = new();
    private readonly RouteMatcher _matcher = new();

    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
    };

    public RedisCacheMiddleware(
        int ttlSeconds = 300,
        IEnumerable<string>? keyStrategy = null,
        IEnumerable<string>? excludes = null,
        IEnumerable<string>? includes = null,
        IEnumerable<string>? methods = null,
        string headerName = "X-Cache",
        string prefix = "octopus:cache",
        bool cacheControlHeaders = true,
        string redisUrl = "127.0.0.1:6379"
    )
    {
        _ttlSeconds = ttlSeconds > 0 ? ttlSeconds : 300;
        _keyStrategy = keyStrategy?.ToList() ?? new List<string> { "route", "claim:user_id", "query" };
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _methods = methods?.Select(m => m.ToUpperInvariant()).ToList() ?? new List<string> { "GET" };
        _headerName = !string.IsNullOrWhiteSpace(headerName) ? headerName : "X-Cache";
        _prefix = !string.IsNullOrWhiteSpace(prefix) ? prefix : "octopus:cache";
        _cacheControlHeaders = cacheControlHeaders;

        try
        {
            var connStr = redisUrl.Contains("localhost") ? redisUrl.Replace("localhost", "127.0.0.1") : redisUrl;
            var opts = ConfigurationOptions.Parse(connStr);
            opts.AbortOnConnectFail = false;
            opts.ConnectTimeout = 2000;
            _redis = ConnectionMultiplexer.Connect(opts);
            _db = _redis.GetDatabase();
        }
        catch
        {
            _redis = null;
            _db = null;
        }
    }

    private string BuildCacheKey(HttpContext context, string path)
    {
        var sb = new StringBuilder();
        sb.Append(_prefix);
        sb.Append(':');
        sb.Append(context.Request.Method.ToUpperInvariant());
        sb.Append(':');
        sb.Append(path);

        foreach (var strategy in _keyStrategy)
        {
            var sLower = strategy.ToLowerInvariant();
            if (sLower.StartsWith("claim:"))
            {
                var claimKey = strategy.Substring(6);
                string claimVal = "";
                if (context.Items.TryGetValue(claimKey, out var cv) && cv != null)
                {
                    claimVal = cv.ToString()!;
                }
                else if (context.Items.TryGetValue("auth_claims", out var ac) && ac is Dictionary<string, object> claimsDict && claimsDict.TryGetValue(claimKey, out var cv2) && cv2 != null)
                {
                    claimVal = cv2.ToString()!;
                }
                if (!string.IsNullOrEmpty(claimVal))
                {
                    sb.Append(':');
                    sb.Append(claimKey);
                    sb.Append('=');
                    sb.Append(claimVal);
                }
            }
            else if (sLower == "query")
            {
                var queryStr = context.Request.QueryString.Value;
                if (!string.IsNullOrEmpty(queryStr))
                {
                    sb.Append(":q=");
                    sb.Append(queryStr);
                }
            }
            else if (sLower.StartsWith("header:"))
            {
                var hdrKey = strategy.Substring(7);
                var hdrVal = context.Request.Headers[hdrKey].FirstOrDefault();
                if (!string.IsNullOrEmpty(hdrVal))
                {
                    sb.Append(':');
                    sb.Append(hdrKey);
                    sb.Append('=');
                    sb.Append(hdrVal);
                }
            }
        }

        return sb.ToString();
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var method = context.Request.Method.ToUpperInvariant();
        if (!_methods.Contains(method)) return true;

        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes)) return true;

        if (_cacheControlHeaders)
        {
            var cc = context.Request.Headers.CacheControl.ToString().ToLowerInvariant();
            if (cc.Contains("no-cache") || cc.Contains("no-store") || context.Request.Query.ContainsKey("no_cache") || context.Request.Query.ContainsKey("force_refresh"))
            {
                context.Response.Headers[_headerName] = "BYPASS";
                context.Items["cache_bypass"] = true;
                return true;
            }
        }

        var cacheKey = BuildCacheKey(context, path);
        context.Items["octopus_cache_key"] = cacheKey;

        string? cachedContent = null;
        TimeSpan? remainingTtl = null;

        if (_db != null)
        {
            try
            {
                var val = await _db.StringGetAsync(cacheKey);
                if (val.HasValue && !string.IsNullOrEmpty(val))
                {
                    cachedContent = (string)val!;
                    remainingTtl = await _db.KeyTimeToLiveAsync(cacheKey);
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"[RedisCacheMiddleware] Error getting key {cacheKey} from Redis: {ex.Message}");
            }
        }

        if (cachedContent == null && _fallbackCache.TryGetValue(cacheKey, out var fbItem))
        {
            if (fbItem.Expiry > DateTime.UtcNow)
            {
                cachedContent = fbItem.Content;
                remainingTtl = fbItem.Expiry - DateTime.UtcNow;
            }
            else
            {
                _fallbackCache.TryRemove(cacheKey, out _);
            }
        }

        if (cachedContent != null)
        {
            context.Response.Headers[_headerName] = "HIT";
            context.Response.Headers["X-Cache-Key"] = cacheKey;
            if (remainingTtl.HasValue)
            {
                context.Response.Headers["X-Cache-TTL"] = Math.Max(0, (int)remainingTtl.Value.TotalSeconds).ToString();
            }
            context.Response.Headers["X-Job-ID"] = $"job_cache_hit_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
            context.Response.Headers["X-Execution-Status"] = "COMPLETED";
            context.Response.StatusCode = 200;
            context.Response.ContentType = "application/json; charset=utf-8";

            await context.Response.WriteAsync(cachedContent);
            return false; // Short-circuit pipeline on cache hit!
        }

        context.Response.Headers[_headerName] = "MISS";
        context.Response.Headers["X-Cache-Key"] = cacheKey;
        return true;
    }

    public async Task ProcessPostAsync(HttpContext context, object? responseData, int statusCode, double durationMs, ExecutionState state)
    {
        if (statusCode != 200 || responseData == null) return;

        var method = context.Request.Method.ToUpperInvariant();
        if (!_methods.Contains(method)) return;

        var path = context.Request.Path.Value ?? "";
        if (_matcher.ShouldSkip(path, _excludes, _includes)) return;

        if (!context.Items.TryGetValue("octopus_cache_key", out var kObj) || kObj is not string cacheKey || string.IsNullOrEmpty(cacheKey))
        {
            cacheKey = BuildCacheKey(context, path);
        }

        try
        {
            string json;
            if (responseData is string s) json = s;
            else json = JsonSerializer.Serialize(responseData, JsonOpts);

            _fallbackCache[cacheKey] = (json, DateTime.UtcNow.AddSeconds(_ttlSeconds));

            if (_db != null)
            {
                await _db.StringSetAsync(cacheKey, json, TimeSpan.FromSeconds(_ttlSeconds));
            }
            Console.WriteLine($"[RedisCacheMiddleware] Successfully cached key: {cacheKey} (TTL: {_ttlSeconds}s)");
        }
        catch (Exception ex)
        {
            Console.WriteLine($"[RedisCacheMiddleware] Error caching response for key {cacheKey}: {ex.Message}");
        }
    }
}

// --- H. Payload & Input Validation Middleware (400 Bad Request) ---
public class PayloadValidationMiddleware : IOctopusMiddleware
{
    private readonly long _maxBodySizeBytes;
    private readonly List<string> _requiredHeaders;
    private readonly List<string> _requiredFields;
    private readonly Dictionary<string, List<string>> _requiredFieldsMap;
    private readonly List<string> _excludes;
    private readonly List<string> _includes;
    private readonly List<MiddlewareExceptionConfig> _exceptions;
    private readonly IActivityExecutor? _executor;
    private readonly IAsyncActivityDispatcher? _asyncDispatcher;

    public PayloadValidationMiddleware(
        long maxBodySizeBytes = 0,
        IEnumerable<string>? requiredHeaders = null,
        IEnumerable<string>? requiredFields = null,
        Dictionary<string, List<string>>? requiredFieldsMap = null,
        IEnumerable<string>? excludes = null,
        IEnumerable<string>? includes = null,
        IEnumerable<MiddlewareExceptionConfig>? exceptions = null,
        IActivityExecutor? executor = null,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        _maxBodySizeBytes = maxBodySizeBytes;
        _requiredHeaders = requiredHeaders?.ToList() ?? new List<string>();
        _requiredFields = requiredFields?.ToList() ?? new List<string>();
        _requiredFieldsMap = requiredFieldsMap != null ? new Dictionary<string, List<string>>(requiredFieldsMap, StringComparer.OrdinalIgnoreCase) : new Dictionary<string, List<string>>(StringComparer.OrdinalIgnoreCase);
        _excludes = excludes?.ToList() ?? new List<string>();
        _includes = includes?.ToList() ?? new List<string>();
        _exceptions = exceptions?.ToList() ?? new List<MiddlewareExceptionConfig>();
        _executor = executor;
        _asyncDispatcher = asyncDispatcher;
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var path = context.Request.Path.Value ?? "";
        if (RouteMatcher.ShouldSkipStatic(path, _excludes, _includes))
        {
            return true;
        }

        // 1. Validate Required Headers
        foreach (var reqHdr in _requiredHeaders)
        {
            if (string.IsNullOrWhiteSpace(reqHdr)) continue;
            if (!context.Request.Headers.ContainsKey(reqHdr) || string.IsNullOrWhiteSpace(context.Request.Headers[reqHdr]))
            {
                await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                    context,
                    new MissingRequiredFieldException($"Missing required header '{reqHdr}'"),
                    _exceptions,
                    _executor,
                    _asyncDispatcher
                );
                return false;
            }
        }

        // 2. Validate Body for Modifying Requests
        var method = context.Request.Method.ToUpperInvariant();
        if (method == "POST" || method == "PUT" || method == "PATCH")
        {
            context.Request.EnableBuffering();

            using var reader = new StreamReader(context.Request.Body, Encoding.UTF8, leaveOpen: true);
            var bodyText = await reader.ReadToEndAsync();
            context.Request.Body.Position = 0;

            var bodyBytesLength = Encoding.UTF8.GetByteCount(bodyText);

            if (_maxBodySizeBytes > 0 && bodyBytesLength > _maxBodySizeBytes)
            {
                await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                    context,
                    new PayloadTooLargeException(
                        $"Payload size of {bodyBytesLength} bytes exceeds limit of {_maxBodySizeBytes} bytes",
                        _maxBodySizeBytes,
                        bodyBytesLength
                    ),
                    _exceptions,
                    _executor,
                    _asyncDispatcher
                );
                return false;
            }

            var ct = context.Request.ContentType?.ToLowerInvariant() ?? "";
            var trimmed = bodyText.Trim();
            if (!string.IsNullOrEmpty(trimmed) && (ct.Contains("application/json") || trimmed.StartsWith("{") || trimmed.StartsWith("[")))
            {
                JsonDocument doc;
                try
                {
                    doc = JsonDocument.Parse(trimmed);
                }
                catch (Exception ex)
                {
                    await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                        context,
                        new MalformedJsonException($"Malformed JSON syntax in request body: {ex.Message}"),
                        _exceptions,
                        _executor,
                        _asyncDispatcher
                    );
                    return false;
                }

                // Resolve route-specific required fields or fallback to global list
                List<string>? effectiveRequiredFields = null;
                if (_requiredFieldsMap.Count > 0)
                {
                    foreach (var (routeKey, fields) in _requiredFieldsMap)
                    {
                        if (RouteMatcher.MatchesStatic(path, routeKey) || path.EndsWith(routeKey, StringComparison.OrdinalIgnoreCase) || routeKey.EndsWith(path, StringComparison.OrdinalIgnoreCase))
                        {
                            effectiveRequiredFields = fields;
                            break;
                        }
                    }
                }
                if (effectiveRequiredFields == null && _requiredFields.Count > 0)
                {
                    effectiveRequiredFields = _requiredFields;
                }

                if (effectiveRequiredFields != null && effectiveRequiredFields.Count > 0 && doc.RootElement.ValueKind == JsonValueKind.Object)
                {
                    foreach (var field in effectiveRequiredFields)
                    {
                        if (string.IsNullOrWhiteSpace(field)) continue;
                        if (!doc.RootElement.TryGetProperty(field, out var prop) ||
                            prop.ValueKind == JsonValueKind.Null ||
                            prop.ValueKind == JsonValueKind.Undefined ||
                            (prop.ValueKind == JsonValueKind.String && string.IsNullOrWhiteSpace(prop.GetString())))
                        {
                            await MiddlewareErrorResponder.RespondMiddlewareErrorAsync(
                                context,
                                new MissingRequiredFieldException($"Missing required field '{field}' in request payload."),
                                _exceptions,
                                _executor,
                                _asyncDispatcher
                            );
                            return false;
                        }
                    }
                }
            }
        }

        return true;
    }
}

// ============================================================================
// 5. Middleware Pipeline & Declarative Factory
// ============================================================================

public delegate IOctopusMiddleware CustomMiddlewareFactory(MiddlewareConfig config, IActivityExecutor? executor, IAsyncActivityDispatcher? asyncDispatcher = null);

public class MiddlewarePipeline
{
    private readonly List<IOctopusMiddleware> _middlewares = new();

    public void AddMiddleware(IOctopusMiddleware middleware)
    {
        _middlewares.Add(middleware);
    }

    public async Task<bool> ExecutePipelineAsync(HttpContext context)
    {
        foreach (var mw in _middlewares)
        {
            var canContinue = await mw.InvokeAsync(context);
            if (!canContinue)
            {
                return false;
            }
        }
        return true;
    }

    public async Task ProcessPostAsync(HttpContext context, object? responseData, int statusCode, double durationMs, ExecutionState state)
    {
        foreach (var mw in _middlewares)
        {
            if (mw is ResponseAuditMiddleware ram)
            {
                await ram.ProcessPostAsync(context, responseData, statusCode, durationMs, state);
            }
            else if (mw is RedisCacheMiddleware rcm)
            {
                await rcm.ProcessPostAsync(context, responseData, statusCode, durationMs, state);
            }
        }
    }
}

public static class MiddlewareFactory
{
    public static IOctopusMiddleware? BuildMiddlewareFromConfig(
        MiddlewareConfig cfg,
        IActivityExecutor? executor,
        Dictionary<string, CustomMiddlewareFactory>? customRegistry = null,
        IAsyncActivityDispatcher? asyncDispatcher = null
    )
    {
        var name = (cfg.Name ?? "").Trim();
        if (customRegistry != null && customRegistry.TryGetValue(name, out var factory))
        {
            return factory(cfg, executor, asyncDispatcher);
        }

        var normName = name.Replace("_", "").Replace("-", "").ToLowerInvariant();

        switch (normName)
        {
            case "bearerauthandclaimsmiddleware":
            case "bearerauth":
            case "bearerauthandclaims":
            case "jwtauthmiddleware":
            case "jwtauth":
                var jwtSecret = cfg.JwtSecret ?? GetString(cfg.Config, "jwt_secret", Environment.GetEnvironmentVariable("OCTOPUS_JWT_SECRET") ?? "");
                return new BearerAuthAndClaimsMiddleware(
                    string.IsNullOrEmpty(jwtSecret) ? null : token => Task.FromResult(Hs256Jwt.Verify(token, jwtSecret)),
                    null,
                    cfg.Excludes,
                    cfg.Includes,
                    cfg.Exceptions,
                    executor,
                    "Authorization",
                    asyncDispatcher
                );

            case "ratelimitmiddleware":
            case "ratelimit":
            case "ratelimiter":
                var limit = cfg.Limit ?? GetInt(cfg.Config, "limit", GetInt(cfg.Config, "default_limit", 60));
                var window = cfg.WindowSeconds ?? GetInt(cfg.Config, "window_seconds", GetInt(cfg.Config, "window", 60));
                var keyBy = cfg.KeyBy ?? GetString(cfg.Config, "key_by", "client_ip");
                var tierClaim = cfg.TierClaim ?? GetString(cfg.Config, "tier_claim", "customer_tier");
                var tierHeader = cfg.TierHeader ?? GetString(cfg.Config, "tier_header", "X-User-Tier");
                var tierLimits = cfg.TierLimits ?? GetMapInt(cfg.Config, "tier_limits");

                return new RateLimitMiddleware(
                    limit,
                    window,
                    keyBy,
                    tierClaim,
                    tierHeader,
                    tierLimits,
                    cfg.Excludes,
                    cfg.Includes,
                    cfg.Exceptions,
                    executor,
                    true,
                    asyncDispatcher
                );

            case "ipwhitelistmiddleware":
            case "ipwhitelist":
            case "ipfilter":
            case "ipfiltermiddleware":
                var allowed = cfg.AllowedIps ?? GetStringList(cfg.Config, "allowed_ips", GetStringList(cfg.Config, "whitelist", null));
                var blocked = cfg.BlockedIps ?? GetStringList(cfg.Config, "blocked_ips", GetStringList(cfg.Config, "blacklist", null));

                return new IpWhitelistMiddleware(
                    allowed,
                    blocked,
                    null,
                    cfg.Excludes,
                    cfg.Includes,
                    cfg.Exceptions,
                    executor,
                    asyncDispatcher
                );

            case "correlationidmiddleware":
            case "correlationid":
            case "requestidmiddleware":
            case "requestid":
                var hdr = GetString(cfg.Config, "header_name", "X-Correlation-ID");
                return new CorrelationIdMiddleware(hdr, cfg.Excludes, cfg.Includes);

            case "apikeyauthmiddleware":
            case "apikeyauth":
            case "apikey":
                var keys = cfg.ApiKeys ?? cfg.ValidKeys ?? GetStringList(cfg.Config, "api_keys", GetStringList(cfg.Config, "valid_keys", null));
                return new ApiKeyAuthMiddleware(keys, cfg.Excludes, cfg.Includes, cfg.Exceptions, executor, cfg.HeaderName ?? GetString(cfg.Config, "header_name", "X-API-Key"), asyncDispatcher);

            case "responseauditmiddleware":
            case "responseaudit":
            case "auditlogmiddleware":
            case "auditmiddleware":
                return new ResponseAuditMiddleware(
                    cfg.Activity ?? "RecordResponseAuditLog",
                    cfg.IsAsync || cfg.Async,
                    cfg.Excludes,
                    cfg.Includes,
                    executor,
                    asyncDispatcher
                );

            case "rediscachemiddleware":
            case "rediscache":
            case "cachemiddleware":
            case "cache":
                var ttl = cfg.TtlSeconds ?? GetInt(cfg.Config, "ttl_seconds", GetInt(cfg.Config, "ttl", 300));
                var keyStrat = cfg.KeyStrategy ?? GetStringList(cfg.Config, "key_strategy", null);
                var methods = cfg.Methods ?? GetStringList(cfg.Config, "methods", null);
                var hdrName = cfg.HeaderName ?? GetString(cfg.Config, "header_name", "X-Cache");
                var pfx = cfg.Prefix ?? GetString(cfg.Config, "prefix", "octopus:cache");
                var ccHdr = cfg.CacheControlHeaders ?? GetBool(cfg.Config, "cache_control_headers", true);
                var rUrl = cfg.RedisUrl ?? GetString(cfg.Config, "redis_url", GetString(cfg.Config, "dsn", "127.0.0.1:6379"));

                return new RedisCacheMiddleware(
                    ttl,
                    keyStrat,
                    cfg.Excludes,
                    cfg.Includes,
                    methods,
                    hdrName,
                    pfx,
                    ccHdr,
                    rUrl
                );

            case "payloadvalidationmiddleware":
            case "payloadvalidation":
            case "badrequestmiddleware":
            case "badrequest":
            case "validationmiddleware":
            case "validation":
            case "inputvalidationmiddleware":
                var maxSize = cfg.MaxBodySizeBytes ?? (long)GetInt(cfg.Config, "max_body_size_bytes", GetInt(cfg.Config, "max_size", 0));
                var reqHdrs = cfg.RequiredHeaders ?? GetStringList(cfg.Config, "required_headers", null);
                var rawReqFields = cfg.RequiredFields ?? (cfg.Config.TryGetValue("required_fields", out var rf) ? rf : null);
                var (reqFieldsList, reqFieldsMap) = ParseRequiredFields(rawReqFields);
                return new PayloadValidationMiddleware(
                    maxSize,
                    reqHdrs,
                    reqFieldsList,
                    reqFieldsMap,
                    cfg.Excludes,
                    cfg.Includes,
                    cfg.Exceptions,
                    executor,
                    asyncDispatcher
                );

            default:
                if (string.Equals(cfg.Type, "post-request", StringComparison.OrdinalIgnoreCase) && !string.IsNullOrEmpty(cfg.Activity))
                {
                    return new ResponseAuditMiddleware(
                        cfg.Activity,
                        cfg.IsAsync || cfg.Async,
                        cfg.Excludes,
                        cfg.Includes,
                        executor,
                        asyncDispatcher
                    );
                }
                return null;
        }
    }

    private static bool GetBool(Dictionary<string, object>? dict, string key, bool def)
    {
        if (dict != null && dict.TryGetValue(key, out var val) && val != null)
        {
            if (bool.TryParse(val.ToString(), out var b)) return b;
        }
        return def;
    }

    private static int GetInt(Dictionary<string, object>? dict, string key, int def)
    {
        if (dict != null && dict.TryGetValue(key, out var val) && val != null)
        {
            if (int.TryParse(val.ToString(), out var i)) return i;
        }
        return def;
    }

    private static string GetString(Dictionary<string, object>? dict, string key, string def)
    {
        if (dict != null && dict.TryGetValue(key, out var val) && val != null)
        {
            return val.ToString()!;
        }
        return def;
    }

    private static List<string>? GetStringList(Dictionary<string, object>? dict, string key, List<string>? def)
    {
        if (dict != null && dict.TryGetValue(key, out var val) && val != null)
        {
            if (val is List<string> ls) return ls;
            if (val is IEnumerable<object> lo) return lo.Select(o => o.ToString()!).ToList();
            if (val is string s) return new List<string> { s };
        }
        return def;
    }

    private static Dictionary<string, int>? GetMapInt(Dictionary<string, object>? dict, string key)
    {
        if (dict != null && dict.TryGetValue(key, out var val) && val != null)
        {
            if (val is Dictionary<string, int> di) return di;
            if (val is Dictionary<string, object> dObj)
            {
                var res = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
                foreach (var (k, v) in dObj)
                {
                    if (int.TryParse(v?.ToString(), out var parsed)) res[k] = parsed;
                }
                return res;
            }
        }
        return null;
    }

    private static (List<string>? list, Dictionary<string, List<string>>? map) ParseRequiredFields(object? raw)
    {
        if (raw == null) return (null, null);

        if (raw is List<string> ls) return (ls, null);
        if (raw is string[] sa) return (sa.ToList(), null);
        if (raw is string s) return (new List<string> { s }, null);

        if (raw is System.Collections.IDictionary dict)
        {
            var map = new Dictionary<string, List<string>>(StringComparer.OrdinalIgnoreCase);
            foreach (System.Collections.DictionaryEntry entry in dict)
            {
                var k = entry.Key?.ToString() ?? "";
                var valList = new List<string>();
                if (entry.Value is System.Collections.IEnumerable enumerable && !(entry.Value is string))
                {
                    foreach (var item in enumerable)
                    {
                        if (item != null) valList.Add(item.ToString()!);
                    }
                }
                else if (entry.Value != null)
                {
                    valList.Add(entry.Value.ToString()!);
                }
                map[k] = valList;
            }
            return (null, map);
        }

        if (raw is System.Collections.IEnumerable listObj)
        {
            var list = new List<string>();
            foreach (var item in listObj)
            {
                if (item != null) list.Add(item.ToString()!);
            }
            return (list, null);
        }

        return (null, null);
    }
}
