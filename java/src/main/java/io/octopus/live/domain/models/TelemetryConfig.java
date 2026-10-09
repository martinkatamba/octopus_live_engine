package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.ArrayList;
import java.util.List;

@JsonIgnoreProperties(ignoreUnknown = true)
public class TelemetryConfig {
    private MaskingConfig masking;
    @JsonProperty("mask_fields")
    private List<String> maskFields = new ArrayList<>();

    public MaskingConfig getMasking() {
        return masking;
    }

    public void setMasking(MaskingConfig masking) {
        this.masking = masking;
    }

    public List<String> getMaskFields() {
        return maskFields;
    }

    public void setMaskFields(List<String> maskFields) {
        this.maskFields = maskFields != null ? maskFields : new ArrayList<>();
    }
}
