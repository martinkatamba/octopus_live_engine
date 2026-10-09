package io.octopus.examples.authportal;

import io.octopus.live.LiveEngine;
import io.octopus.live.domain.services.middleware.JwtHelper;
import io.octopus.live.domain.services.middleware.UserNotAuthenticatedException;
import io.octopus.live.domain.services.middleware.InvalidCredentialsException;

import java.util.*;

public class CustomerAuthPortalApplication {

    public static final String JWT_SECRET = JwtHelper.DEFAULT_SECRET;

    // =========================================================================
    // 1. Domain Models
    // =========================================================================

    public static class LoginRequest {
        public String username;
        public String password;
    }

    public static class LoginResponse {
        public String access_token;
        public String token_type = "Bearer";
        public String user_id;
        public String customer_tier;
        public int expires_in_seconds = 3600;
    }

    public static class UserProfileResponse {
        public String user_id;
        public String username;
        public String email;
        public String customer_tier;
        public String account_status;
        public String member_since;
        public List<String> roles = new ArrayList<>();
    }

    public static class CartItem {
        public String sku;
        public String description;
        public int quantity = 1;
        public double unit_price;
    }

    public static class CheckoutRequest {
        public String cart_id;
        public List<CartItem> items = new ArrayList<>();
        public String shipping_address;
    }

    public static class CheckoutResponse {
        public String order_id;
        public String customer_id;
        public String customer_tier;
        public double subtotal;
        public double discount_applied;
        public double final_total;
        public String pricing_tier;
        public String shipping_method;
        public double fast_path_latency_ms;
    }

    // =========================================================================
    // 2. In-Memory User Database
    // =========================================================================

    public static class UserRecord {
        public String userId;
        public String username;
        public String email;
        public String password;
        public String customerTier;
        public List<String> roles;
        public String accountStatus;
        public String memberSince;

        public UserRecord(String userId, String username, String email, String password, String customerTier, List<String> roles, String accountStatus, String memberSince) {
            this.userId = userId;
            this.username = username;
            this.email = email;
            this.password = password;
            this.customerTier = customerTier;
            this.roles = roles;
            this.accountStatus = accountStatus;
            this.memberSince = memberSince;
        }
    }

    public static final Map<String, UserRecord> USER_DB = new HashMap<>();
    static {
        USER_DB.put("sarah_vip", new UserRecord(
            "usr_9988_vip", "sarah_vip", "sarah@acme-corp.com", "password123", "VIP",
            List.of("customer", "vip_member"), "ACTIVE_VIP", "2024-01-15"
        ));
        USER_DB.put("john_doe", new UserRecord(
            "usr_1024_std", "john_doe", "john.doe@example.com", "password123", "STANDARD",
            List.of("customer"), "ACTIVE_STANDARD", "2025-06-20"
        ));
    }

    // =========================================================================
    // 3. Application Setup & Activity Registration
    // =========================================================================

    public static LiveEngine createEngine() {
        LiveEngine engine = new LiveEngine();

        // 1. Register Models for Schema and Contract Validation
        engine.registerModel("LoginRequest", LoginRequest.class);
        engine.registerModel("LoginResponse", LoginResponse.class);
        engine.registerModel("UserProfileResponse", UserProfileResponse.class);
        engine.registerModel("CartItem", CartItem.class);
        engine.registerModel("CheckoutRequest", CheckoutRequest.class);
        engine.registerModel("CheckoutResponse", CheckoutResponse.class);

        // 2. Activity: VerifyUserCredentials
        engine.registerActivity("VerifyUserCredentials", input -> {
            String username = input.get("username") != null ? input.get("username").toString() : "";
            String password = input.get("password") != null ? input.get("password").toString() : "";

            if ("crash_user".equals(username)) {
                throw new RuntimeException("division by zero during unexpected calculation crash");
            }

            UserRecord user = USER_DB.get(username);
            if (user == null || !user.password.equals(password)) {
                throw new InvalidCredentialsException("Invalid username or password. Please try again.");
            }

            Map<String, Object> out = new HashMap<>();
            out.put("authenticated", true);
            out.put("user_id", user.userId);
            out.put("verified_username", user.username);
            out.put("email", user.email);
            out.put("customer_tier", user.customerTier);
            out.put("roles", user.roles);
            return out;
        });

        // 3. Activity: IssueBearerToken
        engine.registerActivity("IssueBearerToken", input -> {
            String userId = input.getOrDefault("user_id", "usr_unknown").toString();
            String tier = input.getOrDefault("customer_tier", "STANDARD").toString();
            String username = input.getOrDefault("verified_username", "anonymous").toString();
            String email = input.getOrDefault("email", "").toString();
            Object rolesObj = input.getOrDefault("roles", List.of("customer"));

            Map<String, Object> claims = new HashMap<>();
            claims.put("sub", userId);
            claims.put("user_id", userId);
            claims.put("username", username);
            claims.put("email", email);
            claims.put("customer_tier", tier);
            claims.put("roles", rolesObj);

            String token = JwtHelper.createToken(claims, JWT_SECRET, 3600);

            Map<String, Object> out = new HashMap<>();
            out.put("access_token", token);
            out.put("token_type", "Bearer");
            out.put("user_id", userId);
            out.put("customer_tier", tier);
            out.put("expires_in_seconds", 3600);
            return out;
        });

        // 4. Activity: FetchUserProfile
        engine.registerActivity("FetchUserProfile", input -> {
            String userId = input.getOrDefault("user_id", "").toString();

            UserRecord matched = null;
            for (UserRecord u : USER_DB.values()) {
                if (u.userId.equals(userId)) {
                    matched = u;
                    break;
                }
            }

            if (matched == null) {
                // Fallback from token claims
                matched = new UserRecord(
                    userId,
                    input.getOrDefault("username", "user_" + userId).toString(),
                    input.getOrDefault("email", userId + "@portal.local").toString(),
                    "",
                    input.getOrDefault("customer_tier", "STANDARD").toString(),
                    List.of("customer"),
                    "ACTIVE",
                    "2026-01-01"
                );
            }

            Map<String, Object> out = new HashMap<>();
            out.put("user_id", matched.userId);
            out.put("username", matched.username);
            out.put("email", matched.email);
            out.put("customer_tier", matched.customerTier);
            out.put("account_status", matched.accountStatus);
            out.put("member_since", matched.memberSince);
            out.put("roles", matched.roles);
            return out;
        });

        // 5. Activity: ValidateCartItems
        engine.registerActivity("ValidateCartItems", input -> {
            double subtotal = 0.0;
            Object itemsObj = input.get("items");

            if (itemsObj instanceof List<?> list) {
                for (Object item : list) {
                    if (item instanceof Map<?, ?> m) {
                        int qty = 1;
                        Object q = m.get("quantity");
                        if (q instanceof Number num) qty = num.intValue();
                        else if (q != null) qty = Integer.parseInt(q.toString());

                        double price = 0.0;
                        Object p = m.get("unit_price");
                        if (p == null) p = m.get("price");
                        if (p instanceof Number num) price = num.doubleValue();
                        else if (p != null) price = Double.parseDouble(p.toString());

                        subtotal += qty * price;
                    }
                }
            }

            Map<String, Object> out = new HashMap<>();
            out.put("subtotal", Math.round(subtotal * 100.0) / 100.0);
            out.put("cart_validated", true);
            return out;
        });

        // 6. Activity: ApplyVIPDiscount (VIP Branch)
        engine.registerActivity("ApplyVIPDiscount", input -> {
            double subtotal = input.get("subtotal") != null ? Double.parseDouble(input.get("subtotal").toString()) : 0.0;
            double discount = Math.round((subtotal * 0.20) * 100.0) / 100.0;
            double finalTotal = Math.round((subtotal - discount) * 100.0) / 100.0;

            Map<String, Object> out = new HashMap<>();
            out.put("discount_applied", discount);
            out.put("final_total", finalTotal);
            out.put("pricing_tier", "VIP_PREFERRED (20% OFF)");
            out.put("shipping_method", "Priority Express (Free for VIP)");
            return out;
        });

        // 7. Activity: ApplyStandardPricing (Standard Branch)
        engine.registerActivity("ApplyStandardPricing", input -> {
            double subtotal = input.get("subtotal") != null ? Double.parseDouble(input.get("subtotal").toString()) : 0.0;

            Map<String, Object> out = new HashMap<>();
            out.put("discount_applied", 0.0);
            out.put("final_total", subtotal);
            out.put("pricing_tier", "STANDARD_RETAIL");
            out.put("shipping_method", "Standard Ground ($4.99)");
            return out;
        });

        // 8. Activity: FinalizeCheckoutOrder
        engine.registerActivity("FinalizeCheckoutOrder", input -> {
            String customerId = input.getOrDefault("user_id", "usr_unknown").toString();
            String tier = input.getOrDefault("customer_tier", "STANDARD").toString();
            double subtotal = input.get("subtotal") != null ? Double.parseDouble(input.get("subtotal").toString()) : 0.0;
            double discount = input.get("discount_applied") != null ? Double.parseDouble(input.get("discount_applied").toString()) : 0.0;
            double finalTotal = input.get("final_total") != null ? Double.parseDouble(input.get("final_total").toString()) : subtotal;
            String pricingTier = input.getOrDefault("pricing_tier", "STANDARD_RETAIL").toString();
            String shippingMethod = input.getOrDefault("shipping_method", "Standard Ground").toString();
            String orderId = "ORD-" + System.currentTimeMillis();

            Map<String, Object> out = new HashMap<>();
            out.put("order_id", orderId);
            out.put("customer_id", customerId);
            out.put("customer_tier", tier);
            out.put("subtotal", subtotal);
            out.put("discount_applied", discount);
            out.put("final_total", finalTotal);
            out.put("pricing_tier", pricingTier);
            out.put("shipping_method", shippingMethod);
            out.put("fast_path_latency_ms", 0.42);
            return out;
        });

        // 9. Activity: LogSecurityAuditViolation
        engine.registerActivity("LogSecurityAuditViolation", input -> {
            System.err.printf("🔒 [AUDIT LOG ACTIVITY] Security violation logged: %s%n", input);
            return Collections.emptyMap();
        });

        // 10. Activity: RecordResponseAuditLog (Sample Post-Request Middleware Hook)
        engine.registerActivity("RecordResponseAuditLog", input -> {
            String jobId = input.getOrDefault("job_id", "job_unknown").toString();
            String route = input.getOrDefault("route", "").toString();
            int sc = input.get("status_code") instanceof Number n ? n.intValue() : 200;
            double duration = input.get("duration_ms") instanceof Number n ? n.doubleValue() : 0.0;
            String user = input.getOrDefault("user_id", "anonymous").toString();
            String tier = input.getOrDefault("customer_tier", "STANDARD").toString();

            System.out.printf("📊 [POST-REQUEST AUDIT] Job=%s Route=%s Status=%d User=%s Tier=%s Latency=%.2fms%n",
                    jobId, route, sc, user, tier, duration);

            Map<String, Object> out = new HashMap<>();
            out.put("audited", true);
            out.put("job_id", jobId);
            out.put("timestamp", System.currentTimeMillis());
            return out;
        });

        // 11. Activity: HandleGlobalUnhandledException
        engine.registerActivity("HandleGlobalUnhandledException", input -> {
            String jobId = input.getOrDefault("job_id", "job_unknown").toString();
            String stepId = input.getOrDefault("step_id", input.getOrDefault("failing_step_id", "unknown")).toString();
            String errType = input.getOrDefault("error_type", "UnhandledException").toString();
            String errMsg = input.getOrDefault("error_message", input.getOrDefault("message", "Internal error")).toString();

            System.err.printf("🚨 [GLOBAL EXCEPTION HANDLER] Incident logged for Job=%s Step=%s: %s - %s%n",
                    jobId, stepId, errType, errMsg);

            Map<String, Object> out = new HashMap<>();
            out.put("incident_id", "inc_" + System.currentTimeMillis());
            out.put("handled", true);
            out.put("status", "INCIDENT_LOGGED");
            out.put("error_type", errType);
            out.put("message", "An unexpected internal server error occurred. Incident reference logged.");
            out.put("timestamp", System.currentTimeMillis());
            return out;
        });

        return engine;
    }

    public static void main(String[] args) throws Exception {
        int port = 8084;
        boolean workerOnly = false;

        for (int i = 0; i < args.length; i++) {
            if ("-port".equalsIgnoreCase(args[i]) || "--port".equalsIgnoreCase(args[i])) {
                if (i + 1 < args.length) port = Integer.parseInt(args[i + 1]);
            } else if ("--worker".equalsIgnoreCase(args[i]) || "--worker-only".equalsIgnoreCase(args[i])) {
                workerOnly = true;
            }
        }
        if (System.getenv("PORT") != null) {
            port = Integer.parseInt(System.getenv("PORT"));
        }
        if ("true".equalsIgnoreCase(System.getenv("WORKER_ONLY"))) {
            workerOnly = true;
        }

        LiveEngine engine = createEngine();
        engine.loadBlueprintsFromDirectory(".");

        if (workerOnly) {
            System.out.println("\n==================================================================");
            System.out.println("👷 Dedicated Standalone Temporal Worker Node Started");
            System.out.println("TaskQueue: auth_portal_api -> Polling Temporal Server (127.0.0.1:7233)");
            System.out.println("==================================================================\n");
            engine.startWorker();
        } else {
            engine.startServer(port);
            System.out.println("\n==================================================================");
            System.out.println("🚀 Customer Auth Portal API (Java 21+) listening on http://0.0.0.0:" + port);
            System.out.println("👷 Embedded Temporal Worker Active on TaskQueue 'auth_portal_api'");
            System.out.println("📖 Interactive Swagger Docs: http://localhost:" + port + "/docs");
            System.out.println("🎨 Sleek Live Visualizer:   http://localhost:" + port + "/visualizer?domain=customer_auth_portal");
            System.out.println("==================================================================\n");
        }

        // Keep process alive
        Thread.currentThread().join();
    }
}
