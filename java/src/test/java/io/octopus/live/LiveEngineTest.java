package io.octopus.live;

import io.octopus.live.domain.models.ExecutionStatus;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.StepExecutionTrace;
import io.octopus.live.domain.models.WorkflowBlueprint;
import io.octopus.live.domain.models.WorkflowStep;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

public class LiveEngineTest {

    @Test
    public void testJavaLiveEngineSyncExecution() {
        LiveEngine engine = new LiveEngine();

        engine.registerActivity("ValidateOrder", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("is_valid", true);
            out.put("order_id", input.getOrDefault("order_id", "ORD-001"));
            return out;
        });

        engine.registerActivity("CalculateTax", input -> {
            double amount = ((Number) input.getOrDefault("amount", 100.0)).doubleValue();
            double tax = amount * 0.15;
            Map<String, Object> out = new HashMap<>();
            out.put("tax_amount", tax);
            out.put("total", amount + tax);
            return out;
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("order_tax_pipeline");
        bp.setVersion("1.0.0");
        bp.setMode("sync");

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("validate");
        s1.setActivity("ValidateOrder");

        WorkflowStep s2 = new WorkflowStep();
        s2.setId("calc_tax");
        s2.setActivity("CalculateTax");

        bp.setSteps(List.of(s1, s2));
        engine.registerBlueprint(bp);

        Map<String, Object> payload = new HashMap<>();
        payload.put("order_id", "ORD-999");
        payload.put("amount", 200.0);

        ExecutionState state = engine.executeSync("order_tax_pipeline", payload);

        assertNotNull(state);
        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        assertEquals(2, state.getTraces().size());
        assertEquals(230.0, ((Number) state.getData().get("total")).doubleValue(), 0.001);

        // Test Visualizer Diagram compilation
        Map<String, Object> diagram = engine.getVisualizerUseCase().compileX6Diagram("order_tax_pipeline", state.getJobId());
        assertNotNull(diagram);
        assertTrue(diagram.containsKey("nodes"));
        List<?> nodes = (List<?>) diagram.get("nodes");
        assertEquals(4, nodes.size()); // START, validate, calc_tax, END
    }

    @Test
    public void testJavaLiveEngineLoopExecution() {
        LiveEngine engine = new LiveEngine();

        engine.registerActivity("ProcessItem", input -> {
            @SuppressWarnings("unchecked")
            Map<String, Object> item = (Map<String, Object>) input.get("curr_item");
            double price = ((Number) item.get("price")).doubleValue();
            double tax = price * 0.10;
            Map<String, Object> out = new HashMap<>();
            out.put("processed_sku", item.get("sku"));
            out.put("item_tax", tax);
            return out;
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("batch_loop_wf");
        bp.setMode("sync");

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("loop_step");
        s1.setActivity("ProcessItem");
        Map<String, Object> loopCfg = new HashMap<>();
        loopCfg.put("items_field", "items");
        loopCfg.put("item_variable", "curr_item");
        s1.setLoop(loopCfg);

        bp.setSteps(List.of(s1));
        engine.registerBlueprint(bp);

        List<Map<String, Object>> items = List.of(
            Map.of("sku", "ITEM-1", "price", 10.0),
            Map.of("sku", "ITEM-2", "price", 25.0),
            Map.of("sku", "ITEM-3", "price", 50.0)
        );

        Map<String, Object> payload = new HashMap<>();
        payload.put("items", items);

        ExecutionState state = engine.executeSync("batch_loop_wf", payload);
        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        assertTrue(state.getData().containsKey("loop_step_results"));

        @SuppressWarnings("unchecked")
        List<Map<String, Object>> results = (List<Map<String, Object>>) state.getData().get("loop_step_results");
        assertEquals(3, results.size());
    }

    @Test
    public void testJavaLiveEngineAsyncActivity() throws InterruptedException {
        LiveEngine engine = new LiveEngine();
        AtomicBoolean asyncExecuted = new AtomicBoolean(false);

        engine.registerActivity("SyncStep", input -> Map.of("sync_done", true));
        engine.registerActivity("BackgroundAudit", input -> {
            try {
                Thread.sleep(100);
            } catch (InterruptedException ignored) {}
            asyncExecuted.set(true);
            return Map.of("audited", true);
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("async_activity_wf");
        bp.setMode("sync");

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("sync_step");
        s1.setActivity("SyncStep");

        WorkflowStep s2 = new WorkflowStep();
        s2.setId("audit_step");
        s2.setActivity("BackgroundAudit");
        s2.setAsync(true);

        bp.setSteps(List.of(s1, s2));
        engine.registerBlueprint(bp);

        ExecutionState state = engine.executeSync("async_activity_wf", new HashMap<>());
        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        assertEquals("ASYNC_DISPATCHED", state.getTraces().get(1).getStatus());

        // Wait a bit for async background execution to complete
        Thread.sleep(200);
        assertTrue(asyncExecuted.get());
    }

    @Test
    public void testJavaLiveEngineSagaCompensationAndSettle() {
        LiveEngine engine = new LiveEngine();
        AtomicBoolean compensated = new AtomicBoolean(false);

        engine.registerActivity("DebitAccount", input -> Map.of("debited", true));
        engine.registerActivity("RefundAccount", input -> {
            compensated.set(true);
            return Map.of("refunded", true);
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("saga_rollback_wf");
        bp.setMode("sync");
        bp.setCompensationMode("sync");

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("debit");
        s1.setActivity("DebitAccount");
        s1.setCompensation("RefundAccount");

        WorkflowStep s2 = new WorkflowStep();
        s2.setId("compensate_trigger");
        s2.setCompensate(true);
        s2.setReason("Fraud detected in security audit");

        bp.setSteps(List.of(s1, s2));
        engine.registerBlueprint(bp);

        assertThrows(IllegalStateException.class, () -> engine.executeSync("saga_rollback_wf", new HashMap<>()));
        assertTrue(compensated.get());

        // Test Settle / Terminate step
        WorkflowBlueprint settleBp = new WorkflowBlueprint();
        settleBp.setWorkflowName("settle_wf");
        settleBp.setMode("sync");

        WorkflowStep settleStep = new WorkflowStep();
        settleStep.setId("settle_early");
        settleStep.setTerminate(true);
        settleStep.setReason("Early exit condition met");

        WorkflowStep neverReached = new WorkflowStep();
        neverReached.setId("unreachable");
        neverReached.setActivity("DebitAccount");

        settleBp.setSteps(List.of(settleStep, neverReached));
        engine.registerBlueprint(settleBp);

        ExecutionState settleState = engine.executeSync("settle_wf", new HashMap<>());
        assertEquals(ExecutionStatus.COMPLETED, settleState.getStatus());
        assertEquals(1, settleState.getTraces().size());
        assertEquals("terminate_settle", settleState.getTraces().get(0).getActivityName());
    }

    public static class CheckoutTaxRequest {
        public double amount;
        public String country;
    }

    public static class CheckoutTaxResponse {
        public double total_tax;
        public double total_amount;
    }

    @Test
    public void testJavaDeclarativeMultiRouteAPI() throws Exception {
        LiveEngine engine = new LiveEngine();

        // 1. Register Client-side Models
        engine.registerModel("CheckoutTaxRequest", CheckoutTaxRequest.class);
        engine.registerModel("CheckoutTaxResponse", CheckoutTaxResponse.class);
        assertEquals(CheckoutTaxRequest.class, engine.getModel("CheckoutTaxRequest"));
        assertEquals(CheckoutTaxResponse.class, engine.getModel("CheckoutTaxResponse"));

        // 2. Register Activities
        engine.registerActivity("NormalizeCartAndAddress", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("normalized", true);
            out.put("country", input.getOrDefault("country", "US").toString().toUpperCase());
            return out;
        });

        engine.registerActivity("CalculateJurisdictionTax", input -> {
            double amount = ((Number) input.getOrDefault("amount", 100.0)).doubleValue();
            String country = input.getOrDefault("country", "US").toString();
            double rate = "US".equals(country) ? 0.08 : 0.20;
            double tax = amount * rate;
            Map<String, Object> out = new HashMap<>();
            out.put("total_tax", tax);
            out.put("total_amount", amount + tax);
            return out;
        });

        engine.registerActivity("PackageTaxResponse", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("total_tax", input.get("total_tax"));
            out.put("total_amount", input.get("total_amount"));
            out.put("status", "SUCCESS");
            return out;
        });

        engine.registerActivity("FetchTaxRates", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("rates", Map.of("US", 0.08, "EU", 0.20, "UK", 0.20));
            return out;
        });

        engine.registerActivity("ValidatePostalCode", input -> {
            Map<String, Object> out = new HashMap<>();
            out.put("is_valid_postal", true);
            return out;
        });

        // 3. Define Declarative Multi-Route Blueprint
        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("checkout_tax_calculator");
        bp.setVersion("1.0.0");
        bp.setMode("sync");

        // Endpoint 1: POST /api/v1/checkout/tax
        WorkflowBlueprint.ApiEndpointConfig ep1 = new WorkflowBlueprint.ApiEndpointConfig();
        ep1.setRoute("/api/v1/checkout/tax");
        ep1.setMethod("POST");
        ep1.setInputModel("CheckoutTaxRequest");
        ep1.setOutputModel("CheckoutTaxResponse");
        ep1.setStatusCode(200);
        ep1.setSummary("Calculate sales tax and duty");
        ep1.setTags(List.of("Checkout"));
        WorkflowStep ep1_s1 = new WorkflowStep();
        ep1_s1.setId("step_normalize");
        ep1_s1.setActivity("NormalizeCartAndAddress");
        WorkflowStep ep1_s2 = new WorkflowStep();
        ep1_s2.setId("step_calc_tax");
        ep1_s2.setActivity("CalculateJurisdictionTax");
        WorkflowStep ep1_s3 = new WorkflowStep();
        ep1_s3.setId("step_package_tax");
        ep1_s3.setActivity("PackageTaxResponse");
        ep1_s3.setOutput(true);
        ep1.setSteps(List.of(ep1_s1, ep1_s2, ep1_s3));

        // Endpoint 2: GET /api/v1/checkout/tax
        WorkflowBlueprint.ApiEndpointConfig ep2 = new WorkflowBlueprint.ApiEndpointConfig();
        ep2.setRoute("/api/v1/checkout/tax");
        ep2.setMethod("GET");
        ep2.setStatusCode(200);
        ep2.setSummary("Get default tax summary");
        WorkflowStep ep2_s1 = new WorkflowStep();
        ep2_s1.setId("step_tax_rates");
        ep2_s1.setActivity("FetchTaxRates");
        ep2.setSteps(List.of(ep2_s1));

        // Endpoint 3: POST /api/v1/checkout/validate
        WorkflowBlueprint.ApiEndpointConfig ep3 = new WorkflowBlueprint.ApiEndpointConfig();
        ep3.setRoute("/api/v1/checkout/validate");
        ep3.setMethod("POST");
        ep3.setStatusCode(201);
        ep3.setSummary("Validate postal code");
        WorkflowStep ep3_s1 = new WorkflowStep();
        ep3_s1.setId("step_validate_postal");
        ep3_s1.setActivity("ValidatePostalCode");
        ep3.setSteps(List.of(ep3_s1));

        bp.setApi(List.of(ep1, ep2, ep3));
        engine.registerBlueprint(bp);

        // 4. Test programmatic executeEndpointSync
        Map<String, Object> postPayload = new HashMap<>();
        postPayload.put("job_id", "job_java_tax_001");
        postPayload.put("amount", 200.0);
        postPayload.put("country", "US");

        ExecutionState state1 = engine.executeEndpointSync("checkout_tax_calculator", ep1, "/api/v1/checkout/tax", "POST", postPayload);
        assertNotNull(state1);
        assertEquals(ExecutionStatus.COMPLETED, state1.getStatus());
        assertEquals("/api/v1/checkout/tax", state1.getRoute());
        assertEquals("POST", state1.getMethod());
        assertNotNull(state1.getOutputData());
        assertEquals(16.0, ((Number) state1.getOutputData().get("total_tax")).doubleValue(), 0.001);
        assertEquals(216.0, ((Number) state1.getOutputData().get("total_amount")).doubleValue(), 0.001);

        // 5. Test Live HTTP Server with declarative routes and headers
        int port = 19188;
        engine.startServer(port);

        try {
            java.net.http.HttpClient client = java.net.http.HttpClient.newHttpClient();

            // Test 5A: POST /api/v1/checkout/tax
            String jsonBody = "{\"amount\": 300.0, \"country\": \"EU\"}";
            java.net.http.HttpRequest reqPost = java.net.http.HttpRequest.newBuilder()
                    .uri(java.net.URI.create("http://localhost:" + port + "/api/v1/checkout/tax"))
                    .header("Content-Type", "application/json")
                    .header("X-Job-ID", "job_java_http_tax_100")
                    .POST(java.net.http.HttpRequest.BodyPublishers.ofString(jsonBody))
                    .build();

            java.net.http.HttpResponse<String> respPost = client.send(reqPost, java.net.http.HttpResponse.BodyHandlers.ofString());
            assertEquals(200, respPost.statusCode());
            assertEquals("job_java_http_tax_100", respPost.headers().firstValue("X-Job-ID").orElse(""));
            assertEquals("COMPLETED", respPost.headers().firstValue("X-Execution-Status").orElse(""));
            assertTrue(respPost.headers().firstValue("X-Execution-Duration-Ms").isPresent());
            assertTrue(respPost.body().contains("\"total_tax\":60"));
            assertTrue(respPost.body().contains("\"total_amount\":360"));
            assertTrue(respPost.body().contains("\"status\":\"SUCCESS\""));

            // Test 5B: POST /api/v1/checkout/validate (Status 201)
            java.net.http.HttpRequest reqVal = java.net.http.HttpRequest.newBuilder()
                    .uri(java.net.URI.create("http://localhost:" + port + "/api/v1/checkout/validate"))
                    .header("Content-Type", "application/json")
                    .POST(java.net.http.HttpRequest.BodyPublishers.ofString("{}"))
                    .build();

            java.net.http.HttpResponse<String> respVal = client.send(reqVal, java.net.http.HttpResponse.BodyHandlers.ofString());
            assertEquals(201, respVal.statusCode());
            assertTrue(respVal.headers().firstValue("X-Job-ID").isPresent());
            assertTrue(respVal.body().contains("\"is_valid_postal\":true"));

            // Test 5C: GET /openapi.json
            java.net.http.HttpRequest reqDocs = java.net.http.HttpRequest.newBuilder()
                    .uri(java.net.URI.create("http://localhost:" + port + "/openapi.json"))
                    .GET()
                    .build();
            java.net.http.HttpResponse<String> respDocs = client.send(reqDocs, java.net.http.HttpResponse.BodyHandlers.ofString());
            assertEquals(200, respDocs.statusCode());
            assertTrue(respDocs.body().contains("/api/v1/checkout/tax"));

            // Test 5D: Visualizer Diagram Compilation per route
            Map<String, Object> diagram = engine.getVisualizerUseCase().compileX6Diagram("checkout_tax_calculator", "job_java_http_tax_100", "/api/v1/checkout/tax", "POST");
            assertNotNull(diagram);
            assertEquals("checkout_tax_calculator", diagram.get("workflow_name"));
            assertEquals("COMPLETED", diagram.get("overall_status"));
        } finally {
            engine.stopServer();
        }
    }

    @Test
    public void testJavaHybridPaymentFastPath() {
        LiveEngine engine = new LiveEngine();

        engine.registerActivity("ScreenFraud", input -> {
            double amount = ((Number) input.getOrDefault("amount", 0.0)).doubleValue();
            double score = amount < 1000 ? 0.02 : 0.85;
            return Map.of("fraud_score", score, "fraud_decision", score < 0.5 ? "APPROVE" : "REVIEW");
        });

        engine.registerActivity("AuthorizeCard", input -> Map.of(
            "transaction_id", "txn_java_9921",
            "auth_code", "AUTH_9921",
            "settled", true
        ));

        engine.registerActivity("FormatPaymentResponse", input -> Map.of(
            "transaction_id", input.getOrDefault("transaction_id", "txn_unknown"),
            "status", "AUTHORIZED",
            "authorized_amount", input.getOrDefault("amount", 0.0),
            "currency", input.getOrDefault("currency", "USD"),
            "fraud_score", input.getOrDefault("fraud_score", 0.0),
            "durable_tracking", "REPLICATED_TO_TEMPORAL_AND_SQLITE"
        ));

        WorkflowBlueprint.ApiEndpointConfig ep = new WorkflowBlueprint.ApiEndpointConfig();
        ep.setRoute("/api/v1/payments/authorize");
        ep.setMethod("POST");
        ep.setInputModel("PaymentRequest");
        ep.setOutputModel("PaymentResponse");
        ep.setStatusCode(200);

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("step_fraud");
        s1.setActivity("ScreenFraud");

        WorkflowStep s2 = new WorkflowStep();
        s2.setId("step_auth");
        s2.setActivity("AuthorizeCard");

        WorkflowStep s3 = new WorkflowStep();
        s3.setId("step_format");
        s3.setActivity("FormatPaymentResponse");
        s3.setOutput(true);

        ep.setSteps(List.of(s1, s2, s3));

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("hybrid_payment_gateway");
        bp.setVersion("1.0.0");
        bp.setMode("hybrid");
        bp.setApi(List.of(ep));

        engine.registerBlueprint(bp);

        Map<String, Object> req = new HashMap<>();
        req.put("job_id", "tx-java-pay-001");
        req.put("amount", 499.0);
        req.put("currency", "USD");

        ExecutionState state = engine.executeEndpointSync("hybrid_payment_gateway", ep, "/api/v1/payments/authorize", "POST", req);

        assertNotNull(state);
        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        assertNotNull(state.getOutputData());
        assertEquals("AUTHORIZED", state.getOutputData().get("status"));
        assertEquals("REPLICATED_TO_TEMPORAL_AND_SQLITE", state.getOutputData().get("durable_tracking"));
        assertEquals(3, state.getTraces().size());
    }

    @Test
    public void testTemporalAsyncActivityDispatcherDirectExecution() throws InterruptedException {
        io.octopus.live.adapters.outbound.TemporalAsyncActivityDispatcher dispatcher = 
                new io.octopus.live.adapters.outbound.TemporalAsyncActivityDispatcher("localhost:7233", "default", "test-queue");

        LiveEngine engine = new LiveEngine("test-key", dispatcher);

        AtomicBoolean activityRan = new AtomicBoolean(false);
        engine.registerActivity("AsyncAuditTask", input -> {
            activityRan.set(true);
            return Map.of("processed", true);
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("async_durable_pipeline");
        bp.setVersion("1.0.0");
        bp.setMode("sync");

        WorkflowStep s1 = new WorkflowStep();
        s1.setId("step_sync");
        s1.setActivity("AsyncAuditTask");
        s1.setAsync(false);

        WorkflowStep s2 = new WorkflowStep();
        s2.setId("step_async_durable");
        s2.setActivity("AsyncAuditTask");
        s2.setAsync(true);

        bp.setSteps(List.of(s1, s2));
        engine.registerBlueprint(bp);

        ExecutionState state = engine.executeSync("async_durable_pipeline", Map.of("order_id", "ORD-123"));

        assertNotNull(state);
        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        assertEquals(2, state.getTraces().size());

        // Verify async trace attributes
        var asyncTrace = state.getTraces().get(1);
        assertEquals("ASYNC_DISPATCHED", asyncTrace.getStatus());
        assertTrue(asyncTrace.isAsync());
        assertEquals("TEMPORAL", asyncTrace.getOutputPayload().get("dispatcher"));

        // Verify Temporal task history
        assertEquals(1, dispatcher.getDispatchedTasksHistory().size());
        Map<String, Object> taskRecord = dispatcher.getDispatchedTasksHistory().get(0);
        assertTrue(taskRecord.get("temporal_workflow_id").toString().contains("octopus_act_"));
        assertEquals("test-queue", taskRecord.get("task_queue"));
        assertEquals("AsyncAuditTask", taskRecord.get("activity_name"));
    }

    @Test
    public void testTemporalAutoConfigurationFromBlueprint() {
        LiveEngine engine = new LiveEngine();
        assertEquals("IN_MEMORY", engine.getAsyncDispatcher().getDispatcherType());

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("temporal_configured_workflow");
        Map<String, Object> persistence = new HashMap<>();
        persistence.put("temporal", Map.of("host", "temporal.internal:7233", "task_queue", "payment-events"));
        bp.setPersistence(persistence);

        engine.registerBlueprint(bp);

        assertEquals("TEMPORAL", engine.getAsyncDispatcher().getDispatcherType());
        if (engine.getAsyncDispatcher() instanceof io.octopus.live.adapters.outbound.TemporalAsyncActivityDispatcher td) {
            assertEquals("temporal.internal:7233", td.getHost());
            assertEquals("payment-events", td.getTaskQueue());
        }
    }

    @Test
    public void testJavaSensitiveFieldMasking() {
        LiveEngine engine = new LiveEngine();

        final String[] receivedPassword = new String[1];
        engine.registerActivity("AuthActivity", input -> {
            receivedPassword[0] = input.getOrDefault("password", "").toString();
            Map<String, Object> out = new HashMap<>();
            out.put("access_token", "jwt_secret_token_abc123");
            out.put("status", "authenticated");
            return out;
        });

        WorkflowBlueprint bp = new WorkflowBlueprint();
        bp.setWorkflowName("secure_auth_pipeline");
        bp.setVersion("1.0.0");
        bp.setMode("sync");

        io.octopus.live.domain.models.TelemetryConfig tel = new io.octopus.live.domain.models.TelemetryConfig();
        io.octopus.live.domain.models.MaskingConfig mask = new io.octopus.live.domain.models.MaskingConfig();
        mask.setEnabled(true);
        mask.setFields(List.of("password", "access_token"));
        mask.setMaskValue("********");
        tel.setMasking(mask);
        bp.setTelemetry(tel);

        WorkflowStep step = new WorkflowStep();
        step.setId("step_auth");
        step.setActivity("AuthActivity");
        step.setMaskFields(List.of("password"));
        bp.setSteps(List.of(step));

        engine.registerBlueprint(bp);

        Map<String, Object> payload = new HashMap<>();
        payload.put("username", "alice");
        payload.put("password", "superSecretPassword999!");

        ExecutionState state = engine.executeSync("secure_auth_pipeline", payload);

        assertEquals(ExecutionStatus.COMPLETED, state.getStatus());
        // Verify in-memory activity received raw plaintext password
        assertEquals("superSecretPassword999!", receivedPassword[0]);

        // Verify trace payloads are masked
        assertEquals(1, state.getTraces().size());
        var trace = state.getTraces().get(0);
        assertEquals("********", trace.getInputPayload().get("password"));
        assertEquals("alice", trace.getInputPayload().get("username"));
        assertEquals("********", trace.getOutputPayload().get("access_token"));
        assertEquals("authenticated", trace.getOutputPayload().get("status"));

        // Verify visualizer diagram inspection nodes are masked
        Map<String, Object> diagram = engine.getVisualizerService().compileX6Diagram("secure_auth_pipeline", state.getJobId());
        assertNotNull(diagram);
        @SuppressWarnings("unchecked")
        List<Map<String, Object>> nodes = (List<Map<String, Object>>) diagram.get("nodes");
        assertNotNull(nodes);

        Map<String, Object> authNode = null;
        for (Map<String, Object> node : nodes) {
            if ("node__step_auth".equals(node.get("id"))) {
                authNode = node;
                break;
            }
        }
        assertNotNull(authNode);
        @SuppressWarnings("unchecked")
        Map<String, Object> nodeData = (Map<String, Object>) authNode.get("data");
        assertNotNull(nodeData);
        @SuppressWarnings("unchecked")
        Map<String, Object> nodeInput = (Map<String, Object>) nodeData.get("input_payload");
        assertNotNull(nodeInput);
        assertEquals("********", nodeInput.get("password"));
        @SuppressWarnings("unchecked")
        Map<String, Object> nodeOutput = (Map<String, Object>) nodeData.get("output_payload");
        assertNotNull(nodeOutput);
        assertEquals("********", nodeOutput.get("access_token"));
    }

    @Test
    public void testJavaRelationalSqlStateStoreDialects() {
        // 1. Dialect SQL Validation
        var pg = new io.octopus.live.adapters.outbound.RelationalSqlStateStore.PostgresDialect();
        assertEquals("postgres", pg.getName());
        assertEquals("$1", pg.getPlaceholder(1));
        assertTrue(pg.getUpsertExecutionStateSql().contains("ON CONFLICT (job_id) DO UPDATE"));

        var my = new io.octopus.live.adapters.outbound.RelationalSqlStateStore.MySqlDialect();
        assertEquals("mysql", my.getName());
        assertEquals("?", my.getPlaceholder(1));
        assertTrue(my.getUpsertExecutionStateSql().contains("ON DUPLICATE KEY UPDATE"));

        var ms = new io.octopus.live.adapters.outbound.RelationalSqlStateStore.SqlServerDialect();
        assertEquals("sqlserver", ms.getName());
        assertTrue(ms.getUpsertExecutionStateSql().contains("MERGE INTO execution_states"));

        var sq = new io.octopus.live.adapters.outbound.RelationalSqlStateStore.SqliteDialect();
        assertEquals("sqlite", sq.getName());
        assertTrue(sq.getUpsertExecutionStateSql().contains("ON CONFLICT(job_id) DO UPDATE"));

        // 2. StoreFactory Creation
        var storeSqlite = io.octopus.live.adapters.outbound.StoreFactory.create("sqlite", null);
        assertNotNull(storeSqlite);
        assertTrue(storeSqlite instanceof io.octopus.live.adapters.outbound.RelationalSqlStateStore);

        var storePg = io.octopus.live.adapters.outbound.StoreFactory.create("postgres", Map.of("dsn", "postgresql://localhost:5432/db"));
        assertNotNull(storePg);
        assertEquals("postgres", ((io.octopus.live.adapters.outbound.RelationalSqlStateStore) storePg).getDialect().getName());

        var storeMy = io.octopus.live.adapters.outbound.StoreFactory.create("mysql", Map.of("dsn", "mysql://localhost:3306/db"));
        assertNotNull(storeMy);
        assertEquals("mysql", ((io.octopus.live.adapters.outbound.RelationalSqlStateStore) storeMy).getDialect().getName());

        var storeMs = io.octopus.live.adapters.outbound.StoreFactory.create("sqlserver", Map.of("dsn", "sqlserver://localhost:1433"));
        assertNotNull(storeMs);
        assertEquals("sqlserver", ((io.octopus.live.adapters.outbound.RelationalSqlStateStore) storeMs).getDialect().getName());

        // 3. Functional Execution
        ExecutionState state = new ExecutionState();
        state.setJobId("java-relational-job-001");
        state.setWorkflowName("order_fulfillment_pipeline");
        state.setStatus(ExecutionStatus.COMPLETED);
        state.setRoute("/api/v1/orders/checkout");
        state.setMethod("POST");
        state.setStartedAt("2026-09-23T18:00:00Z");
        state.setData(Map.of("subtotal", 250.0));
        state.setOutputData(Map.of("total", 250.0, "status", "PROCESSED"));

        StepExecutionTrace tr = new StepExecutionTrace();
        tr.setStepId("step_reserve");
        tr.setActivityName("ReserveInventoryItem");
        tr.setStatus("COMPLETED");
        tr.setInputPayload(Map.of("sku", "SKU_JAVA"));
        tr.setOutputPayload(Map.of("reserved", true));
        tr.setDurationMs(0.68);
        state.setTraces(List.of(tr));

        storePg.saveState(state);

        ExecutionState fetched = storePg.getState("java-relational-job-001");
        assertNotNull(fetched);
        assertEquals("java-relational-job-001", fetched.getJobId());
        assertEquals("order_fulfillment_pipeline", fetched.getWorkflowName());
        assertEquals(ExecutionStatus.COMPLETED, fetched.getStatus());
        assertEquals(1, fetched.getTraces().size());
        assertEquals("ReserveInventoryItem", fetched.getTraces().get(0).getActivityName());
        assertEquals(0.68, fetched.getTraces().get(0).getDurationMs(), 0.001);

        List<ExecutionState> list = storePg.listStates("order_fulfillment_pipeline", 10);
        assertEquals(1, list.size());
        assertEquals("java-relational-job-001", list.get(0).getJobId());
    }
}



