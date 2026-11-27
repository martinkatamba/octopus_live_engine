// Runs the conformance cases on the .NET SDK and prints the results as JSON.
//
//   dotnet run -- CASES_DIR [dotnet|core]
//
// "dotnet" (the default) runs the SDK's own engine and reports as "dotnet";
// "core" runs the SDK on octopus-core through its C ABI and reports as
// "dotnet-core", which checks the binding as well as the core.

using System.Reflection;
using System.Reflection.Emit;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Domain.Models;

var casesDir = args.Length > 0 ? args[0] : Path.Combine("..", "..", "cases");
var engineChoice = args.Length > 1 ? args[1] : "dotnet";

// stdout carries the results only: what the engine logs goes to stderr.
var stdout = Console.Out;
Console.SetOut(Console.Error);

var results = new JsonObject();
foreach (var dir in Directory.GetDirectories(casesDir).Order(StringComparer.Ordinal))
{
    if (!File.Exists(Path.Combine(dir, "case.json"))) continue;
    JsonArray runs;
    try
    {
        runs = await Runner.RunCase(dir, engineChoice);
    }
    catch (Exception e)
    {
        runs = new JsonArray(new JsonObject { ["load_error"] = $"{e.GetType().Name}: {e.Message}" });
    }
    results[Path.GetFileName(dir)] = runs;
}
var report = new JsonObject
{
    ["engine"] = engineChoice == "core" ? "dotnet-core" : "dotnet",
    ["cases"] = results,
};
stdout.WriteLine(report.ToJsonString());
stdout.Flush();

static class Runner
{
    static readonly TimeSpan AsyncWait = TimeSpan.FromSeconds(5);

    // The state as the REST API's status route writes it: snake_case, the status as text.
    static readonly JsonSerializerOptions StateJson = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter() },
    };

    static JsonNode? Dump(object? value) =>
        value == null ? null : JsonSerializer.SerializeToNode(value, value.GetType(), StateJson);

    public static async Task<JsonArray> RunCase(string dir, string engineChoice)
    {
        var testCase = JsonNode.Parse(File.ReadAllText(Path.Combine(dir, "case.json")))!.AsObject();
        var workflow = testCase["workflow"]!.GetValue<string>();
        var shared = testCase["shared_engine"]?.GetValue<bool>() ?? false;
        var runs = new JsonArray();
        var calls = new Calls();
        OctopusLiveEngine? engine = null;

        foreach (var runNode in testCase["runs"]!.AsArray())
        {
            var run = runNode!.AsObject();
            if (engine == null || !shared)
            {
                engine?.Dispose();
                engine = null;
                calls = new Calls();
                try
                {
                    engine = NewEngine(dir, testCase, engineChoice, calls);
                }
                catch (Exception e)
                {
                    runs.Add(new JsonObject { ["load_error"] = $"{e.GetType().Name}: {e.Message}" });
                    continue;
                }
            }

            if (run["diagram"]?.GetValue<bool>() == true)
            {
                runs.Add(new JsonObject { ["diagram"] = await Diagram(engine, workflow) });
                continue;
            }

            var input = Plain.Object(run["input"]?.AsObject());
            JsonArray? events = null;
            Task? listener = null;
            using var stopListening = new CancellationTokenSource();
            if (run["events"]?.GetValue<bool>() == true)
            {
                events = new JsonArray();
                var jobId = input.TryGetValue("job_id", out var j) ? j?.ToString() ?? "" : "";
                listener = CollectEvents(engine, jobId, events, stopListening.Token);
                await Task.Delay(50); // subscribed before the run starts
            }

            JsonNode? state = null;
            string? error = null;
            var jobsBefore = await JobIds(engine, workflow);
            try
            {
                state = Dump(await engine.ExecuteSyncAsync(workflow, input));
            }
            catch (Exception e)
            {
                // A failed run throws (the activity's own exception, as .NET code
                // expects) after the engine has stored the job; its state is that job.
                error = $"{e.GetType().Name}: {e.Message}";
                var jobId = input.TryGetValue("job_id", out var given) && given is string s && s != ""
                    ? s
                    : (await JobIds(engine, workflow)).Except(jobsBefore).FirstOrDefault();
                if (jobId != null) state = Dump(await engine.Workflows.GetExecutionStateAsync(jobId));
            }

            JsonNode? stored = null;
            var stateJobId = state?["job_id"]?.GetValue<string>();
            if (!string.IsNullOrEmpty(stateJobId))
            {
                stored = await StoredState(engine, stateJobId, run["wait_async"]?.GetValue<bool>() ?? false);
            }

            if (listener != null)
            {
                // The stream ends at the job's last workflow event.
                await Task.WhenAny(listener, Task.Delay(1000));
                stopListening.Cancel();
                try { await listener; } catch (OperationCanceledException) { }
            }

            var result = new JsonObject
            {
                ["events"] = events,
                ["state"] = state,
                ["stored"] = stored,
                ["error"] = error,
            };
            calls.WriteTo(result);
            runs.Add(result);
        }
        engine?.Dispose();
        return runs;
    }

    static OctopusLiveEngine NewEngine(string dir, JsonObject testCase, string engineChoice, Calls calls)
    {
        var engine = new OctopusLiveEngine(engine: engineChoice);
        try
        {
            engine.LoadBlueprintFile(Path.Combine(dir, "blueprint.yaml"));
            foreach (var (name, script) in testCase["activities"]!.AsObject())
            {
                var entries = script!.AsArray().Select(e => e!.AsObject()).ToList();
                engine.RegisterActivity(name, async (input, _) =>
                {
                    var n = calls.Record(name);
                    var entry = entries[Math.Min(n, entries.Count - 1)];
                    if (entry["delay_ms"] is { } delay) await Task.Delay(delay.GetValue<int>());
                    if (entry["fail"] is { } fail) throw TypedErrors.Make(entry, fail.GetValue<string>());
                    var output = new Dictionary<string, object>();
                    foreach (var (key, source) in entry["echo"]?.AsObject() ?? new JsonObject())
                    {
                        output[key] = input.TryGetValue(source!.GetValue<string>(), out var v) ? v : null!;
                    }
                    foreach (var (key, value) in Plain.Object(entry["return"]?.AsObject()))
                    {
                        output[key] = value;
                    }
                    return output;
                });
            }
            return engine;
        }
        catch
        {
            engine.Dispose();
            throw;
        }
    }

    /// <summary>The job as the state store holds it; with waitAsync, once no step is still ASYNC_DISPATCHED.</summary>
    static async Task<JsonNode?> StoredState(OctopusLiveEngine engine, string jobId, bool waitAsync)
    {
        var deadline = DateTime.UtcNow + AsyncWait;
        while (true)
        {
            ExecutionState? st;
            try
            {
                st = await engine.Workflows.GetExecutionStateAsync(jobId);
            }
            catch
            {
                return null;
            }
            if (st == null) return null;
            var pending = st.Traces.Any(t => t.Status == "ASYNC_DISPATCHED");
            if (!waitAsync || !pending || DateTime.UtcNow > deadline) return Dump(st);
            await Task.Delay(20);
        }
    }

    static async Task<HashSet<string>> JobIds(OctopusLiveEngine engine, string workflow) =>
        (await engine.Workflows.ListExecutionStatesAsync(workflow, 1000)).Select(s => s.JobId).ToHashSet();

    static async Task CollectEvents(OctopusLiveEngine engine, string jobId, JsonArray events, CancellationToken ct)
    {
        await foreach (var ev in engine.Workflows.StreamEventsAsync(jobId, ct))
        {
            lock (events)
            {
                events.Add(new JsonObject { ["event"] = ev.Event, ["data"] = Dump(ev.Data) });
            }
        }
    }

    /// <summary>The visualizer graph with nothing named: {"nodes": [node ids]}.</summary>
    static async Task<JsonObject> Diagram(OctopusLiveEngine engine, string workflow)
    {
        var ids = new JsonArray();
        try
        {
            var graph = Dump(await engine.CompileX6DiagramAsync(workflow));
            foreach (var node in graph?["nodes"]?.AsArray() ?? new JsonArray())
            {
                ids.Add(node?["id"]?.DeepClone());
            }
        }
        catch
        {
            // No graph: no nodes.
        }
        return new JsonObject { ["nodes"] = ids };
    }
}

/// <summary>Each activity's call count, and every call in order.</summary>
sealed class Calls
{
    private readonly Dictionary<string, int> _counts = new();
    private readonly List<string> _sequence = new();

    public int Record(string name)
    {
        lock (this)
        {
            var n = _counts.GetValueOrDefault(name);
            _counts[name] = n + 1;
            _sequence.Add(name);
            return n;
        }
    }

    public void WriteTo(JsonObject result)
    {
        lock (this)
        {
            var counts = new JsonObject();
            foreach (var (k, v) in _counts) counts[k] = v;
            result["calls"] = counts;
            result["sequence"] = new JsonArray(_sequence.Select(s => (JsonNode?)s).ToArray());
        }
    }
}

/// <summary>JSON as the plain values an activity sees: long, double, string, bool, lists and dictionaries.</summary>
static class Plain
{
    public static Dictionary<string, object> Object(JsonObject? o)
    {
        var d = new Dictionary<string, object>();
        foreach (var (k, v) in o ?? new JsonObject()) d[k] = Value(v)!;
        return d;
    }

    static object? Value(JsonNode? n) => n switch
    {
        null => null,
        JsonObject o => Object(o),
        JsonArray a => a.Select(Value).ToList(),
        JsonValue v when v.GetValueKind() == JsonValueKind.String => v.GetValue<string>(),
        JsonValue v when v.GetValueKind() == JsonValueKind.True => true,
        JsonValue v when v.GetValueKind() == JsonValueKind.False => false,
        JsonValue v when v.TryGetValue<long>(out var l) => l,
        JsonValue v when v.GetValueKind() == JsonValueKind.Number => v.GetValue<double>(),
        _ => null,
    };
}

/// <summary>
/// The script's failure. With error_type, an exception of a class by that name
/// whose base classes are error_bases, most specific first (D14).
/// </summary>
static class TypedErrors
{
    private static readonly ModuleBuilder Module = AssemblyBuilder
        .DefineDynamicAssembly(new AssemblyName("ConformanceErrors"), AssemblyBuilderAccess.Run)
        .DefineDynamicModule("ConformanceErrors");
    private static readonly Dictionary<string, Type> Types = new();

    public static Exception Make(JsonObject entry, string message)
    {
        if (entry["error_type"] is not { } typeName) return new InvalidOperationException(message);
        var parent = typeof(Exception);
        var bases = entry["error_bases"]?.AsArray().Select(b => b!.GetValue<string>()).Reverse() ?? [];
        foreach (var name in bases.Append(typeName.GetValue<string>()))
        {
            parent = Define(name, parent);
        }
        return (Exception)Activator.CreateInstance(parent, message)!;
    }

    // A class named `name` deriving from `parent`, with a (string message) constructor.
    // Each (name, parent) pair gets its own namespace, so Type.Name stays `name`.
    private static Type Define(string name, Type parent)
    {
        lock (Types)
        {
            var key = $"{name}<{parent.FullName}";
            if (Types.TryGetValue(key, out var existing)) return existing;
            var tb = Module.DefineType($"E{Types.Count}.{name}", TypeAttributes.Public | TypeAttributes.Class, parent);
            var ctor = tb.DefineConstructor(MethodAttributes.Public, CallingConventions.Standard, [typeof(string)]);
            var il = ctor.GetILGenerator();
            il.Emit(OpCodes.Ldarg_0);
            il.Emit(OpCodes.Ldarg_1);
            il.Emit(OpCodes.Call, parent.GetConstructor(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance, [typeof(string)])!);
            il.Emit(OpCodes.Ret);
            var type = tb.CreateType();
            Types[key] = type;
            return type;
        }
    }
}
