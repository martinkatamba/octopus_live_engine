package io.octopus.examples.payment;

import io.octopus.live.LiveEngine;
import java.util.HashMap;
import java.util.Map;

// ==========================================================================
// ZERO-IMPORT DEVELOPER EXPERIENCE (Java 21+):
// Notice: ZERO imports of Temporal.io, SQLite, DuckDB, or Redis drivers!
// All connection management, drivers, and background durability replication
// are handled automatically by the OCTOPUS Live Engine.
// ==========================================================================

public class HybridPaymentApplication {

    // 1. Domain Models
    public static class PaymentRequest {
        public String account_id;
        public double amount;
        public String currency = "USD";
        public String card_token;
        public String job_id;
    }

    public static class PaymentResponse {
        public String transaction_id;
        public String status;
        public double authorized_amount;
        public String currency;
        public double fraud_score;
        public double fast_path_latency_ms;
        public String durable_tracking;
    }

    public static void main(String[] args) throws Exception {
        LiveEngine engine = new LiveEngine();

        // 1. Register Models for Schema and Contract Validation
        engine.registerModel("PaymentRequest", PaymentRequest.class);
        engine.registerModel("PaymentResponse", PaymentResponse.class);

        // 2. Register Native In-Memory Activities (<1ms execution)
        engine.registerActivity("ScreenFraud", input -> {
            double amount = input.get("amount") != null ? Double.parseDouble(input.get("amount").toString()) : 0.0;
            double score = amount < 1000.0 ? 0.02 : 0.85;
            String decision = score < 0.5 ? "APPROVE" : "REVIEW";

            Map<String, Object> out = new HashMap<>();
            out.put("fraud_score", score);
            out.put("fraud_decision", decision);
            return out;
        });

        engine.registerActivity("AuthorizeCard", input -> {
            String txnId = "txn_" + System.currentTimeMillis();
            Map<String, Object> out = new HashMap<>();
            out.put("transaction_id", txnId);
            out.put("auth_code", "AUTH_9921");
            out.put("settled", true);
            return out;
        });

        // Response Packaging Activity (is_output: true)
        engine.registerActivity("FormatPaymentResponse", input -> {
            double amount = input.get("amount") != null ? Double.parseDouble(input.get("amount").toString()) : 0.0;
            String currency = input.getOrDefault("currency", "USD").toString();
            double fraudScore = input.get("fraud_score") != null ? Double.parseDouble(input.get("fraud_score").toString()) : 0.0;
            String txnId = input.getOrDefault("transaction_id", "txn_unknown").toString();

            Map<String, Object> out = new HashMap<>();
            out.put("transaction_id", txnId);
            out.put("status", "AUTHORIZED");
            out.put("authorized_amount", amount);
            out.put("currency", currency);
            out.put("fraud_score", fraudScore);
            out.put("fast_path_latency_ms", 0.35);
            out.put("durable_tracking", "REPLICATED_TO_TEMPORAL_AND_SQLITE");
            return out;
        });

        // 3. Load Blueprint from YAML (Zero-Import persistence & Temporal config)
        engine.loadBlueprintsFromDirectory(".");
        int port = System.getenv("PORT") != null ? Integer.parseInt(System.getenv("PORT")) : 8000;
        engine.startServer(port);

        System.out.println("\n==================================================================");
        System.out.println("🚀 OCTOPUS Live Engine - Java 21+ Hybrid Payment API (<1ms + Temporal)");
        System.out.println("📖 Swagger Docs:    http://localhost:" + port + "/docs");
        System.out.println("🎨 Live Visualizer: http://localhost:" + port + "/visualizer?domain=hybrid_payment_gateway");
        System.out.println("==================================================================\n");
    }
}
