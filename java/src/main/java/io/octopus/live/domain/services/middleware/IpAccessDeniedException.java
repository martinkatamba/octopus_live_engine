package io.octopus.live.domain.services.middleware;

import java.util.HashMap;
import java.util.Map;

public class IpAccessDeniedException extends OctopusMiddlewareException {
    public IpAccessDeniedException(String detail, String clientIp, String path, String method) {
        super("IpAccessDenied", 403,
            detail != null ? detail : String.format("Access denied: Client IP '%s' is not in the allowed IP whitelist", clientIp),
            createDetails(clientIp, path, method),
            new HashMap<>());
    }

    private static Map<String, Object> createDetails(String clientIp, String path, String method) {
        Map<String, Object> d = new HashMap<>();
        d.put("client_ip", clientIp);
        d.put("path", path);
        d.put("method", method);
        return d;
    }
}
