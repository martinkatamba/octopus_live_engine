using Octopus.LiveEngine.Adapters.Outbound;
using Octopus.LiveEngine.Core;
using Octopus.LiveEngine.Domain.Models;
using Xunit;

namespace Octopus.LiveEngine.Tests;

/// <summary>
/// The engine on the core (octopus-ffi). Needs the library: a build in
/// core/target, or OCTOPUS_CORE_LIB. The other tests run on whichever engine
/// OCTOPUS_LIVE_ENGINE picks.
/// </summary>
public class CoreEngineTests
{
    private static WorkflowBlueprint Greet() => new()
    {
        WorkflowName = "greet",
        Steps = new List<WorkflowStep>
        {
            new() { Id = "hello", Activity = "Hello" },
            new() { Id = "shout", Activity = "Shout" },
        },
    };

    private static OctopusLiveEngine NewCoreEngine()
    {
        var engine = new OctopusLiveEngine(engine: "core");
        engine.RegisterActivity("Hello", input =>
            new Dictionary<string, object> { ["greeting"] = $"hi {input["name"]}" });
        engine.RegisterActivity("Shout", async (input, ct) =>
        {
            await Task.Yield();
            if ((string)input["name"] == "nobody") throw new ArgumentException("no one to shout at");
            return new Dictionary<string, object> { ["shout"] = ((string)input["greeting"]).ToUpperInvariant() };
        });
        return engine;
    }

    [Fact]
    public void TheEngineChoiceIsHonoured()
    {
        using var core = new OctopusLiveEngine(engine: "core");
        Assert.Equal("core", core.EngineName);
        Assert.IsType<CoreStateStore>(core.StateStore);
        using var dotnet = new OctopusLiveEngine(engine: "dotnet");
        Assert.Equal("dotnet", dotnet.EngineName);
        Assert.Throws<ArgumentException>(() => new OctopusLiveEngine(engine: "java"));
    }

    [Fact]
    public async Task RunsSyncAndAsyncActivities()
    {
        using var engine = NewCoreEngine();
        await engine.RegisterBlueprintAsync(Greet());
        var state = await engine.ExecuteSyncAsync("greet", new Dictionary<string, object> { ["name"] = "Ada", ["job_id"] = "job-ada" });

        Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
        Assert.Equal("job-ada", state.JobId);
        Assert.Equal("HI ADA", state.Data["shout"]);
        Assert.Equal(new[] { "hello", "shout" }, state.Traces.Select(t => t.StepId));

        var stored = await engine.StateStore.GetStateAsync("job-ada");
        Assert.NotNull(stored);
        Assert.Equal("HI ADA", stored!.Data["shout"]);
    }

    [Fact]
    public async Task AFailedRunThrowsTheActivitysException()
    {
        using var engine = NewCoreEngine();
        await engine.RegisterBlueprintAsync(Greet());
        var ex = await Assert.ThrowsAsync<ArgumentException>(() =>
            engine.ExecuteSyncAsync("greet", new Dictionary<string, object> { ["name"] = "nobody", ["job_id"] = "job-nobody" }));
        Assert.Equal("no one to shout at", ex.Message);

        var stored = await engine.StateStore.GetStateAsync("job-nobody");
        Assert.Equal(ExecutionStatus.FAILED, stored!.Status);
        Assert.Equal("ArgumentException", stored.ErrorType);
    }

    [Fact]
    public async Task ValuesRoundTripThroughActivities()
    {
        using var engine = new OctopusLiveEngine(engine: "core");
        Dictionary<string, object>? seen = null;
        engine.RegisterActivity("Echo", input =>
        {
            seen = input;
            return new Dictionary<string, object>
            {
                ["emoji"] = "😀 日本 é",
                ["count"] = 3,
                ["ratio"] = 0.5,
                ["when"] = new DateTime(2026, 11, 21, 0, 0, 0, DateTimeKind.Utc),
                ["nested"] = new Dictionary<string, object> { ["list"] = new List<object> { 1, "two", true } },
            };
        });
        await engine.RegisterBlueprintAsync(new WorkflowBlueprint
        {
            WorkflowName = "echo",
            Steps = new List<WorkflowStep> { new() { Id = "echo", Activity = "Echo" } },
        });

        var state = await engine.ExecuteSyncAsync("echo", new Dictionary<string, object> { ["name"] = "Zoë 👍🏽", ["amount"] = 12 });

        Assert.Equal("Zoë 👍🏽", seen!["name"]);
        // Integers stay integers, as in the routes' request bodies.
        Assert.IsType<long>(seen["amount"]);
        Assert.Equal(12L, seen["amount"]);
        Assert.Equal("😀 日本 é", state.Data["emoji"]);
        Assert.Equal(3L, state.Data["count"]);
        Assert.Equal(0.5, state.Data["ratio"]);
        Assert.Equal("2026-11-21T00:00:00.0000000Z", state.Data["when"]);
        var nested = Assert.IsType<Dictionary<string, object>>(state.Data["nested"]);
        Assert.Equal(new List<object> { 1L, "two", true }, nested["list"]);
        // Keys read back case-insensitively, as the routes' request bodies do.
        Assert.Equal("😀 日本 é", state.Data["EMOJI"]);
    }

    [Fact]
    public async Task StreamsTheJobsLiveEvents()
    {
        using var engine = NewCoreEngine();
        await engine.RegisterBlueprintAsync(Greet());
        var started = await engine.InitiateAsync("greet", new Dictionary<string, object> { ["name"] = "Ev", ["job_id"] = "job-ev" });
        Assert.Equal("job-ev", started.JobId);

        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        var events = new List<string>();
        await foreach (var ev in engine.Workflows.StreamEventsAsync("job-ev", timeout.Token)) events.Add(ev.Event);
        Assert.Equal("workflow_completed", events.Last());

        var done = await engine.Workflows.GetExecutionStateAsync("job-ev");
        Assert.Equal(ExecutionStatus.COMPLETED, done!.Status);
        Assert.Equal("HI EV", done.Data["shout"]);
    }

    [Fact]
    public async Task TheStateStoreKeepsWhatTheRoutesSave()
    {
        using var engine = NewCoreEngine();
        await engine.RegisterBlueprintAsync(Greet());
        var state = await engine.ExecuteSyncAsync("greet", new Dictionary<string, object> { ["name"] = "Ada", ["job_id"] = "job-saved" });
        state.Route = "/api/v1.0.0/greet/execute";
        state.Method = "POST";
        state.DurationMs = 12.5;
        await engine.StateStore.SaveStateAsync(state);

        var stored = await engine.StateStore.GetStateAsync("job-saved");
        Assert.Equal("/api/v1.0.0/greet/execute", stored!.Route);
        Assert.Equal(12.5, stored.DurationMs);
        var listed = await engine.StateStore.ListStatesAsync("greet", 10);
        Assert.Contains(listed, s => s.JobId == "job-saved");
    }

    [Fact]
    public void CustomStoresAreRefusedOnTheCore()
    {
        using var engine = new OctopusLiveEngine(engine: "core");
        var ex = Assert.Throws<NotSupportedException>(() => engine.SetStateStore(new InMemoryStateStore()));
        Assert.Contains("engine: \"dotnet\"", ex.Message);
    }
}
