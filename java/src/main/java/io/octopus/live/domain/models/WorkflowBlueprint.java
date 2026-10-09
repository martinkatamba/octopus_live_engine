package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

@JsonIgnoreProperties(ignoreUnknown = true)
public class WorkflowBlueprint {
    @JsonProperty("workflow_name")
    private String workflowName;
    private String version = "1.0.0";
    private String namespace = "default";
    private String description = "";
    private String mode = "sync";
    @JsonProperty("compensation_mode")
    private String compensationMode = "sync";
    @JsonProperty("allowed_initiator_roles")
    private List<String> allowedInitiatorRoles = new ArrayList<>();
    private List<MiddlewareConfig> middlewares = new ArrayList<>();
    private List<WorkflowStep> steps = new ArrayList<>();
    private List<ApiEndpointConfig> api = new ArrayList<>();
    private Map<String, Object> persistence = new HashMap<>();
    private Map<String, Object> metadata = new HashMap<>();
    private MaskingConfig masking;
    private TelemetryConfig telemetry;
    private MaskingConfig redaction;
    @JsonProperty("error_handling")
    private ErrorHandlingConfig errorHandling;
    @JsonProperty("global_exception_handler")
    private GlobalExceptionHandlerConfig globalExceptionHandler;

    public MaskingConfig getMasking() { return masking; }
    public void setMasking(MaskingConfig masking) { this.masking = masking; }
    public TelemetryConfig getTelemetry() { return telemetry; }
    public void setTelemetry(TelemetryConfig telemetry) { this.telemetry = telemetry; }
    public MaskingConfig getRedaction() { return redaction; }
    public void setRedaction(MaskingConfig redaction) { this.redaction = redaction; }
    public ErrorHandlingConfig getErrorHandling() { return errorHandling; }
    public void setErrorHandling(ErrorHandlingConfig errorHandling) { this.errorHandling = errorHandling; }
    public GlobalExceptionHandlerConfig getGlobalExceptionHandler() {
        if (globalExceptionHandler != null) return globalExceptionHandler;
        return errorHandling != null ? errorHandling.getGlobalHandler() : null;
    }
    public void setGlobalExceptionHandler(GlobalExceptionHandlerConfig globalExceptionHandler) { this.globalExceptionHandler = globalExceptionHandler; }

    public String getWorkflowName() { return workflowName; }
    public void setWorkflowName(String workflowName) { this.workflowName = workflowName; }
    public String getVersion() { return version; }
    public void setVersion(String version) { this.version = version; }
    public String getNamespace() { return namespace; }
    public void setNamespace(String namespace) { this.namespace = namespace; }
    public String getDescription() { return description; }
    public void setDescription(String description) { this.description = description; }
    public String getMode() { return mode; }
    public void setMode(String mode) { this.mode = mode; }
    public String getCompensationMode() { return compensationMode; }
    public void setCompensationMode(String compensationMode) { this.compensationMode = compensationMode; }
    public Map<String, Object> getPersistence() { return persistence; }
    public void setPersistence(Map<String, Object> persistence) { this.persistence = persistence != null ? persistence : new HashMap<>(); }
    public List<String> getAllowedInitiatorRoles() { return allowedInitiatorRoles; }
    public void setAllowedInitiatorRoles(List<String> allowedInitiatorRoles) { this.allowedInitiatorRoles = allowedInitiatorRoles; }
    public List<MiddlewareConfig> getMiddlewares() { return middlewares; }
    public void setMiddlewares(List<MiddlewareConfig> middlewares) { this.middlewares = middlewares != null ? middlewares : new ArrayList<>(); }
    public List<WorkflowStep> getSteps() { return steps; }
    public void setSteps(List<WorkflowStep> steps) { this.steps = steps; }
    public List<ApiEndpointConfig> getApi() { return api; }
    
    @com.fasterxml.jackson.annotation.JsonSetter("api")
    public void setApiNode(com.fasterxml.jackson.databind.JsonNode node) {
        if (node == null || node.isNull()) {
            this.api = new ArrayList<>();
            return;
        }
        com.fasterxml.jackson.databind.ObjectMapper mapper = new com.fasterxml.jackson.databind.ObjectMapper()
                .configure(com.fasterxml.jackson.databind.DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
        List<ApiEndpointConfig> list = new ArrayList<>();
        if (node.isArray()) {
            for (com.fasterxml.jackson.databind.JsonNode item : node) {
                try {
                    ApiEndpointConfig ep = mapper.treeToValue(item, ApiEndpointConfig.class);
                    if (ep != null) list.add(ep);
                } catch (Exception ignored) {}
            }
        } else if (node.isObject()) {
            com.fasterxml.jackson.databind.JsonNode epArray = null;
            if (node.has("endpoints") && node.get("endpoints").isArray()) {
                epArray = node.get("endpoints");
            } else if (node.has("routes") && node.get("routes").isArray()) {
                epArray = node.get("routes");
            }

            if (epArray != null) {
                String basePfx = node.has("base_prefix") ? node.get("base_prefix").asText() : (node.has("base_url") ? node.get("base_url").asText() : "/api");
                String apiVer = node.has("version") ? node.get("version").asText() : null;

                for (com.fasterxml.jackson.databind.JsonNode item : epArray) {
                    try {
                        ApiEndpointConfig ep = mapper.treeToValue(item, ApiEndpointConfig.class);
                        if (ep != null) {
                            list.add(ep);
                        }
                    } catch (Exception ignored) {}
                }
            } else {
                try {
                    ApiEndpointConfig ep = mapper.treeToValue(node, ApiEndpointConfig.class);
                    if (ep != null) list.add(ep);
                } catch (Exception ignored) {}
            }
        }
        this.api = list;
    }

    public void setApi(List<ApiEndpointConfig> api) { this.api = api != null ? api : new ArrayList<>(); }

    public void normalizeEndpoints() {
        if (this.api == null || this.api.isEmpty()) return;
        String wfName = this.workflowName != null ? this.workflowName.trim() : "";
        String ver = this.version != null ? this.version.trim() : "1.0.0";
        String verStr = ver.startsWith("v") || ver.startsWith("/") ? ver : "v" + ver;
        String prefix = "/api/" + verStr.replaceAll("^/+|/+$", "") + "/" + wfName.replaceAll("^/+|/+$", "");

        for (ApiEndpointConfig ep : this.api) {
            if (ep.getRoute() != null && !ep.getRoute().isEmpty()) {
                String r = ep.getRoute().trim();
                if (!r.startsWith("/api/") && !r.startsWith(prefix)) {
                    ep.setRoute(prefix + "/" + r.replaceAll("^/+", ""));
                }
            }
        }
    }
    public Map<String, Object> getMetadata() { return metadata; }
    public void setMetadata(Map<String, Object> metadata) { this.metadata = metadata; }

    public static class MaskConfigResult {
        private final List<String> fields;
        private final String maskValue;

        public MaskConfigResult(List<String> fields, String maskValue) {
            this.fields = fields;
            this.maskValue = maskValue;
        }

        public List<String> getFields() { return fields; }
        public String getMaskValue() { return maskValue; }
    }

    public MaskConfigResult getMaskConfig(WorkflowStep step) {
        java.util.Set<String> fieldSet = new java.util.TreeSet<>(String.CASE_INSENSITIVE_ORDER);
        String maskVal = "********";

        // 1. Global blueprint configs
        if (masking != null && masking.isEnabled()) {
            if (masking.getFields() != null) {
                for (String f : masking.getFields()) {
                    if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                }
            }
            if (masking.getMaskValue() != null && !masking.getMaskValue().trim().isEmpty()) {
                maskVal = masking.getMaskValue();
            }
        }

        if (telemetry != null) {
            if (telemetry.getMasking() != null && telemetry.getMasking().isEnabled()) {
                if (telemetry.getMasking().getFields() != null) {
                    for (String f : telemetry.getMasking().getFields()) {
                        if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                    }
                }
                if (telemetry.getMasking().getMaskValue() != null && !telemetry.getMasking().getMaskValue().trim().isEmpty()) {
                    maskVal = telemetry.getMasking().getMaskValue();
                }
            }
            if (telemetry.getMaskFields() != null) {
                for (String f : telemetry.getMaskFields()) {
                    if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                }
            }
        }

        if (redaction != null && redaction.isEnabled()) {
            if (redaction.getFields() != null) {
                for (String f : redaction.getFields()) {
                    if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                }
            }
            if (redaction.getMaskValue() != null && !redaction.getMaskValue().trim().isEmpty()) {
                maskVal = redaction.getMaskValue();
            }
        }

        // 2. Step-level override
        if (step != null) {
            if (step.getMaskFields() != null) {
                for (String f : step.getMaskFields()) {
                    if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                }
            }
            if (step.getMask() != null) {
                for (String f : step.getMask()) {
                    if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                }
            }
            if (step.getMasking() != null && step.getMasking().isEnabled()) {
                if (step.getMasking().getFields() != null) {
                    for (String f : step.getMasking().getFields()) {
                        if (f != null && !f.trim().isEmpty()) fieldSet.add(f.trim());
                    }
                }
                if (step.getMasking().getMaskValue() != null && !step.getMasking().getMaskValue().trim().isEmpty()) {
                    maskVal = step.getMasking().getMaskValue();
                }
            }
        }

        return new MaskConfigResult(new ArrayList<>(fieldSet), maskVal);
    }

    @JsonIgnoreProperties(ignoreUnknown = true)
    public static class ApiEndpointConfig {
        private String route;
        private String method = "POST";
        @JsonProperty("input_model")
        private String inputModel;
        @JsonProperty("output_model")
        private String outputModel;
        @JsonProperty("status_code")
        private int statusCode = 200;
        private String summary;
        private String description;
        private List<String> tags = new ArrayList<>();
        private List<WorkflowStep> steps = new ArrayList<>();

        public String getRoute() { return route; }
        public void setRoute(String route) { this.route = route; }
        public String getMethod() { return method; }
        public void setMethod(String method) { this.method = method; }
        public String getInputModel() { return inputModel; }
        public void setInputModel(String inputModel) { this.inputModel = inputModel; }
        public String getOutputModel() { return outputModel; }
        public void setOutputModel(String outputModel) { this.outputModel = outputModel; }
        public int getStatusCode() { return statusCode; }
        public void setStatusCode(int statusCode) { this.statusCode = statusCode; }
        public String getSummary() { return summary; }
        public void setSummary(String summary) { this.summary = summary; }
        public String getDescription() { return description; }
        public void setDescription(String description) { this.description = description; }
        public List<String> getTags() { return tags; }
        public void setTags(List<String> tags) { this.tags = tags; }
        public List<WorkflowStep> getSteps() { return steps; }
        public void setSteps(List<WorkflowStep> steps) { this.steps = steps; }
    }
}
