using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Xunit;

namespace Octopus.LiveEngine.Tests;

/// <summary>
/// Who may use the tooling routes (EngineOptions), on whichever engine
/// OCTOPUS_LIVE_ENGINE picks, and the core's middlewares and visualizer over HTTP.
/// </summary>
public class ToolingAndMiddlewareTests
{
    private const string Blueprint = @"workflow_name: shop
version: ""1.0.0""
middlewares:
  - name: CorrelationIdMiddleware
  - name: BearerAuthAndClaimsMiddleware
    config:
      jwt_secret: test-secret
    includes: [/api/v1.0.0/shop/orders]
    exceptions:
      - exception: UserNotAuthenticated
        code: 418
        message: show your token
  - name: ResponseAuditMiddleware
    activity: Audit
    includes: [/api/v1.0.0/shop/orders]
  # Only the core has this one; on the .NET engine it is skipped.
  - name: RoleAuthorizationMiddleware
    config:
      required_roles: [admin]
    includes: [/api/v1.0.0/shop/admin]
api:
  - route: /api/v1.0.0/shop/admin
    method: GET
    steps:
      - id: order
        activity: Order
        is_output: true
  - route: /api/v1.0.0/shop/orders
    method: POST
    steps:
      - id: order
        activity: Order
        is_output: true
steps:
  - id: order
    activity: Order
    is_output: true
";

    private sealed class Server : IAsyncDisposable
    {
        public required WebApplication App { get; init; }
        public required HttpClient Client { get; init; }
        public required OctopusLiveEngine Engine { get; init; }
        public List<Dictionary<string, object>> Orders { get; } = new();
        public List<Dictionary<string, object>> Audits { get; } = new();

        public async ValueTask DisposeAsync()
        {
            Client.Dispose();
            await App.DisposeAsync();
            Engine.Dispose();
        }
    }

    private static async Task<Server> Serve(string? apiKey = null, EngineOptions? options = null, string? engine = null)
    {
        var octopus = new OctopusLiveEngine(apiKey, engine: engine, options: options);
        var path = Path.Combine(Directory.CreateTempSubdirectory("octopus-shop-").FullName, "shop.yaml");
        await File.WriteAllTextAsync(path, Blueprint);
        octopus.LoadBlueprintFile(path);

        var builder = WebApplication.CreateBuilder();
        builder.Logging.ClearProviders();
        builder.WebHost.UseUrls("http://127.0.0.1:0");
        var app = builder.Build();
        octopus.MountRoutes(app);
        await app.StartAsync();
        var address = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First();
        var server = new Server { App = app, Client = new HttpClient { BaseAddress = new Uri(address) }, Engine = octopus };
        octopus.RegisterActivity("Order", input =>
        {
            lock (server.Orders) server.Orders.Add(input);
            return new Dictionary<string, object> { ["ok"] = true };
        });
        octopus.RegisterActivity("Audit", input =>
        {
            lock (server.Audits) server.Audits.Add(input);
            return new Dictionary<string, object>();
        });
        return server;
    }

    private static async Task<HttpStatusCode> Get(HttpClient client, string path, string? forwardedFor = null, string? key = null)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, path);
        if (forwardedFor != null) req.Headers.Add("X-Forwarded-For", forwardedFor);
        if (key != null) req.Headers.Add("X-API-Key", key);
        using var res = await client.SendAsync(req);
        return res.StatusCode;
    }

    private static readonly string[] ToolingRoutes =
    {
        "/visualizer", "/api/visualizer/domains", "/api/visualizer/jobs", "/api/jobs", "/docs", "/openapi.json",
        "/api/v1.0.0/shop/jobs", "/api/v1.0.0/shop/diagram/x6",
    };

    [Fact]
    public async Task ToolingIsLocalOnlyByDefault()
    {
        // The test client is local; through a trusted proxy it stands for a remote one.
        await using var server = await Serve(options: new EngineOptions { TrustedProxies = { "127.0.0.1" } });
        foreach (var route in ToolingRoutes)
        {
            Assert.Equal(HttpStatusCode.OK, await Get(server.Client, route));
            Assert.Equal(HttpStatusCode.Forbidden, await Get(server.Client, route, forwardedFor: "203.0.113.9"));
        }
        Assert.Equal(HttpStatusCode.Forbidden, await Get(server.Client, "/api/v1.0.0/shop/status/nope", forwardedFor: "203.0.113.9"));
        // The health check is not tooling.
        Assert.Equal(HttpStatusCode.OK, await Get(server.Client, "/health", forwardedFor: "203.0.113.9"));
    }

    [Fact]
    public async Task ForwardedHeadersCountOnlyFromTrustedProxies()
    {
        // No trusted proxy: the header is ignored, so a local client stays local and a
        // remote one could not claim to be local.
        await using var server = await Serve();
        Assert.Equal(HttpStatusCode.OK, await Get(server.Client, "/visualizer", forwardedFor: "203.0.113.9"));
    }

    [Fact]
    public async Task AnApiKeyOpensToolingToRemoteClients()
    {
        await using var server = await Serve("real-key", new EngineOptions { TrustedProxies = { "127.0.0.1" } });
        Assert.Equal(HttpStatusCode.Unauthorized, await Get(server.Client, "/visualizer"));
        Assert.Equal(HttpStatusCode.Unauthorized, await Get(server.Client, "/visualizer", forwardedFor: "203.0.113.9", key: "wrong"));
        Assert.Equal(HttpStatusCode.OK, await Get(server.Client, "/visualizer", forwardedFor: "203.0.113.9", key: "real-key"));
        Assert.Equal(HttpStatusCode.OK, await Get(server.Client, "/api/jobs?api_key=real-key"));
    }

    [Fact]
    public async Task ToolingCanBeOpenedDisabledOrLeftToAnAuthorizer()
    {
        await using (var open = await Serve(options: new EngineOptions { Tooling = ToolingAccess.Open, TrustedProxies = { "127.0.0.1" } }))
        {
            Assert.Equal(HttpStatusCode.OK, await Get(open.Client, "/visualizer", forwardedFor: "203.0.113.9"));
        }
        await using (var off = await Serve(options: new EngineOptions { Tooling = ToolingAccess.Disabled }))
        {
            Assert.Equal(HttpStatusCode.NotFound, await Get(off.Client, "/visualizer"));
            Assert.Equal(HttpStatusCode.NotFound, await Get(off.Client, "/api/v1.0.0/shop/jobs"));
        }
        await using (var keyed = await Serve(options: new EngineOptions { Tooling = ToolingAccess.ApiKey }))
        {
            // Required, but none configured: refused even locally.
            Assert.Equal(HttpStatusCode.Forbidden, await Get(keyed.Client, "/visualizer"));
        }
        await using var authorized = await Serve(options: new EngineOptions
        {
            ToolingAuthorizer = ctx => ctx.Request.Headers["X-Staff"] == "yes",
        });
        using var req = new HttpRequestMessage(HttpMethod.Get, "/visualizer");
        req.Headers.Add("X-Staff", "yes");
        using var ok = await authorized.Client.SendAsync(req);
        Assert.Equal(HttpStatusCode.OK, ok.StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, await Get(authorized.Client, "/visualizer"));
    }

    [Fact]
    public async Task TheVisualizerOnlyNamesRegisteredWorkflows()
    {
        await using var server = await Serve();
        var html = await server.Client.GetStringAsync("/visualizer?domain=%3C%2Fscript%3E%3Cscript%3Ealert(1)");
        Assert.DoesNotContain("alert(1)", html);
        Assert.Contains("shop", html);
    }

    private static string Jwt(Dictionary<string, object> claims, string secret)
    {
        static string B64(byte[] b) => Convert.ToBase64String(b).TrimEnd('=').Replace('+', '-').Replace('/', '_');
        var head = B64(JsonSerializer.SerializeToUtf8Bytes(new { alg = "HS256", typ = "JWT" }));
        var body = B64(JsonSerializer.SerializeToUtf8Bytes(claims));
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
        return $"{head}.{body}.{B64(hmac.ComputeHash(Encoding.UTF8.GetBytes($"{head}.{body}")))}";
    }

    private static async Task<HttpResponseMessage> Order(HttpClient client, string? token)
    {
        using var req = new HttpRequestMessage(HttpMethod.Post, "/api/v1.0.0/shop/orders")
        {
            Content = new StringContent("{\"sku\": \"A1\"}", Encoding.UTF8, "application/json"),
        };
        if (token != null) req.Headers.Add("Authorization", "Bearer " + token);
        return await client.SendAsync(req);
    }

    [Fact]
    public async Task TheCoresMiddlewaresGuardAndEnrichDeclaredRoutes()
    {
        await using var server = await Serve(engine: "core");

        // Refused by the core's bearer check, with the blueprint's exceptions: override.
        using (var refused = await Order(server.Client, null))
        {
            Assert.Equal((HttpStatusCode)418, refused.StatusCode);
            Assert.Contains("show your token", await refused.Content.ReadAsStringAsync());
            Assert.True(refused.Headers.Contains("X-Correlation-ID"));
        }
        using (var forged = await Order(server.Client, Jwt(new() { ["sub"] = "eve" }, "other-secret")))
        {
            Assert.Equal((HttpStatusCode)418, forged.StatusCode);
        }
        Assert.Empty(server.Orders);

        var token = Jwt(new() { ["sub"] = "ada", ["user_id"] = "u-1", ["customer_tier"] = "gold" }, "test-secret");
        using var ok = await Order(server.Client, token);
        Assert.Equal(HttpStatusCode.OK, ok.StatusCode);
        // The claims reach the workflow, and the body is still there for the route.
        var order = Assert.Single(server.Orders);
        Assert.Equal("u-1", order["user_id"]);
        Assert.Equal("A1", order["sku"]);
        // The core's audit ran after the answer.
        var audit = Assert.Single(server.Audits);
        Assert.Equal("u-1", audit["user_id"]);
        Assert.Equal(200L, audit["status_code"]);
        Assert.Equal(ok.Headers.GetValues("X-Job-ID").First(), audit["job_id"]);
    }

    [Fact]
    public async Task OnTheCoreTheBuiltInsAreTheCores()
    {
        await using var server = await Serve(engine: "core");
        Assert.Equal(HttpStatusCode.Forbidden, await Get(server.Client, "/api/v1.0.0/shop/admin"));
        using var req = new HttpRequestMessage(HttpMethod.Get, "/api/v1.0.0/shop/admin");
        req.Headers.Add("X-User-Roles", "admin");
        using var ok = await server.Client.SendAsync(req);
        Assert.Equal(HttpStatusCode.OK, ok.StatusCode);
    }

    [Fact]
    public async Task MiddlewaresRegisteredInCodeStillRunInDotnet()
    {
        var octopus = new OctopusLiveEngine(engine: "core");
        var seen = 0;
        octopus.RegisterCustomMiddleware("BearerAuthAndClaimsMiddleware", (cfg, exec, disp) => new CountingMiddleware(() => seen++));
        var path = Path.Combine(Directory.CreateTempSubdirectory("octopus-shop-").FullName, "shop.yaml");
        await File.WriteAllTextAsync(path, Blueprint);
        octopus.LoadBlueprintFile(path);
        var bp = await octopus.BlueprintRepo.GetBlueprintAsync("shop");
        var pipeline = octopus.BuildMiddlewarePipeline(bp!);
        var ctx = new Microsoft.AspNetCore.Http.DefaultHttpContext();
        ctx.Request.Path = "/api/v1.0.0/shop/orders";
        Assert.True(await pipeline.ExecutePipelineAsync(ctx));
        Assert.Equal(1, seen);
        octopus.Dispose();
    }

    private sealed class CountingMiddleware : Octopus.LiveEngine.Domain.Services.IOctopusMiddleware
    {
        private readonly Action _seen;
        public CountingMiddleware(Action seen) => _seen = seen;
        public Task<bool> InvokeAsync(Microsoft.AspNetCore.Http.HttpContext context)
        {
            _seen();
            return Task.FromResult(true);
        }
    }

    [Fact]
    public async Task TheCoreServesTheVisualizerAndItsDiagrams()
    {
        await using var server = await Serve(engine: "core");
        var html = await server.Client.GetStringAsync("/visualizer");
        Assert.Contains("C# Activity", html);

        var diagram = await server.Client.GetStringAsync("/api/v1.0.0/shop/diagram/x6?route=/api/v1.0.0/shop/orders&method=POST");
        using var doc = JsonDocument.Parse(diagram);
        Assert.True(doc.RootElement.GetProperty("nodes").GetArrayLength() > 0);
    }
}
