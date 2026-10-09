package io.octopus.live.domain.services.middleware;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

public class RoleAuthorizationMiddleware extends LiveMiddleware {
    private Set<String> requiredRoles = new HashSet<>();
    private String rolesHeader = "X-User-Roles";

    public RoleAuthorizationMiddleware() {
        super("RoleAuthorizationMiddleware");
    }

    public Set<String> getRequiredRoles() { return requiredRoles; }
    public void setRequiredRoles(Set<String> requiredRoles) { this.requiredRoles = requiredRoles != null ? requiredRoles : new HashSet<>(); }

    public String getRolesHeader() { return rolesHeader; }
    public void setRolesHeader(String rolesHeader) { this.rolesHeader = rolesHeader != null ? rolesHeader : "X-User-Roles"; }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        if (requiredRoles == null || requiredRoles.isEmpty()) {
            return;
        }

        List<String> userRoles = new ArrayList<>();
        if (ctx.getAuthClaims() != null && !ctx.getAuthClaims().isEmpty()) {
            Object r = ctx.getAuthClaims().get("roles");
            if (r == null) r = ctx.getAuthClaims().get("role");
            if (r instanceof List<?> list) {
                for (Object item : list) {
                    if (item != null) userRoles.add(item.toString());
                }
            } else if (r != null) {
                userRoles.add(r.toString());
            }
        }

        if (userRoles.isEmpty()) {
            String rawHdr = ctx.getHeaders().get(rolesHeader);
            if (rawHdr == null) rawHdr = ctx.getHeaders().get(rolesHeader.toLowerCase());
            if (rawHdr != null) {
                for (String part : rawHdr.split(",")) {
                    if (!part.isBlank()) userRoles.add(part.trim());
                }
            }
        }

        boolean matched = false;
        for (String role : userRoles) {
            if (requiredRoles.contains(role) || requiredRoles.contains(role.toLowerCase()) || requiredRoles.contains(role.toUpperCase())) {
                matched = true;
                break;
            }
            if ("domain_admin".equalsIgnoreCase(role) || "admin".equalsIgnoreCase(role) || "system".equalsIgnoreCase(role)) {
                matched = true;
                break;
            }
        }

        if (!matched) {
            throw new UserNotAuthorizedException("User lacks required role(s): " + requiredRoles);
        }
    }
}
