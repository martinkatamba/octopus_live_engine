using System.Net;
using System.Text;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Octopus.LiveEngine.Domain.Models;
using Xunit;

namespace Octopus.LiveEngine.Tests;

/// <summary>The engine's own API key (<c>new OctopusLiveEngine(apiKey)</c>) on the routes it mounts.</summary>
public class EngineApiKeyTests
{
    private const string Execute = "/api/v1.0.0/ping/execute";

    private static async Task<(WebApplication App, HttpClient Client)> Serve(string? apiKey)
    {
        var engine = new OctopusLiveEngine(apiKey);
        engine.RegisterActivity("Ping", _ => new Dictionary<string, object> { ["pong"] = true });
        await engine.RegisterBlueprintAsync(new WorkflowBlueprint
        {
            WorkflowName = "ping",
            Steps = new List<WorkflowStep> { new() { Id = "ping", Activity = "Ping" } },
        });

        var builder = WebApplication.CreateBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.UseUrls("http://127.0.0.1:0");
        var app = builder.Build();
        engine.MountRoutes(app);
        await app.StartAsync();
        var address = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First();
        return (app, new HttpClient { BaseAddress = new Uri(address) });
    }

    private static async Task<HttpStatusCode> Post(HttpClient client, string path, string? key)
    {
        using var req = new HttpRequestMessage(HttpMethod.Post, path)
        {
            Content = new StringContent("{}", Encoding.UTF8, "application/json"),
        };
        if (key != null) req.Headers.Add("X-API-Key", key);
        using var res = await client.SendAsync(req);
        return res.StatusCode;
    }

    [Fact]
    public async Task OnlyTheConfiguredKeyIsAccepted()
    {
        var (app, client) = await Serve("real-key");
        await using var _ = app;
        using var __ = client;

        Assert.Equal(HttpStatusCode.OK, await Post(client, Execute, "real-key"));
        Assert.Equal(HttpStatusCode.OK, await Post(client, Execute + "?api_key=real-key", null));

        Assert.Equal(HttpStatusCode.Unauthorized, await Post(client, Execute, null));
        Assert.Equal(HttpStatusCode.Unauthorized, await Post(client, Execute, "wrong-key"));
        Assert.Equal(HttpStatusCode.Unauthorized, await Post(client, Execute, "REAL-KEY"));
        Assert.Equal(HttpStatusCode.Unauthorized, await Post(client, Execute, "real-key-2"));
        foreach (var old in new[] { "default-api-key", "secret-key", "katz-secret-key-12345" })
        {
            Assert.Equal(HttpStatusCode.Unauthorized, await Post(client, Execute, old));
        }
        using var status = await client.GetAsync("/api/v1.0.0/ping/status/nope");
        Assert.Equal(HttpStatusCode.Unauthorized, status.StatusCode);
    }

    [Fact]
    public async Task WithNoKeyConfiguredTheRoutesAreOpen()
    {
        var (app, client) = await Serve(null);
        await using var _ = app;
        using var __ = client;

        Assert.Equal(HttpStatusCode.OK, await Post(client, Execute, null));
        Assert.Equal(HttpStatusCode.OK, await Post(client, Execute, "anything"));
    }
}
