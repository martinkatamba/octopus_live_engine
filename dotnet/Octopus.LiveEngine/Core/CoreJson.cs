using System.Collections;
using System.Globalization;
using System.Text;
using System.Text.Json;
using Octopus.LiveEngine.Domain.Models;

namespace Octopus.LiveEngine.Core;

/// <summary>
/// JSON between the SDK's values and the core. Read values take the shapes
/// the routes give a request body (<c>UnwrapJsonElement</c>): case-insensitive
/// <c>Dictionary&lt;string, object&gt;</c>, <c>List&lt;object&gt;</c>,
/// <c>long</c> or else <c>double</c>, <c>string</c>, <c>bool</c>, null.
/// </summary>
internal static class CoreJson
{
    // --- Writing ---

    public static string Write(object? value)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            WriteValue(writer, value, 0);
        }
        return Encoding.UTF8.GetString(stream.GetBuffer(), 0, (int)stream.Length);
    }

    private static void WriteValue(Utf8JsonWriter w, object? value, int depth)
    {
        if (depth > 128) throw new InvalidOperationException("the value nests too deeply to write as JSON");
        switch (value)
        {
            case null:
                w.WriteNullValue();
                return;
            case string s:
                w.WriteStringValue(s);
                return;
            case bool b:
                w.WriteBooleanValue(b);
                return;
            case JsonElement el:
                el.WriteTo(w);
                return;
            case JsonDocument doc:
                doc.RootElement.WriteTo(w);
                return;
            case int or long or short or sbyte or byte or ushort or uint:
                w.WriteNumberValue(Convert.ToInt64(value, CultureInfo.InvariantCulture));
                return;
            case ulong ul:
                w.WriteNumberValue(ul);
                return;
            case float or double:
                var d = Convert.ToDouble(value, CultureInfo.InvariantCulture);
                if (double.IsFinite(d)) w.WriteNumberValue(d);
                else w.WriteNullValue();
                return;
            case decimal m:
                w.WriteNumberValue(m);
                return;
            case DateTime dt:
                w.WriteStringValue(dt.ToString("o", CultureInfo.InvariantCulture));
                return;
            case DateTimeOffset dto:
                w.WriteStringValue(dto.ToString("o", CultureInfo.InvariantCulture));
                return;
            case Guid or Enum or char or TimeSpan or Uri:
                w.WriteStringValue(Convert.ToString(value, CultureInfo.InvariantCulture));
                return;
            case IDictionary dict:
                w.WriteStartObject();
                foreach (DictionaryEntry e in dict)
                {
                    w.WritePropertyName(Convert.ToString(e.Key, CultureInfo.InvariantCulture) ?? "");
                    WriteValue(w, e.Value, depth + 1);
                }
                w.WriteEndObject();
                return;
            case IEnumerable seq:
                w.WriteStartArray();
                foreach (var item in seq) WriteValue(w, item, depth + 1);
                w.WriteEndArray();
                return;
            default:
                // Records, anonymous objects and other POCOs, as the routes answer with them.
                JsonSerializer.Serialize(w, value, value.GetType());
                return;
        }
    }

    // --- Reading ---

    public static object? ToPlain(JsonElement el) => el.ValueKind switch
    {
        JsonValueKind.String => el.GetString(),
        // (object) on one side: a long and a double make the conditional a double.
        JsonValueKind.Number => el.TryGetInt64(out var l) ? (object)l : el.GetDouble(),
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        JsonValueKind.Object => ToDictionary(el),
        JsonValueKind.Array => el.EnumerateArray().Select(ToPlain).ToList(),
        _ => null,
    };

    public static Dictionary<string, object> ToDictionary(JsonElement el)
    {
        var dict = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        if (el.ValueKind != JsonValueKind.Object) return dict;
        foreach (var prop in el.EnumerateObject()) dict[prop.Name] = ToPlain(prop.Value)!;
        return dict;
    }

    public static Dictionary<string, object> ParseObject(string json)
    {
        using var doc = JsonDocument.Parse(json);
        return ToDictionary(doc.RootElement);
    }

    // --- States ---

    public static ExecutionState? ParseState(string json)
    {
        if (string.IsNullOrEmpty(json)) return null;
        using var doc = JsonDocument.Parse(json);
        return ToState(doc.RootElement);
    }

    public static List<ExecutionState> ParseStates(string json)
    {
        using var doc = JsonDocument.Parse(json);
        return doc.RootElement.ValueKind == JsonValueKind.Array
            ? doc.RootElement.EnumerateArray().Select(ToState).ToList()
            : new List<ExecutionState>();
    }

    private static string? Str(JsonElement o, string key) =>
        o.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    private static double Num(JsonElement o, string key) =>
        o.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.Number ? v.GetDouble() : 0;

    public static ExecutionState ToState(JsonElement o)
    {
        var state = new ExecutionState
        {
            JobId = Str(o, "job_id") ?? "",
            WorkflowName = Str(o, "workflow_name") ?? "",
            Status = Enum.TryParse<ExecutionStatus>(Str(o, "status"), out var st) ? st : ExecutionStatus.IDLE,
            Route = Str(o, "route"),
            Method = Str(o, "method"),
            CurrentStepId = Str(o, "current_step_id"),
            CurrentIndex = (int)Num(o, "current_index"),
            TotalSteps = (int)Num(o, "total_steps"),
            StartedAt = Str(o, "started_at") ?? "",
            FinishedAt = Str(o, "finished_at"),
            ClientHost = Str(o, "client_host"),
            DurationMs = Num(o, "duration_ms"),
            Error = Str(o, "error"),
            ErrorType = Str(o, "error_type"),
        };
        if (o.TryGetProperty("error_code", out var code) && code.ValueKind == JsonValueKind.Number && code.GetInt32() != 0)
        {
            state.ErrorCode = code.GetInt32();
        }
        if (o.TryGetProperty("data", out var data)) state.Data = ToDictionary(data);
        if (o.TryGetProperty("output_data", out var output) && output.ValueKind == JsonValueKind.Object)
        {
            state.OutputData = ToDictionary(output);
        }
        if (o.TryGetProperty("route_output", out var routeOutput) && routeOutput.ValueKind == JsonValueKind.Object)
        {
            state.RouteOutput = ToDictionary(routeOutput);
        }
        if (o.TryGetProperty("traces", out var traces) && traces.ValueKind == JsonValueKind.Array)
        {
            foreach (var t in traces.EnumerateArray())
            {
                var trace = new StepExecutionTrace
                {
                    StepId = Str(t, "step_id") ?? "",
                    ActivityName = Str(t, "activity_name"),
                    Status = Str(t, "status") ?? "",
                    IsAsync = t.TryGetProperty("is_async", out var a) && a.ValueKind == JsonValueKind.True,
                    ErrorMessage = Str(t, "error_message"),
                    StartedAt = Str(t, "started_at") ?? "",
                    FinishedAt = Str(t, "finished_at") ?? "",
                    DurationMs = Num(t, "duration_ms"),
                };
                if (t.TryGetProperty("input_payload", out var input)) trace.InputPayload = ToDictionary(input);
                if (t.TryGetProperty("output_payload", out var outp)) trace.OutputPayload = ToDictionary(outp);
                state.Traces.Add(trace);
            }
        }
        return state;
    }

    /// <summary>The state in the core's shape, as <c>octopus_save_state</c> takes it.</summary>
    public static string WriteState(ExecutionState s)
    {
        var map = new Dictionary<string, object?>
        {
            ["job_id"] = s.JobId,
            ["workflow_name"] = s.WorkflowName,
            ["status"] = s.Status.ToString(),
            ["route"] = s.Route,
            ["method"] = s.Method,
            ["current_step_id"] = s.CurrentStepId,
            ["current_index"] = s.CurrentIndex,
            ["total_steps"] = s.TotalSteps,
            ["data"] = s.Data,
            ["output_data"] = s.OutputData,
            ["traces"] = s.Traces.Select(t => new Dictionary<string, object?>
            {
                ["step_id"] = t.StepId,
                ["activity_name"] = t.ActivityName,
                ["status"] = t.Status,
                ["is_async"] = t.IsAsync,
                ["input_payload"] = t.InputPayload,
                ["output_payload"] = t.OutputPayload,
                ["error_message"] = t.ErrorMessage,
                ["started_at"] = t.StartedAt,
                ["finished_at"] = t.FinishedAt,
                ["duration_ms"] = t.DurationMs,
            }).ToList(),
            ["started_at"] = s.StartedAt,
            ["finished_at"] = s.FinishedAt,
            ["client_host"] = s.ClientHost,
            ["duration_ms"] = s.DurationMs,
            ["error"] = s.Error,
            ["error_code"] = s.ErrorCode,
            ["error_type"] = s.ErrorType,
        };
        foreach (var key in map.Where(kv => kv.Value is null).Select(kv => kv.Key).ToList()) map.Remove(key);
        return Write(map);
    }
}
