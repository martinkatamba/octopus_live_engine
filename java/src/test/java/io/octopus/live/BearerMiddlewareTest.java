package io.octopus.live;

import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.net.ServerSocket;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.util.Base64;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;

public class BearerMiddlewareTest {
    private static final ObjectMapper JSON = new ObjectMapper();
    private final HttpClient http = HttpClient.newHttpClient();

    @TempDir
    Path tmp;

    private static String hs256(Map<String, Object> claims, String secret, String alg) throws Exception {
        Base64.Encoder enc = Base64.getUrlEncoder().withoutPadding();
        String in = enc.encodeToString(JSON.writeValueAsBytes(Map.of("alg", alg, "typ", "JWT")))
            + "." + enc.encodeToString(JSON.writeValueAsBytes(claims));
        Mac mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(secret.getBytes(StandardCharsets.UTF_8), "HmacSHA256"));
        return in + "." + enc.encodeToString(mac.doFinal(in.getBytes(StandardCharsets.UTF_8)));
    }

    // Tokens are verified with the blueprint's HS256 secret; with no validator and no secret every token is refused.
    @Test
    public void verifiesHs256OrRefuses() throws Exception {
        Map<String, Boolean> cases = new LinkedHashMap<>();
        cases.put("    config:\n      jwt_secret: blueprint-secret-1\n", true);
        cases.put("    jwt_secret: blueprint-secret-1\n", true);
        cases.put("", "blueprint-secret-1".equals(System.getenv("OCTOPUS_JWT_SECRET")));

        int n = 0;
        long later = Instant.now().getEpochSecond() + 600;
        for (Map.Entry<String, Boolean> c : cases.entrySet()) {
            Path dir = Files.createDirectory(tmp.resolve("case" + n++));
            Files.writeString(dir.resolve("jwtwf.yaml"),
                "workflow_name: jwtwf\nversion: \"1.0.0\"\nmiddlewares:\n  - name: BearerAuthAndClaimsMiddleware\n" + c.getKey()
                    + "    excludes: [/health]\nsteps:\n  - id: me\n    activity: Me\n");
            LiveEngine engine = new LiveEngine();
            engine.registerActivity("Me", in -> new HashMap<>(Map.of("ok", true)));
            engine.loadBlueprintsFromDirectory(dir.toString());
            int port;
            try (ServerSocket s = new ServerSocket(0)) { port = s.getLocalPort(); }
            engine.startServer(port);
            try {
                String url = "http://localhost:" + port + "/api/v1.0.0/jwtwf/execute";
                List<String> refused = List.of(
                    "anything-at-all",
                    hs256(Map.of("sub", "mallory", "exp", later), "wrong-secret", "HS256"),
                    hs256(Map.of("sub", "alice", "exp", Instant.now().getEpochSecond() - 60), "blueprint-secret-1", "HS256"),
                    hs256(Map.of("sub", "alice", "exp", later), "blueprint-secret-1", "none"),
                    hs256(Map.of("sub", "alice"), "octopus-live-engine-secret-key-2026", "HS256"));
                for (String tok : refused) {
                    assertEquals(401, post(url, tok), "refused token: " + tok);
                }
                String good = hs256(Map.of("sub", "alice", "exp", later), "blueprint-secret-1", "HS256");
                assertEquals(c.getValue() ? 200 : 401, post(url, good));
                assertEquals(200, http.send(HttpRequest.newBuilder(URI.create("http://localhost:" + port + "/health")).build(),
                    HttpResponse.BodyHandlers.discarding()).statusCode());
            } finally {
                engine.stopServer();
            }
        }
    }

    private int post(String url, String token) throws Exception {
        return http.send(HttpRequest.newBuilder(URI.create(url))
            .header("Content-Type", "application/json")
            .header("Authorization", "Bearer " + token)
            .POST(HttpRequest.BodyPublishers.ofString("{}")).build(), HttpResponse.BodyHandlers.discarding()).statusCode();
    }
}
