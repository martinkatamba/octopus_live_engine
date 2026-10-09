package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import java.util.ArrayList;
import java.util.List;

@JsonIgnoreProperties(ignoreUnknown = true)
public class ConditionsBlock {
    private String match = "all";
    private List<SingleCondition> conditions = new ArrayList<>();

    public String getMatch() { return match; }
    public void setMatch(String match) { this.match = match; }
    public List<SingleCondition> getConditions() { return conditions; }
    public void setConditions(List<SingleCondition> conditions) { this.conditions = conditions; }
}
