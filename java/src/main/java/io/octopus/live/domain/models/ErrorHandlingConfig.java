package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;

@JsonIgnoreProperties(ignoreUnknown = true)
public class ErrorHandlingConfig {
    @JsonProperty("global_handler")
    private GlobalExceptionHandlerConfig globalHandler;

    public GlobalExceptionHandlerConfig getGlobalHandler() { return globalHandler; }
    public void setGlobalHandler(GlobalExceptionHandlerConfig globalHandler) { this.globalHandler = globalHandler; }
}
