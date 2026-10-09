package io.octopus.live.adapters.inbound;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;

public class VisualizerHtmlRenderer {

    private static String cachedHtml = null;

    public static String renderHtml(String defaultDomain, String routePrefix) {
        if (cachedHtml == null) {
            try (InputStream is = VisualizerHtmlRenderer.class.getResourceAsStream("/visualizer.html")) {
                if (is != null) {
                    cachedHtml = new String(is.readAllBytes(), StandardCharsets.UTF_8);
                } else {
                    cachedHtml = "<html><body>Visualizer template not found</body></html>";
                }
            } catch (Exception e) {
                cachedHtml = "<html><body>Error loading visualizer: " + e.getMessage() + "</body></html>";
            }
        }

        return cachedHtml.replace("{{DEFAULT_DOMAIN}}", defaultDomain)
                         .replace("{{DEFAULT_PREFIX}}", routePrefix)
                         .replace("{{ROUTE_PREFIX}}", routePrefix);
    }
}
