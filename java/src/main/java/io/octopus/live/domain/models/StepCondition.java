package io.octopus.live.domain.models;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;
import java.util.ArrayList;
import java.util.List;

@JsonIgnoreProperties(ignoreUnknown = true)
public class StepCondition {
    private String rule;
    private String variable;
    private String operator;
    private String field;
    private Object value;
    @JsonProperty("then_steps")
    private List<WorkflowStep> thenSteps = new ArrayList<>();
    @JsonProperty("else_steps")
    private List<WorkflowStep> elseSteps = new ArrayList<>();

    public String getRule() { return rule; }
    public void setRule(String rule) { this.rule = rule; }
    public String getVariable() { return variable; }
    public void setVariable(String variable) { this.variable = variable; }
    public String getOperator() { return operator; }
    public void setOperator(String operator) { this.operator = operator; }
    public String getField() { return field; }
    public void setField(String field) { this.field = field; }
    public Object getValue() { return value; }
    public void setValue(Object value) { this.value = value; }
    public List<WorkflowStep> getThenSteps() { return thenSteps; }
    public void setThenSteps(List<WorkflowStep> thenSteps) { this.thenSteps = thenSteps; }
    public List<WorkflowStep> getElseSteps() { return elseSteps; }
    public void setElseSteps(List<WorkflowStep> elseSteps) { this.elseSteps = elseSteps; }
}
