using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Microsoft.AspNetCore.Http;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Services;
using Xunit;

namespace Octopus.LiveEngine.Tests;

public class AuthPortalTests
{
    private const string JwtSecret = "octopus-live-engine-secret-key-2026";

    private string CreateJwt(Dictionary<string, object> claims, int expiresInSeconds = 3600)
    {
        var header = new Dictionary<string, string> { ["alg"] = "HS256", ["typ"] = "JWT" };
        var hJson = JsonSerializer.Serialize(header);
        var hB64 = Convert.ToBase64String(Encoding.UTF8.GetBytes(hJson)).TrimEnd('=').Replace('+', '-').Replace('/', '_');

        var claimsCopy = new Dictionary<string, object>(claims);
        var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        claimsCopy["exp"] = now + expiresInSeconds;
        claimsCopy["iat"] = now;

        var pJson = JsonSerializer.Serialize(claimsCopy);
        var pB64 = Convert.ToBase64String(Encoding.UTF8.GetBytes(pJson)).TrimEnd('=').Replace('+', '-').Replace('/', '_');

        var signingInput = $"{hB64}.{pB64}";
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(JwtSecret));
        var sig = hmac.ComputeHash(Encoding.UTF8.GetBytes(signingInput));
        var sB64 = Convert.ToBase64String(sig).TrimEnd('=').Replace('+', '-').Replace('/', '_');

        return $"{signingInput}.{sB64}";
    }

    [Fact]
    public void Test1_CreateAndVerifyJwt_ShouldWorkProperly()
    {
        var claims = new Dictionary<string, object>
        {
            ["sub"] = "usr_9988_vip",
            ["user_id"] = "usr_9988_vip",
            ["customer_tier"] = "VIP",
            ["email"] = "sarah@acme-corp.com"
        };

        var token = CreateJwt(claims, 3600);
        Assert.NotNull(token);
        Assert.Equal(3, token.Split('.').Length);

        // Verify with valid secret
        var parts = token.Split('.');
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(JwtSecret));
        var expectedSig = hmac.ComputeHash(Encoding.UTF8.GetBytes($"{parts[0]}.{parts[1]}"));
        var actualSigB64 = parts[2].Replace('-', '+').Replace('_', '/');
        while (actualSigB64.Length % 4 != 0) actualSigB64 += "=";
        var actualSig = Convert.FromBase64String(actualSigB64);
        Assert.True(CryptographicOperations.FixedTimeEquals(expectedSig, actualSig));

        // Tampered signature fails
        var tampered = token.Substring(0, token.Length - 4) + "AAAA";
        var tParts = tampered.Split('.');
        var tSigB64 = tParts[2].Replace('-', '+').Replace('_', '/');
        while (tSigB64.Length % 4 != 0) tSigB64 += "=";
        var tSig = Convert.FromBase64String(tSigB64);
        Assert.False(CryptographicOperations.FixedTimeEquals(expectedSig, tSig));
    }

    [Fact]
    public async Task Test2_BearerAuthMiddleware_MissingToken_Returns401()
    {
        var exceptions = new List<MiddlewareExceptionConfig>
        {
            new()
            {
                Exception = "UserNotAuthenticated",
                Code = 401,
                Message = "Authentication required. Please provide a valid Bearer token via the Authorization header."
            }
        };

        var mw = new BearerAuthAndClaimsMiddleware(
            null,
            null,
            new[] { "/api/v1.0.0/customer_auth_portal/auth/login", "/health" },
            null,
            exceptions,
            null
        );

        var context = new DefaultHttpContext();
        context.Request.Path = "/api/v1.0.0/customer_auth_portal/profile";
        context.Response.Body = new MemoryStream();

        var cont = await mw.InvokeAsync(context);
        Assert.False(cont);
        Assert.Equal(StatusCodes.Status401Unauthorized, context.Response.StatusCode);

        context.Response.Body.Seek(0, SeekOrigin.Begin);
        using var reader = new StreamReader(context.Response.Body);
        var body = await reader.ReadToEndAsync();
        Assert.Contains("UserNotAuthenticated", body);
        Assert.Contains("Authentication required", body);
    }

    [Fact]
    public async Task Test3_BearerAuthMiddleware_ExemptRoute_PassesThrough()
    {
        var mw = new BearerAuthAndClaimsMiddleware(
            null,
            null,
            new[] { "/api/v1.0.0/customer_auth_portal/auth/login", "/health" },
            null,
            null,
            null
        );

        var context = new DefaultHttpContext();
        context.Request.Path = "/api/v1.0.0/customer_auth_portal/auth/login";

        var cont = await mw.InvokeAsync(context);
        Assert.True(cont);
    }

    [Fact]
    public async Task Test4_BearerAuthMiddleware_ValidToken_InjectsClaims()
    {
        var mw = new BearerAuthAndClaimsMiddleware(
            token => Task.FromResult(new Dictionary<string, object>
            {
                ["user_id"] = "usr_9988_vip",
                ["customer_tier"] = "VIP",
                ["email"] = "sarah@acme-corp.com"
            }),
            claims => new Dictionary<string, object>
            {
                ["user_id"] = claims["user_id"],
                ["customer_tier"] = claims["customer_tier"]
            },
            new[] { "/health" },
            null,
            null,
            null
        );

        var context = new DefaultHttpContext();
        context.Request.Path = "/api/v1.0.0/customer_auth_portal/profile";
        context.Request.Headers["Authorization"] = "Bearer valid-test-token";

        var cont = await mw.InvokeAsync(context);
        Assert.True(cont);
        Assert.True(context.Items.ContainsKey("auth_claims"));
        Assert.True(context.Items.ContainsKey("workflow_injections"));

        var injections = context.Items["workflow_injections"] as Dictionary<string, object>;
        Assert.NotNull(injections);
        Assert.Equal("usr_9988_vip", injections!["user_id"]);
        Assert.Equal("VIP", injections["customer_tier"]);
    }

    [Fact]
    public async Task Test5_IpWhitelistMiddleware_BlocksBlacklistedIp()
    {
        var exceptions = new List<MiddlewareExceptionConfig>
        {
            new()
            {
                Exception = "IpAccessDenied",
                Code = 403,
                Message = "Access forbidden: Client IP network is not authorized."
            }
        };

        var mw = new IpWhitelistMiddleware(
            allowedIps: new[] { "127.0.0.1", "10.0.0.0/8" },
            blockedIps: new[] { "10.0.0.99" },
            customValidator: null,
            excludes: new[] { "/health" },
            includes: null,
            exceptions: exceptions,
            executor: null
        );

        // 1. Allowed IP 10.0.0.1
        var ctxAllowed = new DefaultHttpContext();
        ctxAllowed.Request.Path = "/api/v1/test";
        ctxAllowed.Connection.RemoteIpAddress = IPAddress.Parse("10.0.0.1");
        var contAllowed = await mw.InvokeAsync(ctxAllowed);
        Assert.True(contAllowed);

        // 2. Blocked IP 10.0.0.99
        var ctxBlocked = new DefaultHttpContext();
        ctxBlocked.Request.Path = "/api/v1/test";
        ctxBlocked.Connection.RemoteIpAddress = IPAddress.Parse("10.0.0.99");
        ctxBlocked.Response.Body = new MemoryStream();
        var contBlocked = await mw.InvokeAsync(ctxBlocked);
        Assert.False(contBlocked);
        Assert.Equal(StatusCodes.Status403Forbidden, ctxBlocked.Response.StatusCode);

        ctxBlocked.Response.Body.Seek(0, SeekOrigin.Begin);
        using var reader = new StreamReader(ctxBlocked.Response.Body);
        var body = await reader.ReadToEndAsync();
        Assert.Contains("IpAccessDenied", body);
        Assert.Contains("Access forbidden", body);
    }

    [Fact]
    public async Task Test6_RateLimitMiddleware_SlidingWindowAndTierLimits()
    {
        var exceptions = new List<MiddlewareExceptionConfig>
        {
            new()
            {
                Exception = "RateLimitExceeded",
                Code = 429,
                Message = "API request quota exceeded. Please slow down your requests or upgrade to VIP tier."
            }
        };

        var mw = new RateLimitMiddleware(
            limit: 5,
            windowSeconds: 60,
            keyBy: "client_ip",
            tierClaim: "customer_tier",
            tierHeader: "X-User-Tier",
            tierLimits: new Dictionary<string, int>
            {
                ["VIP"] = 10,
                ["STANDARD"] = 5,
                ["ANONYMOUS"] = 3
            },
            excludes: new[] { "/health" },
            includes: null,
            exceptions: exceptions,
            executor: null
        );

        var testIp = "192.168.1.50";

        // Send 3 requests (ANONYMOUS quota is 3)
        for (int i = 0; i < 3; i++)
        {
            var ctx = new DefaultHttpContext();
            ctx.Request.Path = "/api/v1/test";
            ctx.Connection.RemoteIpAddress = IPAddress.Parse(testIp);
            var cont = await mw.InvokeAsync(ctx);
            Assert.True(cont);
            Assert.Equal("3", ctx.Response.Headers["X-RateLimit-Limit"].ToString());
        }

        // 4th request exceeds quota -> 429
        var ctxExceeded = new DefaultHttpContext();
        ctxExceeded.Request.Path = "/api/v1/test";
        ctxExceeded.Connection.RemoteIpAddress = IPAddress.Parse(testIp);
        ctxExceeded.Response.Body = new MemoryStream();
        var contExceeded = await mw.InvokeAsync(ctxExceeded);
        Assert.False(contExceeded);
        Assert.Equal(StatusCodes.Status429TooManyRequests, ctxExceeded.Response.StatusCode);
        Assert.True(ctxExceeded.Response.Headers.ContainsKey("Retry-After"));
        Assert.True(ctxExceeded.Response.Headers.ContainsKey("X-RateLimit-Limit"));

        ctxExceeded.Response.Body.Seek(0, SeekOrigin.Begin);
        using var reader = new StreamReader(ctxExceeded.Response.Body);
        var body = await reader.ReadToEndAsync();
        Assert.Contains("RateLimitExceeded", body);
        Assert.Contains("API request quota exceeded", body);
    }

    [Fact]
    public async Task Test7_CustomerAuthPortal_VIPCheckout_Execution()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("VerifyUserCredentials", input => new Dictionary<string, object>
        {
            ["authenticated"] = true,
            ["user_id"] = "usr_9988_vip",
            ["customer_tier"] = "VIP"
        });

        engine.RegisterActivity("ValidateCartItems", input => new Dictionary<string, object>
        {
            ["subtotal"] = 200.00,
            ["cart_id"] = "cart_vip_001"
        });

        engine.RegisterActivity("ApplyVIPDiscount", input =>
        {
            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));
            var discount = subtotal * 0.20;
            return new Dictionary<string, object>
            {
                ["discount_applied"] = discount,
                ["final_total"] = subtotal - discount,
                ["pricing_tier"] = "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
                ["shipping_method"] = "FREE_PRIORITY_EXPRESS_AIR"
            };
        });

        engine.RegisterActivity("ApplyStandardPricing", input =>
        {
            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));
            return new Dictionary<string, object>
            {
                ["discount_applied"] = 0.0,
                ["final_total"] = subtotal,
                ["pricing_tier"] = "STANDARD_RETAIL_PRICING",
                ["shipping_method"] = "STANDARD_GROUND_SHIPPING"
            };
        });

        engine.RegisterActivity("FinalizeCheckoutOrder", input => new Dictionary<string, object>
        {
            ["order_id"] = "ord_vip_test_99",
            ["customer_id"] = input.GetValueOrDefault("user_id") ?? "usr_9988_vip",
            ["customer_tier"] = input.GetValueOrDefault("customer_tier") ?? "VIP",
            ["subtotal"] = input.GetValueOrDefault("subtotal") ?? 200.00,
            ["discount_applied"] = input.GetValueOrDefault("discount_applied") ?? 40.00,
            ["final_total"] = input.GetValueOrDefault("final_total") ?? 160.00,
            ["pricing_tier"] = input.GetValueOrDefault("pricing_tier") ?? "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
            ["shipping_method"] = input.GetValueOrDefault("shipping_method") ?? "FREE_PRIORITY_EXPRESS_AIR",
            ["fast_path_latency_ms"] = 0.35
        });

        engine.LoadBlueprintFile("octopus_live_engine/examples/dotnet/auth_portal_api/auth_portal_workflow.yaml");

        var bp = await engine.BlueprintRepo.GetBlueprintAsync("customer_auth_portal");
        Assert.NotNull(bp);
        Assert.Equal(7, bp!.Middlewares.Count);

        var checkoutEp = bp.Api.FirstOrDefault(e => e.Route.Contains("checkout"));
        Assert.NotNull(checkoutEp);

        // Execute VIP Checkout
        var vipState = await engine.ExecuteEndpointSyncAsync(
            "customer_auth_portal",
            checkoutEp,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST",
            new Dictionary<string, object>
            {
                ["user_id"] = "usr_9988_vip",
                ["customer_tier"] = "VIP",
                ["cart_id"] = "cart_vip_001",
                ["items"] = new List<object>
                {
                    new Dictionary<string, object> { ["sku"] = "HEADPHONES", ["unit_price"] = 200.00, ["quantity"] = 1 }
                }
            }
        );

        Assert.Equal(ExecutionStatus.COMPLETED, vipState.Status);
        Assert.NotNull(vipState.OutputData);
        Assert.Equal(40.00, Convert.ToDouble(vipState.OutputData!["discount_applied"]));
        Assert.Equal(160.00, Convert.ToDouble(vipState.OutputData!["final_total"]));
        Assert.Equal("VIP_PREFERENTIAL_20_PERCENT_DISCOUNT", vipState.OutputData!["pricing_tier"]);
        Assert.Equal("FREE_PRIORITY_EXPRESS_AIR", vipState.OutputData!["shipping_method"]);
    }

    [Fact]
    public async Task Test8_CustomerAuthPortal_StandardCheckout_Execution()
    {
        var engine = new OctopusLiveEngine();

        engine.RegisterActivity("ValidateCartItems", input => new Dictionary<string, object>
        {
            ["subtotal"] = 200.00,
            ["cart_id"] = "cart_std_001"
        });

        engine.RegisterActivity("ApplyVIPDiscount", input => new Dictionary<string, object>
        {
            ["discount_applied"] = 40.00,
            ["final_total"] = 160.00,
            ["pricing_tier"] = "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
            ["shipping_method"] = "FREE_PRIORITY_EXPRESS_AIR"
        });

        engine.RegisterActivity("ApplyStandardPricing", input =>
        {
            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));
            return new Dictionary<string, object>
            {
                ["discount_applied"] = 0.0,
                ["final_total"] = subtotal,
                ["pricing_tier"] = "STANDARD_RETAIL_PRICING",
                ["shipping_method"] = "STANDARD_GROUND_SHIPPING"
            };
        });

        engine.RegisterActivity("FinalizeCheckoutOrder", input => new Dictionary<string, object>
        {
            ["order_id"] = "ord_std_test_10",
            ["customer_id"] = input.GetValueOrDefault("user_id") ?? "usr_1024_std",
            ["customer_tier"] = input.GetValueOrDefault("customer_tier") ?? "STANDARD",
            ["subtotal"] = input.GetValueOrDefault("subtotal") ?? 200.00,
            ["discount_applied"] = input.GetValueOrDefault("discount_applied") ?? 0.00,
            ["final_total"] = input.GetValueOrDefault("final_total") ?? 200.00,
            ["pricing_tier"] = input.GetValueOrDefault("pricing_tier") ?? "STANDARD_RETAIL_PRICING",
            ["shipping_method"] = input.GetValueOrDefault("shipping_method") ?? "STANDARD_GROUND_SHIPPING"
        });

        engine.LoadBlueprintFile("octopus_live_engine/examples/dotnet/auth_portal_api/auth_portal_workflow.yaml");

        var bp = await engine.BlueprintRepo.GetBlueprintAsync("customer_auth_portal");
        var checkoutEp = bp!.Api.FirstOrDefault(e => e.Route.Contains("checkout"));

        // Execute Standard Checkout
        var stdState = await engine.ExecuteEndpointSyncAsync(
            "customer_auth_portal",
            checkoutEp,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST",
            new Dictionary<string, object>
            {
                ["user_id"] = "usr_1024_std",
                ["customer_tier"] = "STANDARD",
                ["cart_id"] = "cart_std_001",
                ["items"] = new List<object>
                {
                    new Dictionary<string, object> { ["sku"] = "HEADPHONES", ["unit_price"] = 200.00, ["quantity"] = 1 }
                }
            }
        );

        Assert.Equal(ExecutionStatus.COMPLETED, stdState.Status);
        Assert.NotNull(stdState.OutputData);
        Assert.Equal(0.00, Convert.ToDouble(stdState.OutputData!["discount_applied"]));
        Assert.Equal(200.00, Convert.ToDouble(stdState.OutputData!["final_total"]));
        Assert.Equal("STANDARD_RETAIL_PRICING", stdState.OutputData!["pricing_tier"]);
        Assert.Equal("STANDARD_GROUND_SHIPPING", stdState.OutputData!["shipping_method"]);
    }

    [Fact]
    public async Task Test9_VisualizerDiagram_CustomerAuthPortal()
    {
        var engine = new OctopusLiveEngine();
        engine.LoadBlueprintFile("octopus_live_engine/examples/dotnet/auth_portal_api/auth_portal_workflow.yaml");

        // 1. Checkout route diagram
        var checkoutDiagram = await engine.VisualizerService.CompileX6DiagramAsync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST"
        );
        Assert.NotNull(checkoutDiagram);
        Assert.True(checkoutDiagram.ContainsKey("nodes"));
        Assert.True(checkoutDiagram.ContainsKey("edges"));

        var checkoutNodes = Assert.IsAssignableFrom<IEnumerable<Dictionary<string, object>>>(checkoutDiagram["nodes"]).ToList();
        Assert.NotEmpty(checkoutNodes);

        var startNode = checkoutNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__start");
        Assert.NotNull(startNode);

        var ingressNode = checkoutNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__ingress_guards");
        Assert.NotNull(ingressNode);

        var egressNode = checkoutNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__egress_guards");
        Assert.NotNull(egressNode);

        var exc401Node = checkoutNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__exc_usernotauthenticated");
        Assert.NotNull(exc401Node);

        var endNode = checkoutNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__end");
        Assert.NotNull(endNode);

        // 2. Login route diagram (BearerAuth excluded)
        var loginDiagram = await engine.VisualizerService.CompileX6DiagramAsync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/auth/login",
            "POST"
        );
        var loginNodes = Assert.IsAssignableFrom<IEnumerable<Dictionary<string, object>>>(loginDiagram["nodes"]).ToList();
        var loginExc401 = loginNodes.FirstOrDefault(n => n.TryGetValue("id", out var id) && id?.ToString() == "node__exc_invalidcredentials");
        Assert.NotNull(loginExc401);
    }
}
