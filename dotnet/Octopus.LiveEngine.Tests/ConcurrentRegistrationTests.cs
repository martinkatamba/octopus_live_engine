using Octopus.LiveEngine.Domain.Models;
using Xunit;

namespace Octopus.LiveEngine.Tests;

/// <summary>
/// Engines on the core registering blueprints and running them at the same moment.
/// They share one YAML serializer, which isn't thread-safe; unguarded, it once threw
/// "Operations that change non-concurrent collections must have exclusive access"
/// (CI, 2026-11-27). This test doesn't reliably reproduce that race (it passed 9 of
/// 9 runs without the lock); the lock in <c>CorePipelineManager.ToYaml</c> is the fix.
/// </summary>
public class ConcurrentRegistrationTests
{
    [Fact]
    public async Task EnginesCanRegisterBlueprintsAtTheSameTime()
    {
        const int engines = 32;
        var start = new Barrier(engines);
        var all = Enumerable.Range(0, engines).Select(i => new OctopusLiveEngine(engine: "core")).ToList();
        try
        {
            // A thread each, so all of them can wait at the barrier together.
            var registrations = all.Select((engine, i) => Task.Factory.StartNew(async () =>
            {
                engine.RegisterActivity("Echo", input => new Dictionary<string, object>(input));
                var blueprint = new WorkflowBlueprint
                {
                    WorkflowName = $"concurrent_{i}",
                    Steps = new List<WorkflowStep>
                    {
                        new() { Id = "echo", Activity = "Echo" },
                    },
                };
                start.SignalAndWait();
                await engine.RegisterBlueprintAsync(blueprint);
                var state = await engine.ExecuteSyncAsync(blueprint.WorkflowName, new Dictionary<string, object> { ["n"] = i });
                Assert.Equal(ExecutionStatus.COMPLETED, state.Status);
            }, CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default).Unwrap());
            await Task.WhenAll(registrations);
        }
        finally
        {
            foreach (var engine in all) engine.Dispose();
        }
    }
}
