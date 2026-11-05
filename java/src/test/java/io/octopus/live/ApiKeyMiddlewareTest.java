package io.octopus.live;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.net.ServerSocket;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class ApiKeyMiddlewareTest {
    private static final List<String> OLD_BUILT_IN_KEYS = List.of("default-api-key", "secret-key", "katz-secret-key-12345");
    private final HttpClient http = HttpClient.newHttpClient();

    @TempDir
    Path tmp;

    // Only the blueprint's keys pass, in the configured header; with none configured every request is refused.
    @Test
    public void onlyConfiguredKeysPass() throws Exception {
        Map<String, String> cases = new java.util.LinkedHashMap<>();
        cases.put("    config:\n      valid_keys: [partner-key-1]\n      header_name: X-Partner-Key\n", "partner-key-1");
        cases.put("    valid_keys: [partner-key-1]\n    header_name: X-Partner-Key\n", "partner-key-1");
        cases.put("", null);

        int n = 0;
        for (Map.Entry<String, String> c : cases.entrySet()) {
            Path dir = Files.createDirectory(tmp.resolve("case" + n++));
            Files.writeString(dir.resolve("keyed.yaml"),
                "workflow_name: keyed\nversion: \"1.0.0\"\nmiddlewares:\n  - name: ApiKeyAuthMiddleware\n" + c.getKey()
                    + "    excludes: [/health]\nsteps:\n  - id: ping\n    activity: Ping\n");
            LiveEngine engine = new LiveEngine();
            engine.registerActivity("Ping", in -> new HashMap<>(Map.of("pong", true)));
            engine.loadBlueprintsFromDirectory(dir.toString());
            int port;
            try (ServerSocket s = new ServerSocket(0)) { port = s.getLocalPort(); }
            engine.startServer(port);
            try {
                String url = "http://localhost:" + port + "/api/v1.0.0/keyed/execute";
                String accepted = c.getValue();
                String header = accepted != null ? "X-Partner-Key" : "X-API-Key";
                for (String old : OLD_BUILT_IN_KEYS) {
                    assertEquals(401, post(url, header, old), header + " accepted the old built-in key " + old);
                }
                assertEquals(401, post(url, null, null));
                if (accepted != null) {
                    assertEquals(200, post(url, "X-Partner-Key", accepted));
                    assertEquals(401, post(url, "X-API-Key", accepted));
                }
                assertEquals(200, http.send(HttpRequest.newBuilder(URI.create("http://localhost:" + port + "/health")).build(),
                    HttpResponse.BodyHandlers.discarding()).statusCode());
            } finally {
                engine.stopServer();
            }
        }
    }

    private int post(String url, String header, String key) throws Exception {
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(url))
            .header("Content-Type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString("{\"amount\": 1}"));
        if (header != null) b.header(header, key);
        return http.send(b.build(), HttpResponse.BodyHandlers.discarding()).statusCode();
    }
}
