using Microsoft.AspNetCore.Http;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Domain.Services;
using Xunit;

namespace Octopus.LiveEngine.Tests;

public class ApiKeyMiddlewareTests
{
    private static readonly string[] OldBuiltInKeys = { "default-api-key", "secret-key", "katz-secret-key-12345" };

    // Builds the ApiKeyAuthMiddleware a blueprint declares, with keysBlock as its settings.
    private static async Task<IOctopusMiddleware> BuildFromYaml(string keysBlock)
    {
        var dir = Directory.CreateTempSubdirectory("octopus-apikey-");
        var path = Path.Combine(dir.FullName, "keyed.yaml");
        await File.WriteAllTextAsync(path,
            "workflow_name: keyed\nversion: \"1.0.0\"\nmiddlewares:\n  - name: ApiKeyAuthMiddleware\n" + keysBlock +
            "    excludes: [/health]\nsteps:\n  - id: ping\n    activity: Ping\n");
        var engine = new OctopusLiveEngine();
        engine.LoadBlueprintFile(path);
        var bp = await engine.BlueprintRepo.GetBlueprintAsync("keyed");
        Assert.NotNull(bp);
        var mw = Octopus.LiveEngine.Domain.Services.MiddlewareFactory.BuildMiddlewareFromConfig(bp!.Middlewares[0], engine.ActivityExecutor, engine.CustomMiddlewares, engine.AsyncDispatcher);
        Assert.NotNull(mw);
        return mw!;
    }

    private static async Task<int> Call(IOctopusMiddleware mw, string path, string? header = null, string? key = null)
    {
        var ctx = new DefaultHttpContext();
        ctx.Request.Path = path;
        ctx.Response.Body = new MemoryStream();
        if (header != null) ctx.Request.Headers[header] = key;
        return await mw.InvokeAsync(ctx) ? StatusCodes.Status200OK : ctx.Response.StatusCode;
    }

    [Theory]
    [InlineData("    config:\n      valid_keys: [partner-key-1]\n      header_name: X-Partner-Key\n", "partner-key-1")]
    [InlineData("    valid_keys: [partner-key-1]\n    header_name: X-Partner-Key\n", "partner-key-1")]
    [InlineData("", null)]
    public async Task OnlyConfiguredKeysPass(string keysBlock, string? accepted)
    {
        var mw = await BuildFromYaml(keysBlock);
        const string url = "/api/v1.0.0/keyed/execute";
        var header = accepted != null ? "X-Partner-Key" : "X-API-Key";

        foreach (var old in OldBuiltInKeys)
            Assert.Equal(401, await Call(mw, url, header, old));
        Assert.Equal(401, await Call(mw, url));
        if (accepted != null)
        {
            Assert.Equal(200, await Call(mw, url, "X-Partner-Key", accepted));
            Assert.Equal(401, await Call(mw, url, "X-API-Key", accepted));
        }
        Assert.Equal(200, await Call(mw, "/health"));
    }
}
