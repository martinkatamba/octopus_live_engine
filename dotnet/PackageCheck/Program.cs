// The README's example, on the installed package: it must run on the core (or on
// the engine EXPECT_ENGINE names) and answer over HTTP. Exits non-zero otherwise.
using System.Net.Http.Json;
using System.Text.Json;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;

var expected = Environment.GetEnvironmentVariable("EXPECT_ENGINE") ?? "core";

var blueprint = Path.Combine(Path.GetTempPath(), $"hello-{Environment.ProcessId}.yaml");
File.WriteAllText(blueprint, """
    workflow_name: hello
    version: "1.0.0"
    api:
      - route: /api/v1/hello
        method: POST
        steps:
          - id: step_greet
            activity: Greet
            is_output: true
    """);

var builder = WebApplication.CreateBuilder(args);
builder.Logging.ClearProviders();
builder.WebHost.UseUrls("http://127.0.0.1:0");
var app = builder.Build();
var engine = new OctopusLiveEngine();

engine.RegisterActivity("Greet", input =>
    new Dictionary<string, object> { ["greeting"] = $"hello {input["name"]}" });

engine.LoadBlueprintFile(blueprint);
app.MapLiveEndpoints(engine);
await app.StartAsync();

var failures = new List<string>();
try
{
    Console.WriteLine($"engine: {engine.EngineName}");
    if (engine.EngineName != expected) failures.Add($"engine is {engine.EngineName}, not {expected}");

    var url = app.Urls.First();
    using var http = new HttpClient { BaseAddress = new Uri(url) };
    var res = await http.PostAsJsonAsync("/api/v1/hello", new { name = "ada" });
    var body = await res.Content.ReadAsStringAsync();
    Console.WriteLine($"POST /api/v1/hello: {(int)res.StatusCode} {body}");
    var greeting = res.IsSuccessStatusCode
        ? JsonDocument.Parse(body).RootElement.TryGetProperty("greeting", out var g) ? g.GetString() : null
        : null;
    if (greeting != "hello ada") failures.Add($"expected greeting \"hello ada\", got {(int)res.StatusCode} {body}");

    var viz = await http.GetAsync("/visualizer?domain=hello");
    Console.WriteLine($"GET /visualizer: {(int)viz.StatusCode}");
    if (!viz.IsSuccessStatusCode) failures.Add($"visualizer answered {(int)viz.StatusCode}");
}
finally
{
    await app.StopAsync();
    engine.Dispose();
    File.Delete(blueprint);
}

foreach (var f in failures) Console.Error.WriteLine($"FAIL: {f}");
Console.WriteLine(failures.Count == 0 ? "package check passed" : "package check failed");
return failures.Count == 0 ? 0 : 1;
