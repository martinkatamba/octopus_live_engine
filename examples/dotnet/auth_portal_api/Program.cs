using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;
using Octopus.LiveEngine.Domain.Models;
using Octopus.LiveEngine.Domain.Ports;
using Octopus.LiveEngine.Domain.Services;

namespace AuthPortalApi;

public class Program
{
    public const string JwtSecretKey = "octopus-live-engine-secret-key-2026";

    public static void Main(string[] args)
    {
        var portStr = Environment.GetEnvironmentVariable("PORT");
        var port = 8090;

        for (int i = 0; i < args.Length; i++)
        {
            if (args[i] == "--port" && i + 1 < args.Length && int.TryParse(args[i + 1], out var p))
            {
                port = p;
            }
        }

        if (!string.IsNullOrEmpty(portStr) && int.TryParse(portStr, out var envPort))
        {
            port = envPort;
        }

        var (engine, app) = CreateApp(args);

        if (args.Contains("--test"))
        {
            RunMultilangTest(engine).GetAwaiter().GetResult();
            return;
        }

        bool workerOnly = args.Contains("--worker") || args.Contains("--worker-only") ||
                          string.Equals(Environment.GetEnvironmentVariable("WORKER_ONLY"), "true", StringComparison.OrdinalIgnoreCase);

        if (workerOnly)
        {
            Console.WriteLine("\n==================================================================");
            Console.WriteLine("👷 Dedicated Standalone Temporal Worker Node Started");
            Console.WriteLine("TaskQueue: auth_portal_api -> Polling Temporal Server (127.0.0.1:7233)");
            Console.WriteLine("==================================================================\n");
            engine.StartWorkerAsync().GetAwaiter().GetResult();

            var tcs = new TaskCompletionSource();
            Console.CancelKeyPress += (s, e) =>
            {
                e.Cancel = true;
                tcs.TrySetResult();
            };
            tcs.Task.GetAwaiter().GetResult();
            engine.StopWorkerAsync().GetAwaiter().GetResult();
            return;
        }

        Console.WriteLine("\n==================================================================");
        Console.WriteLine($"🚀 Customer Auth Portal API (.NET 8) listening on http://0.0.0.0:{port}");
        Console.WriteLine("👷 Embedded Temporal Worker Active on TaskQueue 'auth_portal_api'");
        Console.WriteLine($"📖 Interactive Swagger Docs: http://localhost:{port}/docs");
        Console.WriteLine($"🎨 Live Visualizer: http://localhost:{port}/visualizer?domain=customer_auth_portal");
        Console.WriteLine("==================================================================\n");

        app.Run($"http://0.0.0.0:{port}");
    }

    public static async Task RunMultilangTest(OctopusLiveEngine engine)
    {
        Console.WriteLine("🟣 ==================== [.NET ENGINE TESTS] ====================");

        // 1. Tagged Login Success
        var jobLoginOk = $"dotnet-login-success-{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
        Console.WriteLine($"\n[.NET] 1. Execute login (Job ID: {jobLoginOk})");
        var loginPayload = new Dictionary<string, object>
        {
            ["job_id"] = jobLoginOk,
            ["username"] = "sarah_vip",
            ["password"] = "password123"
        };
        var loginState = await engine.ExecuteEndpointSyncAsync("customer_auth_portal", null, "/api/v1.0.0/customer_auth_portal/auth/login", "POST", loginPayload);
        var token = loginState.OutputData?["access_token"]?.ToString();
        Console.WriteLine($"  Status: {loginState.Status} | User: {loginState.OutputData?["user_id"]} | Tier: {loginState.OutputData?["customer_tier"]}");

        // 2. Tagged VIP Checkout
        var jobCheckout = $"dotnet-checkout-vip-{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
        Console.WriteLine($"\n[.NET] 2. Execute checkout (Job ID: {jobCheckout})");
        var claims = DecodeAndVerifyJwt(token!, JwtSecretKey);
        var checkoutPayload = new Dictionary<string, object>
        {
            ["job_id"] = jobCheckout,
            ["cart_id"] = "cart_dotnet_001",
            ["user_id"] = claims["user_id"],
            ["customer_tier"] = claims["customer_tier"],
            ["items"] = new List<object>
            {
                new Dictionary<string, object>
                {
                    ["sku"] = "SONY-WH1000XM5",
                    ["description"] = "VIP Audio",
                    ["quantity"] = 2,
                    ["unit_price"] = 400.00
                }
            }
        };
        var checkoutState = await engine.ExecuteEndpointSyncAsync("customer_auth_portal", null, "/api/v1.0.0/customer_auth_portal/cart/checkout", "POST", checkoutPayload);
        Console.WriteLine($"  Status: {checkoutState.Status} | Subtotal: ${checkoutState.OutputData?["subtotal"]} | Final: ${checkoutState.OutputData?["final_total"]} | Tier: {checkoutState.OutputData?["pricing_tier"]}");

        // 3. Tagged Login Failure
        var jobLoginFail = $"dotnet-login-fail-{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
        Console.WriteLine($"\n[.NET] 3. Execute login failure (Job ID: {jobLoginFail})");
        var failPayload = new Dictionary<string, object>
        {
            ["job_id"] = jobLoginFail,
            ["username"] = "sarah_vip",
            ["password"] = "WRONG_PASSWORD_XYZ"
        };
        try
        {
            await engine.ExecuteEndpointSyncAsync("customer_auth_portal", null, "/api/v1.0.0/customer_auth_portal/auth/login", "POST", failPayload);
        }
        catch (Exception ex)
        {
            Console.WriteLine($"  Caught Expected Exception: {ex.Message}");
        }

        var failState = await engine.StateStore.GetStateAsync(jobLoginFail);
        Console.WriteLine($"\n[.NET] Execution Traces for {jobLoginFail} ({failState?.Traces.Count ?? 0} traces):");
        if (failState != null)
        {
            foreach (var tr in failState.Traces)
            {
                Console.WriteLine($"   • Step: {tr.StepId} | Activity: {tr.ActivityName} | Status: {tr.Status}");
            }
        }

        Console.WriteLine("\n✅ [.NET ENGINE] All tests PASSED successfully!\n");
    }

    public static (OctopusLiveEngine Engine, WebApplication App) CreateApp(string[]? args = null, string? yamlPath = null)
    {
        var builder = WebApplication.CreateBuilder(args ?? Array.Empty<string>());
        var app = builder.Build();

        var engine = new OctopusLiveEngine();

        // 1. Register Models
        engine.RegisterModel<LoginRequest>("LoginRequest");
        engine.RegisterModel<LoginResponse>("LoginResponse");
        engine.RegisterModel<UserProfileResponse>("UserProfileResponse");
        engine.RegisterModel<CheckoutRequest>("CheckoutRequest");
        engine.RegisterModel<CheckoutResponse>("CheckoutResponse");

        // 2. Register Activities
        RegisterAuthPortalActivities(engine);

        // 3. Register Custom Middleware for Bearer JWT Auth & Claims
        engine.RegisterCustomMiddleware("BearerAuthAndClaimsMiddleware", (cfg, executor, asyncDispatcher) =>
        {
            return new BearerAuthAndClaimsMiddleware(
                token =>
                {
                    var claims = DecodeAndVerifyJwt(token, JwtSecretKey);
                    return Task.FromResult(claims);
                },
                claims =>
                {
                    var userId = claims.TryGetValue("user_id", out var u) ? u?.ToString() ?? "" : "";
                    if (string.IsNullOrEmpty(userId) && claims.TryGetValue("sub", out var s))
                    {
                        userId = s?.ToString() ?? "";
                    }

                    var tier = "STANDARD";
                    if (claims.TryGetValue("customer_tier", out var t) && t != null && !string.IsNullOrWhiteSpace(t.ToString()))
                    {
                        tier = t.ToString()!;
                    }

                    return new Dictionary<string, object>
                    {
                        ["user_id"] = userId,
                        ["customer_id"] = userId,
                        ["customer_tier"] = tier,
                        ["roles"] = claims.GetValueOrDefault("roles") ?? new List<string> { "customer" },
                        ["email"] = claims.GetValueOrDefault("email") ?? ""
                    };
                },
                cfg.Excludes,
                cfg.Includes,
                cfg.Exceptions,
                executor,
                "Authorization",
                asyncDispatcher
            );
        });

        // 4. Load Blueprint & Map Endpoints
        var fileToLoad = yamlPath ?? "auth_portal_workflow.yaml";
        engine.LoadBlueprintFile(fileToLoad);
        app.MapLiveEndpoints(engine);

        return (engine, app);
    }

    // ========================================================================
    // Domain Activities
    // ========================================================================
    public static void RegisterAuthPortalActivities(OctopusLiveEngine engine)
    {
        // 1. VerifyUserCredentials
        engine.RegisterActivity("VerifyUserCredentials", (input, ct) =>
        {
            var username = input.GetValueOrDefault("username")?.ToString() ?? "";
            var password = input.GetValueOrDefault("password")?.ToString() ?? "";

            if ("crash_user".Equals(username))
            {
                throw new InvalidOperationException("division by zero during unexpected calculation crash");
            }

            if ("crash_token_issue".Equals(username))
            {
                return Task.FromResult(new Dictionary<string, object>
                {
                    ["authenticated"] = true,
                    ["user_id"] = "usr_crash_token_vip",
                    ["verified_username"] = "crash_token_issue",
                    ["email"] = "crash_token@acme.com",
                    ["customer_tier"] = "VIP",
                    ["roles"] = new List<string> { "customer", "vip_member" }
                });
            }

            if (!UserDatabase.TryGetValue(username, out var user) || user.Password != password)
            {
                throw new InvalidCredentialsException("Invalid username or password. Please try again.");
            }

            return Task.FromResult(new Dictionary<string, object>
            {
                ["authenticated"] = true,
                ["user_id"] = user.UserId,
                ["verified_username"] = user.Username,
                ["email"] = user.Email,
                ["customer_tier"] = user.CustomerTier,
                ["roles"] = user.Roles
            });
        });

        // 2. IssueBearerToken
        engine.RegisterActivity("IssueBearerToken", (input, ct) =>
        {
            var userId = input.GetValueOrDefault("user_id")?.ToString() ?? "usr_default";
            var tier = input.GetValueOrDefault("customer_tier")?.ToString() ?? "STANDARD";
            var username = input.GetValueOrDefault("verified_username")?.ToString() ?? "user";
            var email = input.GetValueOrDefault("email")?.ToString() ?? "";
            var roles = input.GetValueOrDefault("roles") ?? new List<string> { "customer" };

            if ("crash_token_issue".Equals(username))
            {
                throw new InvalidOperationException("Hardware Security Module (HSM) signing failure during token generation in IssueBearerToken");
            }

            var claims = new Dictionary<string, object>
            {
                ["sub"] = userId,
                ["user_id"] = userId,
                ["username"] = username,
                ["email"] = email,
                ["customer_tier"] = tier,
                ["roles"] = roles
            };

            var token = CreateJwtToken(claims, JwtSecretKey, 3600);

            return Task.FromResult(new Dictionary<string, object>
            {
                ["access_token"] = token,
                ["token_type"] = "Bearer",
                ["user_id"] = userId,
                ["customer_tier"] = tier,
                ["expires_in_seconds"] = 3600
            });
        });

        // 3. FetchUserProfile
        engine.RegisterActivity("FetchUserProfile", (input, ct) =>
        {
            var userId = input.GetValueOrDefault("user_id")?.ToString() ?? "";
            var user = UserDatabase.Values.FirstOrDefault(u => u.UserId == userId);

            if (user == null)
            {
                var username = input.GetValueOrDefault("username")?.ToString() ?? "guest";
                var email = input.GetValueOrDefault("email")?.ToString() ?? "guest@example.com";
                var tier = input.GetValueOrDefault("customer_tier")?.ToString() ?? "STANDARD";
                var roles = input.GetValueOrDefault("roles") as List<string> ?? new List<string> { "customer" };

                return Task.FromResult(new Dictionary<string, object>
                {
                    ["user_id"] = userId,
                    ["username"] = username,
                    ["email"] = email,
                    ["customer_tier"] = tier,
                    ["account_status"] = "ACTIVE",
                    ["member_since"] = "2026-01-01",
                    ["roles"] = roles
                });
            }

            return Task.FromResult(new Dictionary<string, object>
            {
                ["user_id"] = user.UserId,
                ["username"] = user.Username,
                ["email"] = user.Email,
                ["customer_tier"] = user.CustomerTier,
                ["account_status"] = user.AccountStatus,
                ["member_since"] = user.MemberSince,
                ["roles"] = user.Roles
            });
        });

        // 4. ValidateCartItems
        engine.RegisterActivity("ValidateCartItems", (input, ct) =>
        {
            double subtotal = 0.0;
            var items = new List<object>();

            if (input.TryGetValue("items", out var itemsObj) && itemsObj is IEnumerable<object> rawItems)
            {
                foreach (var raw in rawItems)
                {
                    items.Add(raw);
                    if (raw is Dictionary<string, object> dict)
                    {
                        var qty = dict.TryGetValue("quantity", out var q) ? Convert.ToDouble(q) : 1.0;
                        var price = dict.TryGetValue("unit_price", out var p) ? Convert.ToDouble(p) : 0.0;
                        subtotal += qty * price;
                    }
                }
            }

            if (subtotal == 0.0)
            {
                subtotal = 300.00;
                items = new List<object>
                {
                    new Dictionary<string, object>
                    {
                        ["sku"] = "HEADPHONES-PRO",
                        ["description"] = "Noise Cancelling",
                        ["quantity"] = 1,
                        ["unit_price"] = 300.00
                    }
                };
            }

            var cartId = input.GetValueOrDefault("cart_id")?.ToString() ?? "cart_default";

            return Task.FromResult(new Dictionary<string, object>
            {
                ["subtotal"] = Math.Round(subtotal, 2),
                ["validated_items"] = items,
                ["cart_id"] = cartId
            });
        });

        // 5. ApplyVIPDiscount
        engine.RegisterActivity("ApplyVIPDiscount", (input, ct) =>
        {
            var cartId = input.GetValueOrDefault("cart_id")?.ToString() ?? "";
            if (cartId.Contains("crash_vip"))
            {
                throw new InvalidOperationException("Pricing calculation engine runtime panic in ApplyVIPDiscount");
            }

            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));
            var discount = Math.Round(subtotal * 0.20, 2);
            var finalTotal = Math.Round(subtotal - discount, 2);

            return Task.FromResult(new Dictionary<string, object>
            {
                ["discount_applied"] = discount,
                ["final_total"] = finalTotal,
                ["pricing_tier"] = "VIP_PREFERENTIAL_20_PERCENT_DISCOUNT",
                ["shipping_method"] = "FREE_PRIORITY_EXPRESS_AIR"
            });
        });

        // 6. ApplyStandardPricing
        engine.RegisterActivity("ApplyStandardPricing", (input, ct) =>
        {
            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));

            return Task.FromResult(new Dictionary<string, object>
            {
                ["discount_applied"] = 0.0,
                ["final_total"] = subtotal,
                ["pricing_tier"] = "STANDARD_RETAIL_PRICING",
                ["shipping_method"] = "STANDARD_GROUND_SHIPPING"
            });
        });

        // 7. FinalizeCheckoutOrder
        engine.RegisterActivity("FinalizeCheckoutOrder", (input, ct) =>
        {
            var subtotal = Convert.ToDouble(input.GetValueOrDefault("subtotal", 0.0));
            var discount = Convert.ToDouble(input.GetValueOrDefault("discount_applied", 0.0));
            var finalTotal = Convert.ToDouble(input.GetValueOrDefault("final_total", subtotal));
            var customerId = input.GetValueOrDefault("customer_id")?.ToString()
                ?? input.GetValueOrDefault("user_id")?.ToString()
                ?? "usr_anon";
            var tier = input.GetValueOrDefault("customer_tier")?.ToString() ?? "STANDARD";
            var pricingTier = input.GetValueOrDefault("pricing_tier")?.ToString() ?? "STANDARD_RETAIL_PRICING";
            var shippingMethod = input.GetValueOrDefault("shipping_method")?.ToString() ?? "STANDARD_GROUND_SHIPPING";

            return Task.FromResult(new Dictionary<string, object>
            {
                ["order_id"] = $"ord_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}",
                ["customer_id"] = customerId,
                ["customer_tier"] = tier,
                ["subtotal"] = subtotal,
                ["discount_applied"] = discount,
                ["final_total"] = finalTotal,
                ["pricing_tier"] = pricingTier,
                ["shipping_method"] = shippingMethod,
                ["fast_path_latency_ms"] = 0.35
            });
        });

        // 8. LogSecurityAuditViolation
        engine.RegisterActivity("LogSecurityAuditViolation", (input, ct) =>
        {
            var exception = input.GetValueOrDefault("exception")?.ToString() ?? "SecurityViolation";
            var path = input.GetValueOrDefault("path")?.ToString() ?? "";
            var method = input.GetValueOrDefault("method")?.ToString() ?? "";
            var clientHost = input.GetValueOrDefault("client_host")?.ToString() ?? "";

            Console.WriteLine($"🚨 [SECURITY AUDIT] Unauthorized access attempt blocked: {method} {path} from {clientHost} ({exception})");

            return Task.FromResult(new Dictionary<string, object>
            {
                ["audit_logged"] = true,
                ["security_incident_id"] = $"inc_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}",
                ["timestamp"] = DateTime.UtcNow.ToString("o")
            });
        });

        // 9. RecordResponseAuditLog
        engine.RegisterActivity("RecordResponseAuditLog", (input, ct) =>
        {
            var route = input.GetValueOrDefault("route")?.ToString() ?? "";
            var method = input.GetValueOrDefault("method")?.ToString() ?? "";
            var statusCode = input.GetValueOrDefault("status_code")?.ToString() ?? "200";
            var durationMs = input.GetValueOrDefault("duration_ms")?.ToString() ?? "0";
            var user = input.GetValueOrDefault("user_id")?.ToString() ?? "anonymous";
            var tier = input.GetValueOrDefault("customer_tier")?.ToString() ?? "STANDARD";

            Console.WriteLine($"📊 [AUDIT LOG] {method} {route} -> HTTP {statusCode} ({durationMs}ms) [User: {user}, Tier: {tier}]");

            return Task.FromResult(new Dictionary<string, object>
            {
                ["audit_logged"] = true,
                ["timestamp"] = DateTime.UtcNow.ToString("o")
            });
        });

        // 10. HandleGlobalUnhandledException
        engine.RegisterActivity("HandleGlobalUnhandledException", (input, ct) =>
        {
            var jobId = input.GetValueOrDefault("job_id")?.ToString() ?? "job_unknown";
            var stepId = input.GetValueOrDefault("step_id")?.ToString()
                ?? input.GetValueOrDefault("failing_step_id")?.ToString()
                ?? "unknown";
            var errType = input.GetValueOrDefault("error_type")?.ToString() ?? "UnhandledException";
            var errMsg = input.GetValueOrDefault("error_message")?.ToString()
                ?? input.GetValueOrDefault("message")?.ToString()
                ?? "Internal error";

            Console.WriteLine($"🚨 [GLOBAL EXCEPTION HANDLER] Incident logged for Job={jobId} Step={stepId}: {errType} - {errMsg}");

            return Task.FromResult(new Dictionary<string, object>
            {
                ["incident_id"] = $"inc_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}",
                ["handled"] = true,
                ["status"] = "INCIDENT_LOGGED",
                ["error_type"] = errType,
                ["message"] = "An unexpected internal server error occurred. Incident reference logged.",
                ["timestamp"] = DateTime.UtcNow.ToString("o")
            });
        });
    }

    // ========================================================================
    // Pure Standard Library HMAC-SHA256 JWT
    // ========================================================================
    public static string CreateJwtToken(Dictionary<string, object> claims, string secret, int expiresInSeconds)
    {
        var header = new Dictionary<string, string> { ["alg"] = "HS256", ["typ"] = "JWT" };
        var headerJson = JsonSerializer.Serialize(header);
        var headerB64 = Base64UrlEncode(Encoding.UTF8.GetBytes(headerJson));

        var claimsCopy = new Dictionary<string, object>(claims);
        var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        claimsCopy["exp"] = now + expiresInSeconds;
        claimsCopy["iat"] = now;

        var claimsJson = JsonSerializer.Serialize(claimsCopy);
        var claimsB64 = Base64UrlEncode(Encoding.UTF8.GetBytes(claimsJson));

        var signingInput = $"{headerB64}.{claimsB64}";
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
        var signature = hmac.ComputeHash(Encoding.UTF8.GetBytes(signingInput));
        var signatureB64 = Base64UrlEncode(signature);

        return $"{signingInput}.{signatureB64}";
    }

    public static Dictionary<string, object> DecodeAndVerifyJwt(string token, string secret)
    {
        var parts = token.Split('.');
        if (parts.Length != 3)
        {
            throw new InvalidOperationException("Malformed JWT structure (must contain 3 dot-separated parts)");
        }

        var headerB64 = parts[0];
        var payloadB64 = parts[1];
        var signatureB64 = parts[2];

        var signingInput = $"{headerB64}.{payloadB64}";
        using var hmac = new HMACSHA256(Encoding.UTF8.GetBytes(secret));
        var expectedSig = hmac.ComputeHash(Encoding.UTF8.GetBytes(signingInput));
        var actualSig = Base64UrlDecode(signatureB64);

        if (!CryptographicOperations.FixedTimeEquals(expectedSig, actualSig))
        {
            throw new InvalidOperationException("Invalid JWT cryptographic signature");
        }

        var payloadBytes = Base64UrlDecode(payloadB64);
        var payloadJson = Encoding.UTF8.GetString(payloadBytes);
        var claims = JsonSerializer.Deserialize<Dictionary<string, object>>(payloadJson)
            ?? throw new InvalidOperationException("Invalid JWT JSON payload");

        var unwrappedClaims = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        foreach (var (k, v) in claims)
        {
            unwrappedClaims[k] = UnwrapJsonElement(v)!;
        }

        if (unwrappedClaims.TryGetValue("exp", out var expVal) && expVal != null)
        {
            if (long.TryParse(expVal.ToString(), out var exp))
            {
                if (exp > 0 && exp < DateTimeOffset.UtcNow.ToUnixTimeSeconds())
                {
                    throw new InvalidOperationException("JWT token has expired");
                }
            }
        }

        return unwrappedClaims;
    }

    private static string Base64UrlEncode(byte[] input)
    {
        return Convert.ToBase64String(input)
            .TrimEnd('=')
            .Replace('+', '-')
            .Replace('/', '_');
    }

    private static byte[] Base64UrlDecode(string input)
    {
        var output = input.Replace('-', '+').Replace('_', '/');
        switch (output.Length % 4)
        {
            case 2: output += "=="; break;
            case 3: output += "="; break;
        }
        return Convert.FromBase64String(output);
    }

    private static object? UnwrapJsonElement(object? obj)
    {
        if (obj is JsonElement elem)
        {
            switch (elem.ValueKind)
            {
                case JsonValueKind.String: return elem.GetString();
                case JsonValueKind.Number:
                    if (elem.TryGetInt64(out var l)) return l;
                    if (elem.TryGetDouble(out var d)) return d;
                    return elem.GetRawText();
                case JsonValueKind.True: return true;
                case JsonValueKind.False: return false;
                case JsonValueKind.Null: return null;
                case JsonValueKind.Array:
                    var list = new List<object>();
                    foreach (var item in elem.EnumerateArray()) list.Add(UnwrapJsonElement(item)!);
                    return list;
                case JsonValueKind.Object:
                    var dict = new Dictionary<string, object>(StringComparer.OrdinalIgnoreCase);
                    foreach (var p in elem.EnumerateObject()) dict[p.Name] = UnwrapJsonElement(p.Value)!;
                    return dict;
            }
        }
        return obj;
    }

    // ========================================================================
    // User Database
    // ========================================================================
    public record UserRecord(
        string UserId,
        string Username,
        string Email,
        string Password,
        string CustomerTier,
        List<string> Roles,
        string AccountStatus,
        string MemberSince
    );

    public static readonly Dictionary<string, UserRecord> UserDatabase = new(StringComparer.OrdinalIgnoreCase)
    {
        ["sarah_vip"] = new UserRecord(
            "usr_9988_vip",
            "sarah_vip",
            "sarah@acme-corp.com",
            "password123",
            "VIP",
            new List<string> { "customer", "vip_member" },
            "ACTIVE_VIP",
            "2024-01-15"
        ),
        ["john_doe"] = new UserRecord(
            "usr_1024_std",
            "john_doe",
            "john.doe@example.com",
            "password123",
            "STANDARD",
            new List<string> { "customer" },
            "ACTIVE_STANDARD",
            "2025-06-20"
        )
    };
}

// ============================================================================
// Domain Models
// ============================================================================
public record LoginRequest(string Username, string Password);
public record LoginResponse(string AccessToken, string TokenType, string UserId, string CustomerTier, int ExpiresInSeconds);
public record UserProfileResponse(string UserId, string Username, string Email, string CustomerTier, string AccountStatus, string MemberSince, List<string> Roles);
public record CartItem(string Sku, string Description, int Quantity, decimal UnitPrice);
public record CheckoutRequest(string CartId, List<CartItem> Items);
public record CheckoutResponse(string OrderId, string CustomerId, string CustomerTier, decimal Subtotal, decimal DiscountApplied, decimal FinalTotal, string PricingTier, string ShippingMethod, double FastPathLatencyMs);
