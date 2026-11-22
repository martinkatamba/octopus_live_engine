using System.Text;
using Microsoft.AspNetCore.Http;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Services;

namespace Octopus.LiveEngine.Core;

/// <summary>A middleware that also acts once the handler has answered (a response audit, a cache).</summary>
public interface IPostResponseMiddleware
{
    Task ProcessPostAsync(HttpContext context, object? responseData, int statusCode, double durationMs, ExecutionState state);
}

/// <summary>
/// A built-in middleware run by the core (decision D11): the core makes the check,
/// applies the blueprint's <c>exceptions:</c> and runs their activities; this class
/// moves the request to it and applies its decision (headers, request context, or
/// the answer that stops the request).
/// </summary>
internal sealed class CoreMiddleware : IOctopusMiddleware, IPostResponseMiddleware
{
    // What middleware and handlers leave on the request, as the core names it.
    private static readonly string[] ContextKeys = { "auth_claims", "workflow_injections", "correlation_id", "job_id" };

    private readonly CoreMiddlewareHandle _handle;
    private readonly CorePipelineManager _pipeline;
    private const string RequestKey = "octopus_core_middleware_request";

    public string Name { get; }

    internal CoreMiddleware(string name, CoreMiddlewareHandle handle, CorePipelineManager pipeline)
    {
        Name = name;
        _handle = handle;
        _pipeline = pipeline;
    }

    public async Task<bool> InvokeAsync(HttpContext context)
    {
        var request = await RequestAsync(context);
        var status = NativeMethods.octopus_middleware_run_before(Ptr, CoreJson.Write(request), out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(_handle);
        if (status != NativeMethods.OK) throw CoreEngine.LastError(status);

        var decision = CoreJson.ParseObject(text);
        if (decision.TryGetValue("headers", out var h) && h is Dictionary<string, object> headers)
        {
            foreach (var (k, v) in headers) context.Response.Headers[k] = v?.ToString() ?? "";
        }
        if (decision.TryGetValue("respond", out var r) && r is Dictionary<string, object> respond)
        {
            context.Response.StatusCode = respond.TryGetValue("status", out var s) ? Convert.ToInt32(s) : 500;
            context.Response.ContentType = respond.TryGetValue("content_type", out var ct) ? ct?.ToString() : "application/json";
            await context.Response.WriteAsync(respond.TryGetValue("body", out var b) ? b?.ToString() ?? "" : "");
            return false;
        }
        if (decision.TryGetValue("context", out var c) && c is Dictionary<string, object> values)
        {
            foreach (var (k, v) in values) context.Items[k] = v;
        }
        context.Items[RequestKey + ":" + Name] = request;
        return true;
    }

    public Task ProcessPostAsync(HttpContext context, object? responseData, int statusCode, double durationMs, ExecutionState state)
    {
        if (context.Items[RequestKey + ":" + Name] is not Dictionary<string, object> request) return Task.CompletedTask;
        // What the handler left on the request, as the core's `after` reads it.
        var ctx = Context(context);
        ctx.TryAdd("job_id", state.JobId);
        if (ctx.TryGetValue("auth_claims", out var cl) && cl is Dictionary<string, object> claims)
        {
            if ((claims.GetValueOrDefault("user_id") ?? claims.GetValueOrDefault("sub")) is { } user) ctx.TryAdd("user_id", user);
            if (claims.GetValueOrDefault("customer_tier") is { } tier) ctx.TryAdd("customer_tier", tier);
        }
        request["context"] = ctx;
        var response = new Dictionary<string, object>
        {
            ["status"] = statusCode,
            ["body"] = responseData is string s ? s : CoreJson.Write(responseData),
            ["duration_ms"] = durationMs,
        };
        var status = NativeMethods.octopus_middleware_run_after(Ptr, CoreJson.Write(request), CoreJson.Write(response));
        GC.KeepAlive(_handle);
        if (status != NativeMethods.OK) throw CoreEngine.LastError(status);
        return Task.CompletedTask;
    }

    private IntPtr Ptr
    {
        get
        {
            ObjectDisposedException.ThrowIf(_handle.IsClosed, this);
            return _handle.DangerousGetHandle();
        }
    }

    private static Dictionary<string, object> Context(HttpContext context)
    {
        var values = new Dictionary<string, object>();
        foreach (var key in ContextKeys)
        {
            if (context.Items.TryGetValue(key, out var v) && v != null) values[key] = v;
        }
        return values;
    }

    private async Task<Dictionary<string, object>> RequestAsync(HttpContext context)
    {
        var req = context.Request;
        var headers = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        foreach (var (k, v) in req.Headers) headers[k] = v.ToString();
        var query = new Dictionary<string, object>();
        foreach (var (k, v) in req.Query) query[k] = v.Select(x => (object)(x ?? "")).ToList();

        var body = "";
        if (HttpMethods.IsPost(req.Method) || HttpMethods.IsPut(req.Method) || HttpMethods.IsPatch(req.Method))
        {
            // The handler reads the body again after the chain.
            req.EnableBuffering();
            using var reader = new StreamReader(req.Body, Encoding.UTF8, leaveOpen: true);
            body = await reader.ReadToEndAsync(context.RequestAborted);
            req.Body.Position = 0;
        }

        return new Dictionary<string, object>
        {
            ["method"] = req.Method,
            ["path"] = req.Path.Value ?? "",
            ["headers"] = headers,
            ["query"] = query,
            ["client_ip"] = _pipeline.ClientIp(context),
            ["body"] = body,
            ["context"] = Context(context),
        };
    }
}
