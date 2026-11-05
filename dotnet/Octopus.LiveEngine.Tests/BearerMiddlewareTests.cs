using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.AspNetCore.Http;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Domain.Services;
using Xunit;

namespace Octopus.LiveEngine.Tests;

// Not run in parallel with other classes: the cases set OCTOPUS_JWT_SECRET.
[Collection("OctopusJwtSecretEnv")]
public class BearerMiddlewareTests
{
    private static string B64(byte[] b) => Convert.ToBase64String(b).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    private static string Hs256(Dictionary<string, object> claims, string secret, string alg = "HS256")
    {
        var input = B64(JsonSerializer.SerializeToUtf8Bytes(new Dictionary<string, string> { ["alg"] = alg, ["typ"] = "JWT" }))
            + "." + B64(JsonSerializer.SerializeToUtf8Bytes(claims));
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
        return input + "." + B64(hmac.ComputeHash(Encoding.UTF8.GetBytes(input)));
    }

    private static async Task<IOctopusMiddleware> BuildFromYaml(string secretBlock)
    {
        var dir = Directory.CreateTempSubdirectory("octopus-jwt-");
        var path = Path.Combine(dir.FullName, "jwtwf.yaml");
        await File.WriteAllTextAsync(path,
            "workflow_name: jwtwf\nversion: \"1.0.0\"\nmiddlewares:\n  - name: BearerAuthAndClaimsMiddleware\n" + secretBlock +
            "    excludes: [/health]\nsteps:\n  - id: me\n    activity: Me\n");
        var engine = new OctopusLiveEngine();
        engine.LoadBlueprintFile(path);
        var bp = await engine.BlueprintRepo.GetBlueprintAsync("jwtwf");
        var mw = Octopus.LiveEngine.Domain.Services.MiddlewareFactory.BuildMiddlewareFromConfig(bp!.Middlewares[0], engine.ActivityExecutor, engine.CustomMiddlewares, engine.AsyncDispatcher);
        return mw!;
    }

    private static async Task<int> Call(IOctopusMiddleware mw, string path, string? token)
    {
        var ctx = new DefaultHttpContext();
        ctx.Request.Path = path;
        ctx.Response.Body = new MemoryStream();
        if (token != null) ctx.Request.Headers["Authorization"] = "Bearer " + token;
        return await mw.InvokeAsync(ctx) ? 200 : ctx.Response.StatusCode;
    }

    [Theory]
    [InlineData("    config:\n      jwt_secret: blueprint-secret-1\n", null)]
    [InlineData("    jwt_secret: blueprint-secret-1\n", null)]
    [InlineData("", "blueprint-secret-1")]
    [InlineData("", null)]
    public async Task VerifiesHs256OrRefuses(string secretBlock, string? envSecret)
    {
        Environment.SetEnvironmentVariable("OCTOPUS_JWT_SECRET", envSecret);
        try
        {
            var mw = await BuildFromYaml(secretBlock);
            const string url = "/api/v1.0.0/jwtwf/execute";
            var later = DateTimeOffset.UtcNow.AddMinutes(10).ToUnixTimeSeconds();
            var refused = new[]
            {
                "anything-at-all",
                Hs256(new() { ["sub"] = "mallory", ["exp"] = later }, "wrong-secret"),
                Hs256(new() { ["sub"] = "alice", ["exp"] = DateTimeOffset.UtcNow.AddMinutes(-1).ToUnixTimeSeconds() }, "blueprint-secret-1"),
                Hs256(new() { ["sub"] = "alice", ["exp"] = later }, "blueprint-secret-1", "none"),
                Hs256(new() { ["sub"] = "alice" }, "octopus-live-engine-secret-key-2026"),
            };
            foreach (var tok in refused)
                Assert.Equal(401, await Call(mw, url, tok));

            var configured = secretBlock != "" || envSecret != null;
            var good = Hs256(new() { ["sub"] = "alice", ["exp"] = later }, "blueprint-secret-1");
            Assert.Equal(configured ? 200 : 401, await Call(mw, url, good));
            Assert.Equal(200, await Call(mw, "/health", null));
        }
        finally
        {
            Environment.SetEnvironmentVariable("OCTOPUS_JWT_SECRET", null);
        }
    }
}
