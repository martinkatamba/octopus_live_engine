package io.octopus.examples.authportal;

import io.octopus.live.LiveEngine;
import io.octopus.live.domain.models.ExecutionState;
import io.octopus.live.domain.models.ExecutionStatus;
import io.octopus.live.domain.services.middleware.JwtHelper;

import java.util.*;

public class CustomerAuthPortalTest {

    public static void main(String[] args) throws Exception {
        System.out.println("Running Java Customer Auth Portal Test Suite...");

        LiveEngine engine = CustomerAuthPortalApplication.createEngine();
        if (new java.io.File("auth_portal_workflow.yaml").exists()) {
            engine.loadBlueprintsFromDirectory(".");
        } else {
            engine.loadBlueprintsFromDirectory("octopus_live_engine/examples/java/customer_auth_portal");
        }

        testLoginAndTokenGeneration(engine);
        testProfileLookup(engine);
        testVipCheckoutBranching(engine);
        testStandardCheckoutBranching(engine);
        testTemporalDurableAsyncExecution(engine);
        testMultilangTaggedExecution(engine);

        System.out.println("\n✅ ALL JAVA TESTS PASSED SUCCESSFULLY!\n");
    }

    private static void testTemporalDurableAsyncExecution(LiveEngine engine) {
        System.out.print("Testing Temporal-Backed Durable Async Execution... ");

        assert "TEMPORAL".equals(engine.getAsyncDispatcher().getDispatcherType()) : "Expected TEMPORAL dispatcher";
        var temporalDispatcher = (io.octopus.live.adapters.outbound.TemporalAsyncActivityDispatcher) engine.getAsyncDispatcher();
        assert "auth_portal_api".equals(temporalDispatcher.getTaskQueue()) : "Expected auth_portal_api task queue";

        int initialCount = temporalDispatcher.getDispatchedTasksHistory().size();

        Map<String, Object> failPayload = new HashMap<>();
        failPayload.put("job_id", "test_temporal_async_job_" + System.currentTimeMillis());
        failPayload.put("username", "sarah_vip");
        failPayload.put("password", "WRONG_PASSWORD");

        try {
            engine.executeEndpointSync(
                "customer_auth_portal",
                null,
                "/api/v1.0.0/customer_auth_portal/auth/login",
                "POST",
                failPayload
            );
        } catch (Exception expected) {
            // expected 401
        }

        // Verify that LogSecurityAuditViolation was durably queued to Temporal
        List<Map<String, Object>> history = temporalDispatcher.getDispatchedTasksHistory();
        assert history.size() > initialCount : "Expected new task in Temporal dispatch history";

        Map<String, Object> lastTask = history.get(history.size() - 1);
        assert "LogSecurityAuditViolation".equals(lastTask.get("activity_name")) : "Expected LogSecurityAuditViolation activity";
        assert "auth_portal_api".equals(lastTask.get("task_queue")) : "Expected auth_portal_api queue";
        assert lastTask.get("temporal_workflow_id").toString().contains("octopus_act_") : "Expected octopus_act_ workflow ID prefix";

        // Check state store traces
        ExecutionState failState = engine.getStateStore().getState(failPayload.get("job_id").toString());
        assert failState != null : "State should be saved in state store";
        
        boolean foundTemporalTrace = false;
        for (io.octopus.live.domain.models.StepExecutionTrace tr : failState.getTraces()) {
            if ("LogSecurityAuditViolation".equals(tr.getActivityName())) {
                assert "ASYNC_DISPATCHED".equals(tr.getStatus()) : "Trace status should be ASYNC_DISPATCHED";
                assert tr.isAsync() : "Trace isAsync should be true";
                assert "TEMPORAL".equals(tr.getOutputPayload().get("dispatcher")) : "Dispatcher in trace output should be TEMPORAL";
                foundTemporalTrace = true;
            }
        }
        assert foundTemporalTrace : "LogSecurityAuditViolation trace should be present in state";

        System.out.println("PASSED ✓");
    }

    private static void testMultilangTaggedExecution(LiveEngine engine) {
        System.out.println("🟣 ==================== [JAVA ENGINE TESTS] ====================");

        long ts = System.currentTimeMillis();

        // 1. Tagged Login Success
        String jobLoginOk = "java-login-success-" + ts;
        System.out.println("\n[JAVA] 1. Execute login (Job ID: " + jobLoginOk + ")");
        Map<String, Object> loginPayload = new HashMap<>();
        loginPayload.put("job_id", jobLoginOk);
        loginPayload.put("username", "sarah_vip");
        loginPayload.put("password", "password123");

        ExecutionState loginState = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/auth/login",
            "POST",
            loginPayload
        );
        String token = (String) loginState.getOutputData().get("access_token");
        System.out.println("  Status: " + loginState.getStatus() + " | User: " + loginState.getOutputData().get("user_id") + " | Tier: " + loginState.getOutputData().get("customer_tier"));

        // 2. Tagged VIP Checkout
        String jobCheckout = "java-checkout-vip-" + ts;
        System.out.println("\n[JAVA] 2. Execute checkout (Job ID: " + jobCheckout + ")");
        Map<String, Object> claims = JwtHelper.decodeAndVerify(token, CustomerAuthPortalApplication.JWT_SECRET);
        List<Map<String, Object>> items = new ArrayList<>();
        Map<String, Object> item1 = new HashMap<>();
        item1.put("sku", "SONY-WH1000XM5");
        item1.put("description", "VIP Audio");
        item1.put("unit_price", 400.00);
        item1.put("quantity", 2);
        items.add(item1);

        Map<String, Object> checkoutPayload = new HashMap<>();
        checkoutPayload.put("job_id", jobCheckout);
        checkoutPayload.put("cart_id", "cart_java_001");
        checkoutPayload.put("user_id", claims.get("user_id"));
        checkoutPayload.put("customer_tier", claims.get("customer_tier"));
        checkoutPayload.put("items", items);

        ExecutionState checkoutState = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST",
            checkoutPayload
        );
        System.out.println("  Status: " + checkoutState.getStatus() + " | Subtotal: $" + checkoutState.getOutputData().get("subtotal") + " | Final: $" + checkoutState.getOutputData().get("final_total") + " | Tier: " + checkoutState.getOutputData().get("pricing_tier"));

        // 3. Tagged Login Failure
        String jobLoginFail = "java-login-fail-" + ts;
        System.out.println("\n[JAVA] 3. Execute login failure (Job ID: " + jobLoginFail + ")");
        Map<String, Object> failPayload = new HashMap<>();
        failPayload.put("job_id", jobLoginFail);
        failPayload.put("username", "sarah_vip");
        failPayload.put("password", "WRONG_PASSWORD_XYZ");

        try {
            engine.executeEndpointSync(
                "customer_auth_portal",
                null,
                "/api/v1.0.0/customer_auth_portal/auth/login",
                "POST",
                failPayload
            );
        } catch (Exception ex) {
            System.out.println("  Caught Expected Exception: " + ex.getMessage());
        }

        ExecutionState failState = engine.getStateStore().getState(jobLoginFail);
        int traceCount = failState != null && failState.getTraces() != null ? failState.getTraces().size() : 0;
        System.out.println("\n[JAVA] Execution Traces for " + jobLoginFail + " (" + traceCount + " traces):");
        if (failState != null && failState.getTraces() != null) {
            for (io.octopus.live.domain.models.StepExecutionTrace tr : failState.getTraces()) {
                System.out.println("   • Step: " + tr.getStepId() + " | Activity: " + tr.getActivityName() + " | Status: " + tr.getStatus());
            }
        }
    }

    private static void testLoginAndTokenGeneration(LiveEngine engine) {
        System.out.print("Testing Login & Token Generation... ");

        Map<String, Object> payload = new HashMap<>();
        payload.put("username", "sarah_vip");
        payload.put("password", "password123");
        payload.put("job_id", "test_login_001");

        ExecutionState state = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/auth/login",
            "POST",
            payload
        );

        assert state.getStatus() == ExecutionStatus.COMPLETED : "Expected COMPLETED status";
        Map<String, Object> out = state.getOutputData();
        assert out != null : "Output data should not be null";
        assert "usr_9988_vip".equals(out.get("user_id")) : "Expected user_id usr_9988_vip";
        assert "VIP".equals(out.get("customer_tier")) : "Expected customer_tier VIP";
        assert out.get("access_token") != null : "Token should be present";

        // Verify JWT token signature
        Map<String, Object> claims = JwtHelper.decodeAndVerify(out.get("access_token").toString(), CustomerAuthPortalApplication.JWT_SECRET);
        assert "usr_9988_vip".equals(claims.get("user_id")) : "Claim user_id mismatch";
        assert "VIP".equals(claims.get("customer_tier")) : "Claim customer_tier mismatch";

        System.out.println("PASSED ✓");
    }

    private static void testProfileLookup(LiveEngine engine) {
        System.out.print("Testing Profile Lookup... ");

        Map<String, Object> payload = new HashMap<>();
        payload.put("user_id", "usr_9988_vip");
        payload.put("job_id", "test_profile_001");

        ExecutionState state = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/profile",
            "GET",
            payload
        );

        assert state.getStatus() == ExecutionStatus.COMPLETED : "Expected COMPLETED status";
        Map<String, Object> out = state.getOutputData();
        assert "sarah_vip".equals(out.get("username")) : "Expected sarah_vip username";
        assert "ACTIVE_VIP".equals(out.get("account_status")) : "Expected ACTIVE_VIP status";

        System.out.println("PASSED ✓");
    }

    private static void testVipCheckoutBranching(LiveEngine engine) {
        System.out.print("Testing VIP Checkout (20% Discount Branch)... ");

        List<Map<String, Object>> items = new ArrayList<>();
        Map<String, Object> item1 = new HashMap<>();
        item1.put("sku", "ITEM-101");
        item1.put("description", "4K Gaming Monitor");
        item1.put("price", 500.00);
        item1.put("quantity", 1);
        items.add(item1);

        Map<String, Object> payload = new HashMap<>();
        payload.put("user_id", "usr_9988_vip");
        payload.put("customer_tier", "VIP");
        payload.put("items", items);
        payload.put("job_id", "test_vip_checkout_001");

        ExecutionState state = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST",
            payload
        );

        assert state.getStatus() == ExecutionStatus.COMPLETED : "Expected COMPLETED status";
        Map<String, Object> out = state.getOutputData();
        assert Math.abs(((Number) out.get("subtotal")).doubleValue() - 500.00) < 0.01 : "Expected 500.00 subtotal";
        assert Math.abs(((Number) out.get("discount_applied")).doubleValue() - 100.00) < 0.01 : "Expected 100.00 discount (20%)";
        assert Math.abs(((Number) out.get("final_total")).doubleValue() - 400.00) < 0.01 : "Expected 400.00 final total";
        assert out.get("pricing_tier").toString().contains("VIP_PREFERRED") : "Expected VIP pricing tier";

        System.out.println("PASSED ✓");
    }

    private static void testStandardCheckoutBranching(LiveEngine engine) {
        System.out.print("Testing Standard Checkout (Retail Pricing Branch)... ");

        List<Map<String, Object>> items = new ArrayList<>();
        Map<String, Object> item1 = new HashMap<>();
        item1.put("sku", "ITEM-101");
        item1.put("description", "4K Gaming Monitor");
        item1.put("price", 500.00);
        item1.put("quantity", 1);
        items.add(item1);

        Map<String, Object> payload = new HashMap<>();
        payload.put("user_id", "usr_1024_std");
        payload.put("customer_tier", "STANDARD");
        payload.put("items", items);
        payload.put("job_id", "test_std_checkout_001");

        ExecutionState state = engine.executeEndpointSync(
            "customer_auth_portal",
            null,
            "/api/v1.0.0/customer_auth_portal/cart/checkout",
            "POST",
            payload
        );

        assert state.getStatus() == ExecutionStatus.COMPLETED : "Expected COMPLETED status";
        Map<String, Object> out = state.getOutputData();
        assert Math.abs(((Number) out.get("subtotal")).doubleValue() - 500.00) < 0.01 : "Expected 500.00 subtotal";
        assert Math.abs(((Number) out.get("discount_applied")).doubleValue() - 0.00) < 0.01 : "Expected 0.00 discount";
        assert Math.abs(((Number) out.get("final_total")).doubleValue() - 500.00) < 0.01 : "Expected 500.00 final total";
        assert out.get("pricing_tier").toString().contains("STANDARD_RETAIL") : "Expected Standard pricing tier";

        System.out.println("PASSED ✓");
    }
}
