package io.octopus.live.domain.services.middleware;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class PayloadValidationMiddleware extends LiveMiddleware {
    private long maxBodySizeBytes = 0;
    private List<String> requiredHeaders = new ArrayList<>();
    private List<String> requiredFields = new ArrayList<>();
    private Map<String, List<String>> requiredFieldsMap = new HashMap<>();

    public PayloadValidationMiddleware() {
        super("PayloadValidationMiddleware");
    }

    public long getMaxBodySizeBytes() { return maxBodySizeBytes; }
    public void setMaxBodySizeBytes(long maxBodySizeBytes) { this.maxBodySizeBytes = maxBodySizeBytes; }

    public List<String> getRequiredHeaders() { return requiredHeaders; }
    public void setRequiredHeaders(List<String> requiredHeaders) {
        this.requiredHeaders = requiredHeaders != null ? requiredHeaders : new ArrayList<>();
    }

    public List<String> getRequiredFields() { return requiredFields; }
    public void setRequiredFields(List<String> requiredFields) {
        this.requiredFields = requiredFields != null ? requiredFields : new ArrayList<>();
    }

    public Map<String, List<String>> getRequiredFieldsMap() { return requiredFieldsMap; }
    public void setRequiredFieldsMap(Map<String, List<String>> requiredFieldsMap) {
        this.requiredFieldsMap = requiredFieldsMap != null ? requiredFieldsMap : new HashMap<>();
    }

    @Override
    public void process(LiveRequestContext ctx) throws OctopusMiddlewareException {
        if (shouldSkip(ctx.getPath())) {
            return;
        }

        // 1. Validate Required Headers
        if (requiredHeaders != null && !requiredHeaders.isEmpty()) {
            Map<String, String> headers = ctx.getHeaders();
            for (String reqHdr : requiredHeaders) {
                if (reqHdr == null || reqHdr.isBlank()) continue;
                String val = headers.get(reqHdr.toLowerCase());
                if (val == null || val.isBlank()) {
                    val = headers.get(reqHdr);
                }
                if (val == null || val.isBlank()) {
                    throw new MissingRequiredFieldException("Missing required header '" + reqHdr + "'");
                }
            }
        }

        // 2. Validate Payload Size
        if (maxBodySizeBytes > 0) {
            byte[] bytes = ctx.getRawBodyBytes();
            if (bytes != null && bytes.length > maxBodySizeBytes) {
                throw new PayloadTooLargeException(
                    "Payload size of " + bytes.length + " bytes exceeds limit of " + maxBodySizeBytes + " bytes",
                    maxBodySizeBytes,
                    bytes.length
                );
            }
        }

        // 3. Validate Required Fields in Request Body
        String method = ctx.getMethod();
        if ("POST".equalsIgnoreCase(method) || "PUT".equalsIgnoreCase(method) || "PATCH".equalsIgnoreCase(method)) {
            List<String> fieldsToValidate = new ArrayList<>(requiredFields);
            if (requiredFieldsMap != null && !requiredFieldsMap.isEmpty()) {
                String path = ctx.getPath();
                if (requiredFieldsMap.containsKey(path)) {
                    fieldsToValidate.addAll(requiredFieldsMap.get(path));
                } else {
                    for (Map.Entry<String, List<String>> entry : requiredFieldsMap.entrySet()) {
                        String pat = entry.getKey();
                        if (path.endsWith(pat) || path.endsWith(pat.replaceAll("^/+", ""))) {
                            fieldsToValidate.addAll(entry.getValue());
                            break;
                        }
                    }
                }
            }

            if (!fieldsToValidate.isEmpty()) {
                Map<String, Object> body = ctx.getBody();
                for (String field : fieldsToValidate) {
                    if (field == null || field.isBlank()) continue;
                    Object val = body != null ? body.get(field) : null;
                    if (val == null || String.valueOf(val).isBlank()) {
                        throw new MissingRequiredFieldException("Missing required field '" + field + "' in request payload.");
                    }
                }
            }
        }
    }
}
