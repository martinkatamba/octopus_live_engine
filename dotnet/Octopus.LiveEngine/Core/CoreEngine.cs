using System.Collections.Concurrent;
using System.Runtime.ExceptionServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;

namespace Octopus.LiveEngine.Core;

/// <summary>A call into the core failed. <see cref="State"/> is the job's state when the run got that far.</summary>
public class OctopusCoreException : InvalidOperationException
{
    public int Code { get; }
    public string Kind { get; }
    public ExecutionState? State { get; }

    public OctopusCoreException(int code, string kind, string message, ExecutionState? state = null)
        : base(message)
    {
        Code = code;
        Kind = kind;
        State = state;
    }
}

/// <summary>A workflow run on the core failed and no activity exception explains it (a terminate step, a refused rule, ...).</summary>
public class OctopusWorkflowException : OctopusCoreException
{
    public OctopusWorkflowException(string kind, string message, ExecutionState? state)
        : base(NativeMethods.ErrWorkflow, kind, message, state) { }
}

/// <summary>The native engine handle; frees the engine and the callbacks' handle.</summary>
internal sealed class EngineHandle : SafeHandle
{
    public GCHandle Bridge;

    public EngineHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public override bool IsInvalid => handle == IntPtr.Zero;

    public void Set(IntPtr engine) => SetHandle(engine);

    protected override bool ReleaseHandle()
    {
        NativeMethods.octopus_engine_free(handle);
        if (Bridge.IsAllocated) Bridge.Free();
        return true;
    }
}

/// <summary>A built-in middleware of the core; frees it.</summary>
internal sealed class CoreMiddlewareHandle : SafeHandle
{
    public CoreMiddlewareHandle() : base(IntPtr.Zero, ownsHandle: true) { }

    public override bool IsInvalid => handle == IntPtr.Zero;

    public void Set(IntPtr mw) => SetHandle(mw);

    protected override bool ReleaseHandle()
    {
        NativeMethods.octopus_middleware_free(handle);
        return true;
    }
}

/// <summary>
/// One engine of the core (octopus-ffi). Activities run through the SDK's
/// <see cref="IActivityExecutor"/>, called back from the core's threads.
/// </summary>
internal sealed unsafe class CoreEngine : IDisposable
{
    private readonly EngineHandle _handle = new();
    private readonly ActivityBridge _bridge;

    /// <summary>Whether the native library loads and speaks this SDK's contract; <paramref name="reason"/> says why not.</summary>
    public static bool IsAvailable(out string reason)
    {
        try
        {
            var contract = NativeMethods.octopus_contract_version();
            if (contract != NativeMethods.ExpectedContractVersion)
            {
                reason = $"the octopus_ffi library has contract version {contract}, this SDK needs {NativeMethods.ExpectedContractVersion}";
                return false;
            }
            reason = "";
            return true;
        }
        catch (Exception ex) when (ex is DllNotFoundException or EntryPointNotFoundException or BadImageFormatException)
        {
            reason = $"the octopus_ffi library is not available ({ex.Message}); set {NativeMethods.LibraryEnvVar} to its path";
            return false;
        }
    }

    public static string Version => NativeMethods.Take(NativeMethods.octopus_core_version());

    public CoreEngine(IActivityExecutor executor, object? config = null)
    {
        _bridge = new ActivityBridge(executor);
        var engine = NativeMethods.octopus_engine_new(config == null ? null : CoreJson.Write(config));
        if (engine == IntPtr.Zero) throw LastError(NativeMethods.ErrEngine);
        _handle.Set(engine);
        _handle.Bridge = GCHandle.Alloc(_bridge, GCHandleType.Weak);
        var bridge = GCHandle.ToIntPtr(_handle.Bridge);
        Check(NativeMethods.octopus_set_activity_resolver(
            engine,
            (delegate* unmanaged<IntPtr, byte*, nuint, byte*, nuint, void*, int>)&ActivityBridge.Call,
            (delegate* unmanaged<IntPtr, byte*, nuint, byte>)&ActivityBridge.Has,
            bridge));
    }

    public void Dispose() => _handle.Dispose();

    private IntPtr Ptr
    {
        get
        {
            ObjectDisposedException.ThrowIf(_handle.IsClosed, this);
            return _handle.DangerousGetHandle();
        }
    }

    public void RegisterBlueprint(string yamlOrJson)
    {
        Check(NativeMethods.octopus_register_blueprint(Ptr, yamlOrJson));
        GC.KeepAlive(this);
    }

    /// <summary>
    /// Runs a workflow (as a declared endpoint when <paramref name="endpointIndex"/>
    /// is 0 or more, or a route or method is given). A failed run throws the
    /// exception its activity threw, as the .NET engine does, or else an
    /// <see cref="OctopusWorkflowException"/>; either way the core has stored the job.
    /// </summary>
    public ExecutionState Execute(string workflow, Dictionary<string, object> payload, long endpointIndex = -1, string? route = null, string? method = null)
    {
        var status = NativeMethods.octopus_execute_endpoint(
            Ptr, workflow, endpointIndex, route, method, CoreJson.Write(payload), 0, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        var state = CoreJson.ParseState(text);
        if (status == NativeMethods.OK) return state!;
        if (status == NativeMethods.ErrWorkflow)
        {
            var err = LastErrorJson();
            if (state != null && _bridge.TakeFailure(state.JobId) is { } failure) failure.Throw();
            throw new OctopusWorkflowException(err.Kind, err.Message, state);
        }
        throw LastError(status, state);
    }

    public ExecutionState InitiateAsync(string workflow, Dictionary<string, object> payload)
    {
        var status = NativeMethods.octopus_initiate_async(Ptr, workflow, CoreJson.Write(payload), 0, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        Check(status);
        return CoreJson.ParseState(text)!;
    }

    public ExecutionState? GetState(string jobId)
    {
        var status = NativeMethods.octopus_get_state(Ptr, jobId, 0, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        if (status == NativeMethods.ErrNotFound) return null;
        Check(status);
        return CoreJson.ParseState(text);
    }

    public List<ExecutionState> ListStates(string? workflow, int limit, bool tracePayloads = true)
    {
        var flags = tracePayloads ? 0u : NativeMethods.StateNoTracePayloads;
        var status = NativeMethods.octopus_list_states(Ptr, workflow, (nuint)Math.Max(limit, 0), flags, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        Check(status);
        return CoreJson.ParseStates(text);
    }

    public void SaveState(ExecutionState state)
    {
        Check(NativeMethods.octopus_save_state(Ptr, CoreJson.WriteState(state)));
        GC.KeepAlive(this);
    }

    public void StartWorker()
    {
        Check(NativeMethods.octopus_start_worker(Ptr));
        GC.KeepAlive(this);
    }

    public void StopWorker()
    {
        if (_handle.IsClosed) return;
        Check(NativeMethods.octopus_stop_worker(Ptr));
        GC.KeepAlive(this);
    }

    // --- Middleware, access and the visualizer ---

    /// <summary>
    /// The core's built-in for the <paramref name="index"/>-th middleware of a
    /// registered blueprint, or null when the core has no built-in of that name.
    /// </summary>
    public CoreMiddlewareHandle? BlueprintMiddleware(string workflow, int index)
    {
        var status = NativeMethods.octopus_blueprint_middleware_new(Ptr, workflow, (nuint)index, out var mw);
        GC.KeepAlive(this);
        if (status == NativeMethods.ErrNotFound) return null;
        Check(status);
        var handle = new CoreMiddlewareHandle();
        handle.Set(mw);
        return handle;
    }

    /// <summary>The client's IP; forwarded headers count only from the engine's trusted proxies (D11).</summary>
    public string ClientIp(string? peer, string? forwardedFor, string? realIp)
    {
        var status = NativeMethods.octopus_client_ip(Ptr, peer, forwardedFor, realIp, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        Check(status);
        return text;
    }

    /// <summary>The core's decision on a tooling request: allow, not_found, ask_authorizer, require_api_key or deny (with a detail).</summary>
    public (string Decision, string Detail) ToolingDecision(bool hasAuthorizer, bool clientIsLoopback)
    {
        var request = CoreJson.Write(new Dictionary<string, object>
        {
            ["has_authorizer"] = hasAuthorizer,
            ["client_is_loopback"] = clientIsLoopback,
        });
        var status = NativeMethods.octopus_tooling_decision(Ptr, request, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        Check(status);
        var d = CoreJson.ParseObject(text);
        return (d.TryGetValue("decision", out var dec) ? dec?.ToString() ?? "deny" : "deny",
                d.TryGetValue("detail", out var det) ? det?.ToString() ?? "" : "");
    }

    /// <summary>Whether the key opens the engine's protected routes; always true when the engine has no key.</summary>
    public bool ApiKeyAllows(string? header, string? query)
    {
        var allowed = NativeMethods.octopus_api_key_allows(Ptr, header, query);
        GC.KeepAlive(this);
        return allowed;
    }

    /// <summary>The visualizer's graph of a workflow (of one declared endpoint when a route or method is given), or null for an unknown workflow.</summary>
    public string? CompileDiagram(string workflow, string? jobId, string? route, string? method)
    {
        var status = NativeMethods.octopus_compile_endpoint_diagram(Ptr, workflow, jobId, route, method, out var buf);
        var text = NativeMethods.Take(buf);
        GC.KeepAlive(this);
        if (status == NativeMethods.ErrNotFound) return null;
        Check(status);
        return text;
    }

    /// <summary>The core's visualizer page, its step inspector naming .NET's activities as C#.</summary>
    public static string VisualizerPage(string routePrefix, string defaultDomain)
    {
        var status = NativeMethods.octopus_visualizer_page(routePrefix, defaultDomain, "C#", out var buf);
        var text = NativeMethods.Take(buf);
        Check(status);
        return text;
    }

    /// <summary>Calls <paramref name="onEvent"/> with each live event of the job, on a core thread, until disposed.</summary>
    public IDisposable Subscribe(string jobId, Action<LiveEvent> onEvent)
    {
        var sub = new Subscription(this, onEvent);
        sub.Handle = GCHandle.Alloc(sub);
        var id = NativeMethods.octopus_subscribe_events(
            Ptr, jobId, (delegate* unmanaged<IntPtr, byte*, nuint, void>)&Subscription.OnEvent, GCHandle.ToIntPtr(sub.Handle));
        GC.KeepAlive(this);
        if (id == 0)
        {
            sub.Handle.Free();
            throw LastError(NativeMethods.ErrEngine);
        }
        sub.Id = id;
        return sub;
    }

    private sealed class Subscription : IDisposable
    {
        private readonly CoreEngine _engine;
        private readonly Action<LiveEvent> _onEvent;
        public GCHandle Handle;
        public ulong Id;
        private int _disposed;

        public Subscription(CoreEngine engine, Action<LiveEvent> onEvent)
        {
            _engine = engine;
            _onEvent = onEvent;
        }

        [UnmanagedCallersOnly]
        public static void OnEvent(IntPtr userData, byte* ev, nuint len)
        {
            try
            {
                if (GCHandle.FromIntPtr(userData).Target is not Subscription sub) return;
                using var doc = JsonDocument.Parse(new ReadOnlySpan<byte>(ev, checked((int)len)).ToArray());
                var root = doc.RootElement;
                var name = root.TryGetProperty("event", out var e) ? e.GetString() ?? "" : "";
                object data = root.TryGetProperty("data", out var d) ? CoreJson.ToPlain(d) ?? new Dictionary<string, object>() : new Dictionary<string, object>();
                sub._onEvent(new LiveEvent(name, data));
            }
            catch
            {
                // An exception must not cross into the core; a bad event is dropped.
            }
        }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0) return;
            if (!_engine._handle.IsClosed)
            {
                NativeMethods.octopus_unsubscribe(_engine._handle.DangerousGetHandle(), Id);
            }
            if (Handle.IsAllocated) Handle.Free();
        }
    }

    // --- Errors ---

    private readonly record struct ErrorInfo(int Code, string Kind, string Message);

    private static ErrorInfo LastErrorJson()
    {
        var text = NativeMethods.Take(NativeMethods.octopus_last_error());
        if (string.IsNullOrEmpty(text)) return new ErrorInfo(0, "EngineError", "the core reported a failure without an error");
        using var doc = JsonDocument.Parse(text);
        var r = doc.RootElement;
        return new ErrorInfo(
            r.TryGetProperty("code", out var c) && c.TryGetInt32(out var ci) ? ci : 0,
            r.TryGetProperty("type", out var t) ? t.GetString() ?? "" : "",
            r.TryGetProperty("message", out var m) ? m.GetString() ?? "" : "");
    }

    internal static OctopusCoreException LastError(int status, ExecutionState? state = null)
    {
        var err = LastErrorJson();
        return new OctopusCoreException(status, err.Kind, err.Message, state);
    }

    private static void Check(int status)
    {
        if (status != NativeMethods.OK) throw LastError(status);
    }

    // --- Activities ---

    /// <summary>Runs the core's activity calls through the SDK's executor.</summary>
    private sealed class ActivityBridge
    {
        private readonly IActivityExecutor _executor;
        private readonly ConcurrentDictionary<string, ExceptionDispatchInfo> _failures = new();

        public ActivityBridge(IActivityExecutor executor) => _executor = executor;

        /// <summary>The exception an activity of the job threw last, if any; forgets it.</summary>
        public ExceptionDispatchInfo? TakeFailure(string jobId) =>
            _failures.TryRemove(jobId, out var f) ? f : null;

        [UnmanagedCallersOnly]
        public static int Call(IntPtr userData, byte* name, nuint nameLen, byte* input, nuint inputLen, void* output)
        {
            string text;
            int status;
            try
            {
                if (GCHandle.FromIntPtr(userData).Target is not ActivityBridge bridge)
                {
                    text = "the engine was disposed";
                    status = 1;
                }
                else
                {
                    (status, text) = bridge.Run(
                        Encoding.UTF8.GetString(name, checked((int)nameLen)),
                        Encoding.UTF8.GetString(input, checked((int)inputLen)));
                }
            }
            catch (Exception ex)
            {
                (status, text) = (1, ex.Message);
            }
            var bytes = Encoding.UTF8.GetBytes(text);
            fixed (byte* p = bytes)
            {
                NativeMethods.octopus_out_write(output, p, (nuint)bytes.Length);
            }
            return status;
        }

        [UnmanagedCallersOnly]
        public static byte Has(IntPtr userData, byte* name, nuint nameLen)
        {
            try
            {
                return GCHandle.FromIntPtr(userData).Target is ActivityBridge bridge
                    && bridge._executor.HasActivity(Encoding.UTF8.GetString(name, checked((int)nameLen)))
                    ? (byte)1 : (byte)0;
            }
            catch
            {
                return 0;
            }
        }

        private (int, string) Run(string name, string inputJson)
        {
            var input = CoreJson.ParseObject(inputJson);
            try
            {
                // The core's threads have no synchronization context, so blocking is safe here.
                var result = _executor.ExecuteActivityAsync(name, input).GetAwaiter().GetResult();
                return (NativeMethods.OK, CoreJson.Write(result ?? new Dictionary<string, object>()));
            }
            catch (Exception ex)
            {
                if (input.TryGetValue("job_id", out var job) && job?.ToString() is { Length: > 0 } jobId)
                {
                    _failures[jobId] = ExceptionDispatchInfo.Capture(ex);
                }
                return (1, ErrorText(ex));
            }
        }

        /// <summary>
        /// The core's error form, <c>[Type,Base,...:code] message</c>: it then records
        /// the type and message apart, and an <c>exceptions</c> entry naming any base
        /// class matches (decision D14). Code 0 means no HTTP status.
        /// </summary>
        internal static string ErrorText(Exception ex)
        {
            var types = new List<string>();
            for (var t = ex.GetType(); t != null && t != typeof(object); t = t.BaseType) types.Add(t.Name);
            var code = 0;
            foreach (var prop in new[] { "StatusCode", "Code" })
            {
                var v = ex.GetType().GetProperty(prop)?.GetValue(ex);
                if (v is int i && i is >= 100 and < 600)
                {
                    code = i;
                    break;
                }
            }
            return $"[{string.Join(",", types)}:{code}] {ex.Message}";
        }
    }
}
