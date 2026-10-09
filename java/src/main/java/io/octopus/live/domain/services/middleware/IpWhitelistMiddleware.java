package io.octopus.live.domain.services.middleware;

import java.net.InetAddress;
import java.util.ArrayList;
import java.util.List;

public class IpWhitelistMiddleware extends LiveMiddleware {
    private List<String> allowedIps = new ArrayList<>();
    private List<String> blockedIps = new ArrayList<>();
    private boolean allowAll = false;

    public IpWhitelistMiddleware() {
        super("IpWhitelistMiddleware");
    }

    public List<String> getAllowedIps() { return allowedIps; }
    public void setAllowedIps(List<String> allowedIps) {
        this.allowedIps = allowedIps != null ? allowedIps : new ArrayList<>();
        this.allowAll = this.allowedIps.contains("*");
    }

    public List<String> getBlockedIps() { return blockedIps; }
    public void setBlockedIps(List<String> blockedIps) { this.blockedIps = blockedIps != null ? blockedIps : new ArrayList<>(); }

    public boolean isAllowAll() { return allowAll; }
    public void setAllowAll(boolean allowAll) { this.allowAll = allowAll; }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        String clientIp = ctx.getClientIp();
        if (clientIp == null || clientIp.isBlank()) {
            clientIp = "127.0.0.1";
        }

        // 1. Check blacklist
        if (isIpInList(clientIp, blockedIps)) {
            throw new IpAccessDeniedException("Access forbidden: Client IP network is not authorized.", clientIp, ctx.getPath(), ctx.getMethod());
        }

        // 2. Check allowAll
        if (allowAll) {
            return;
        }

        // 3. Check allowed list
        if (allowedIps != null && !allowedIps.isEmpty()) {
            if (isIpInList(clientIp, allowedIps)) {
                return;
            }
            throw new IpAccessDeniedException("Access forbidden: Client IP network is not authorized.", clientIp, ctx.getPath(), ctx.getMethod());
        }

        // 4. Default: allow loopback
        if (isLoopback(clientIp)) {
            return;
        }

        throw new IpAccessDeniedException("Access forbidden: Client IP network is not authorized.", clientIp, ctx.getPath(), ctx.getMethod());
    }

    private boolean isIpInList(String ip, List<String> list) {
        if (list == null || list.isEmpty()) return false;
        String normalizedClient = normalizeIp(ip);

        for (String rule : list) {
            if (rule == null || rule.isBlank()) continue;
            rule = rule.trim();
            if ("*".equals(rule)) return true;

            // Direct comparison
            if (rule.equalsIgnoreCase(ip) || rule.equalsIgnoreCase(normalizedClient)) {
                return true;
            }

            // CIDR check
            if (rule.contains("/")) {
                if (matchesCidr(normalizedClient, rule)) {
                    return true;
                }
            }
        }
        return false;
    }

    private String normalizeIp(String ip) {
        if (ip == null) return "127.0.0.1";
        String s = ip.trim();
        if (s.startsWith("::ffff:")) {
            return s.substring(7);
        }
        if ("::1".equals(s) || "0:0:0:0:0:0:0:1".equals(s)) {
            return "127.0.0.1";
        }
        return s;
    }

    private boolean isLoopback(String ip) {
        String n = normalizeIp(ip);
        return n.startsWith("127.") || "localhost".equalsIgnoreCase(n) || "::1".equals(ip);
    }

    private boolean matchesCidr(String ip, String cidr) {
        try {
            String[] parts = cidr.split("/");
            if (parts.length != 2) return false;

            InetAddress targetAddr = InetAddress.getByName(ip);
            InetAddress subnetAddr = InetAddress.getByName(parts[0]);
            int prefixLen = Integer.parseInt(parts[1]);

            byte[] targetBytes = targetAddr.getAddress();
            byte[] subnetBytes = subnetAddr.getAddress();

            if (targetBytes.length != subnetBytes.length) return false;

            int fullBytes = prefixLen / 8;
            int remBits = prefixLen % 8;

            for (int i = 0; i < fullBytes; i++) {
                if (targetBytes[i] != subnetBytes[i]) return false;
            }

            if (remBits > 0 && fullBytes < targetBytes.length) {
                int mask = (0xFF << (8 - remBits)) & 0xFF;
                if ((targetBytes[fullBytes] & mask) != (subnetBytes[fullBytes] & mask)) {
                    return false;
                }
            }

            return true;
        } catch (Exception ignored) {
            return false;
        }
    }
}
