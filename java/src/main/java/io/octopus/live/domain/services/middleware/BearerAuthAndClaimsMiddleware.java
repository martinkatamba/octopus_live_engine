package io.octopus.live.domain.services.middleware;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;

public class BearerAuthAndClaimsMiddleware extends LiveMiddleware {
    private String tokenHeader = "Authorization";
    private List<String> claimsToInject = new ArrayList<>();
    private String jwtSecret = JwtHelper.DEFAULT_SECRET;
    private Function<String, Map<String, Object>> customValidator;

    public BearerAuthAndClaimsMiddleware() {
        super("BearerAuthAndClaimsMiddleware");
    }

    public String getTokenHeader() { return tokenHeader; }
    public void setTokenHeader(String tokenHeader) { this.tokenHeader = tokenHeader != null ? tokenHeader : "Authorization"; }

    public List<String> getClaimsToInject() { return claimsToInject; }
    public void setClaimsToInject(List<String> claimsToInject) { this.claimsToInject = claimsToInject != null ? claimsToInject : new ArrayList<>(); }

    public String getJwtSecret() { return jwtSecret; }
    public void setJwtSecret(String jwtSecret) { this.jwtSecret = jwtSecret; }

    public Function<String, Map<String, Object>> getCustomValidator() { return customValidator; }
    public void setCustomValidator(Function<String, Map<String, Object>> customValidator) { this.customValidator = customValidator; }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        String authHeader = ctx.getHeaders().get(tokenHeader);
        if (authHeader == null || authHeader.isBlank()) {
            authHeader = ctx.getHeaders().get(tokenHeader.toLowerCase());
        }

        if (authHeader == null || authHeader.isBlank()) {
            throw new UserNotAuthenticatedException("Authentication required. Please provide a valid Bearer token via the Authorization header.");
        }

        String token = authHeader.trim();
        if (token.toLowerCase().startsWith("bearer ")) {
            token = token.substring(7).trim();
        }

        Map<String, Object> claims;
        if (customValidator != null) {
            try {
                claims = customValidator.apply(token);
            } catch (Exception ex) {
                throw new UserNotAuthenticatedException("Token validation error: " + ex.getMessage());
            }
        } else {
            claims = JwtHelper.decodeAndVerify(token, jwtSecret);
        }

        ctx.setAuthClaims(claims);

        // Inject claims into workflow execution context
        Map<String, Object> injections = new HashMap<>();
        if (claimsToInject != null && !claimsToInject.isEmpty()) {
            for (String claimKey : claimsToInject) {
                if (claims.containsKey(claimKey)) {
                    injections.put(claimKey, claims.get(claimKey));
                } else if ("user_id".equals(claimKey) && claims.containsKey("sub")) {
                    injections.put("user_id", claims.get("sub"));
                } else if ("sub".equals(claimKey) && claims.containsKey("user_id")) {
                    injections.put("sub", claims.get("user_id"));
                }
            }
        } else {
            injections.putAll(claims);
        }

        ctx.getWorkflowInjections().putAll(injections);
    }
}
