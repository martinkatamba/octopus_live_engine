package io.octopus.live.domain.services.middleware;

import com.fasterxml.jackson.databind.ObjectMapper;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.Base64;
import java.util.HashMap;
import java.util.Map;

public class JwtHelper {
    private static final ObjectMapper mapper = new ObjectMapper();
    public static final String DEFAULT_SECRET = "octopus-live-engine-secret-key-2026";

    public static String createToken(Map<String, Object> claims, String secret, int expiresInSeconds) {
        try {
            Map<String, Object> header = new HashMap<>();
            header.put("alg", "HS256");
            header.put("typ", "JWT");

            long now = Instant.now().getEpochSecond();
            Map<String, Object> payload = new HashMap<>(claims != null ? claims : new HashMap<>());
            payload.put("iat", now);
            payload.put("exp", now + expiresInSeconds);

            String headerJson = mapper.writeValueAsString(header);
            String payloadJson = mapper.writeValueAsString(payload);

            String headerB64 = Base64.getUrlEncoder().withoutPadding().encodeToString(headerJson.getBytes(StandardCharsets.UTF_8));
            String payloadB64 = Base64.getUrlEncoder().withoutPadding().encodeToString(payloadJson.getBytes(StandardCharsets.UTF_8));
            String signingInput = headerB64 + "." + payloadB64;

            String signatureB64 = sign(signingInput, secret != null ? secret : DEFAULT_SECRET);
            return signingInput + "." + signatureB64;
        } catch (Exception e) {
            throw new RuntimeException("Failed to generate JWT: " + e.getMessage(), e);
        }
    }

    @SuppressWarnings("unchecked")
    public static Map<String, Object> decodeAndVerify(String token, String secret) {
        if (token == null || token.isBlank()) {
            throw new UserNotAuthenticatedException("Missing Bearer token");
        }

        String cleanToken = token.trim();
        if (cleanToken.toLowerCase().startsWith("bearer ")) {
            cleanToken = cleanToken.substring(7).trim();
        }

        String[] parts = cleanToken.split("\\.");
        if (parts.length != 3) {
            throw new UserNotAuthenticatedException("Malformed JWT structure (expected 3 parts)");
        }

        String headerB64 = parts[0];
        String payloadB64 = parts[1];
        String signatureB64 = parts[2];

        String signingInput = headerB64 + "." + payloadB64;
        String expectedSig = sign(signingInput, secret != null ? secret : DEFAULT_SECRET);

        byte[] sigBytes = Base64.getUrlDecoder().decode(signatureB64);
        byte[] expBytes = Base64.getUrlDecoder().decode(expectedSig);

        if (!MessageDigest.isEqual(sigBytes, expBytes)) {
            throw new UserNotAuthenticatedException("Invalid JWT signature");
        }

        try {
            byte[] payloadBytes = Base64.getUrlDecoder().decode(payloadB64);
            Map<String, Object> claims = mapper.readValue(payloadBytes, Map.class);

            if (claims.containsKey("exp")) {
                long exp = 0;
                Object expVal = claims.get("exp");
                if (expVal instanceof Number num) {
                    exp = num.longValue();
                } else if (expVal != null) {
                    exp = Long.parseLong(expVal.toString());
                }

                if (exp > 0 && exp < Instant.now().getEpochSecond()) {
                    throw new UserNotAuthenticatedException("JWT token has expired");
                }
            }

            return claims;
        } catch (UserNotAuthenticatedException e) {
            throw e;
        } catch (Exception e) {
            throw new UserNotAuthenticatedException("Invalid JWT payload: " + e.getMessage());
        }
    }

    private static String sign(String data, String secret) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            SecretKeySpec keySpec = new SecretKeySpec(secret.getBytes(StandardCharsets.UTF_8), "HmacSHA256");
            mac.init(keySpec);
            byte[] rawHmac = mac.doFinal(data.getBytes(StandardCharsets.UTF_8));
            return Base64.getUrlEncoder().withoutPadding().encodeToString(rawHmac);
        } catch (Exception e) {
            throw new RuntimeException("HMAC signing failed", e);
        }
    }
}
