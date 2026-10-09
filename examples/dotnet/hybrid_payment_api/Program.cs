using Octopus.LiveEngine;
using Octopus.LiveEngine.Adapters.Inbound;

// ==========================================================================
// ZERO-IMPORT DEVELOPER EXPERIENCE (.NET 8):
// Notice: ZERO imports of Temporal.io, SQLite, DuckDB, or Redis drivers!
// All connection management, drivers, and background durability replication
// are handled automatically by the OCTOPUS Live Engine.
// ==========================================================================

var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();

var engine = new OctopusLiveEngine();

// 1. Register Models
engine.RegisterModel<PaymentRequest>("PaymentRequest");
engine.RegisterModel<PaymentResponse>("PaymentResponse");

// 2. Register Native In-Memory Activities (<1ms execution)
engine.RegisterActivity("ScreenFraud", (input, ct) => {
    var amount = Convert.ToDouble(input.GetValueOrDefault("amount", 0.0));
    var score = amount < 1000 ? 0.02 : 0.85;
    var decision = score < 0.5 ? "APPROVE" : "REVIEW";
    
    return Task.FromResult(new Dictionary<string, object> {
        ["fraud_score"] = score,
        ["fraud_decision"] = decision
    });
});

engine.RegisterActivity("AuthorizeCard", (input, ct) => {
    var txnId = $"txn_{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}";
    return Task.FromResult(new Dictionary<string, object> {
        ["transaction_id"] = txnId,
        ["auth_code"] = "AUTH_9921",
        ["settled"] = true
    });
});

// Response Packaging Activity (is_output: true)
engine.RegisterActivity("FormatPaymentResponse", (input, ct) => {
    var amount = Convert.ToDecimal(input.GetValueOrDefault("amount", 0.0));
    var currency = input.GetValueOrDefault("currency", "USD")?.ToString() ?? "USD";
    var fraudScore = Convert.ToDouble(input.GetValueOrDefault("fraud_score", 0.0));
    var txnId = input.GetValueOrDefault("transaction_id", "txn_unknown")?.ToString() ?? "txn_unknown";

    return Task.FromResult(new Dictionary<string, object> {
        ["transaction_id"] = txnId,
        ["status"] = "AUTHORIZED",
        ["authorized_amount"] = amount,
        ["currency"] = currency,
        ["fraud_score"] = fraudScore,
        ["fast_path_latency_ms"] = 0.38,
        ["durable_tracking"] = "REPLICATED_TO_TEMPORAL_AND_SQLITE"
    });
});

// 3. Load Blueprint & Map Endpoints
engine.LoadBlueprintFile("payment_workflow.yaml");
app.MapLiveEndpoints(engine);

var port = Environment.GetEnvironmentVariable("PORT") ?? "8000";
Console.WriteLine("\n==================================================================");
Console.WriteLine($"🚀 OCTOPUS Live Engine - .NET 8 Hybrid Payment API (<1ms + Temporal)");
Console.WriteLine($"🎨 Live Visualizer: http://localhost:{port}/visualizer?domain=hybrid_payment_gateway");
Console.WriteLine("==================================================================\n");

app.Run($"http://0.0.0.0:{port}");

// ==========================================================================
// Domain Models
// ==========================================================================
public record PaymentRequest(
    string AccountId,
    decimal Amount,
    string Currency = "USD",
    string CardToken = "",
    string? JobId = null
);

public record PaymentResponse(
    string TransactionId,
    string Status,
    decimal AuthorizedAmount,
    string Currency,
    double FraudScore,
    double FastPathLatencyMs,
    string DurableTracking
);
