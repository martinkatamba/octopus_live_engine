package io.octopus.live.domain.services;

import io.octopus.live.domain.models.SingleCondition;
import io.octopus.live.domain.models.StepCondition;
import io.octopus.live.domain.models.WorkflowStep;
import java.util.Map;
import java.util.Objects;

public class ConditionEvaluator {

    public boolean evaluateStep(WorkflowStep step, Map<String, Object> context) {
        if (step == null) return true;

        if (step.getConditions() != null && step.getConditions().getConditions() != null && !step.getConditions().getConditions().isEmpty()) {
            boolean matchAny = "any".equalsIgnoreCase(step.getConditions().getMatch());
            for (SingleCondition cond : step.getConditions().getConditions()) {
                String f = cond.getVariable() != null && !cond.getVariable().isBlank() ? cond.getVariable() : cond.getField();
                Object val = f != null ? getNestedField(context, f) : null;
                boolean res = compareValues(val, cond.getOperator() != null ? cond.getOperator() : "==", cond.getValue());
                if (matchAny && res) return true;
                if (!matchAny && !res) return false;
            }
            return !matchAny;
        }

        if (step.getCondition() != null) {
            return evaluate(step.getCondition(), context);
        }

        return true;
    }

    public boolean evaluate(StepCondition condition, Map<String, Object> context) {
        if (condition == null) return true;

        if (condition.getRule() != null && !condition.getRule().trim().isEmpty()) {
            return evaluateRuleString(condition.getRule(), context);
        }

        String field = condition.getVariable() != null && !condition.getVariable().isBlank() ? condition.getVariable() : condition.getField();
        if (field != null && condition.getOperator() != null) {
            Object fieldVal = getNestedField(context, field);
            return compareValues(fieldVal, condition.getOperator(), condition.getValue());
        }

        return true;
    }

    private boolean evaluateRuleString(String rule, Map<String, Object> context) {
        String[] partsEq = rule.split("==");
        if (partsEq.length == 2) {
            Object fieldVal = getNestedField(context, partsEq[0].trim());
            String expected = partsEq[1].trim().replaceAll("^\"|\"$|^'|'$", "");
            return Objects.equals(String.valueOf(fieldVal), expected);
        }

        String[] partsNeq = rule.split("!=");
        if (partsNeq.length == 2) {
            Object fieldVal = getNestedField(context, partsNeq[0].trim());
            String expected = partsNeq[1].trim().replaceAll("^\"|\"$|^'|'$", "");
            return !Objects.equals(String.valueOf(fieldVal), expected);
        }

        return true;
    }

    @SuppressWarnings("unchecked")
    public Object getNestedField(Map<String, Object> data, String fieldPath) {
        if (data == null || fieldPath == null || fieldPath.isBlank()) return null;

        // Try direct lookup
        Object val = resolvePath(data, fieldPath);
        if (val != null) return val;

        // Fallback: strip "payload." or "data." prefix
        if (fieldPath.startsWith("payload.")) {
            val = resolvePath(data, fieldPath.substring(8));
            if (val != null) return val;
        } else if (fieldPath.startsWith("data.")) {
            val = resolvePath(data, fieldPath.substring(5));
            if (val != null) return val;
        }

        // Check if root has "payload" or "data" map
        if (data.containsKey("payload") && data.get("payload") instanceof Map<?, ?> pMap) {
            val = resolvePath((Map<String, Object>) pMap, fieldPath);
            if (val != null) return val;
        }
        if (data.containsKey("data") && data.get("data") instanceof Map<?, ?> dMap) {
            val = resolvePath((Map<String, Object>) dMap, fieldPath);
            if (val != null) return val;
        }

        return null;
    }

    @SuppressWarnings("unchecked")
    private Object resolvePath(Map<String, Object> map, String path) {
        if (map == null || path == null) return null;
        if (map.containsKey(path)) return map.get(path);

        String[] parts = path.split("\\.");
        Object current = map;
        for (String part : parts) {
            if (current instanceof Map<?, ?> curMap) {
                current = ((Map<String, Object>) curMap).get(part);
            } else {
                return null;
            }
        }
        return current;
    }

    private boolean compareValues(Object actual, String op, Object expected) {
        if (actual == null) {
            return expected == null && ("==".equalsIgnoreCase(op) || "eq".equalsIgnoreCase(op) || "equals".equalsIgnoreCase(op));
        }

        Double actualNum = toDouble(actual);
        Double expectedNum = toDouble(expected);
        if (actualNum != null && expectedNum != null) {
            switch (op.toLowerCase()) {
                case "==":
                case "eq":
                case "equals":
                case "equal":
                    return Math.abs(actualNum - expectedNum) < 0.0000001;
                case "!=":
                case "ne":
                case "not_equals":
                case "not_equal":
                    return Math.abs(actualNum - expectedNum) >= 0.0000001;
                case ">":
                case "gt":
                    return actualNum > expectedNum;
                case ">=":
                case "gte":
                    return actualNum >= expectedNum;
                case "<":
                case "lt":
                    return actualNum < expectedNum;
                case "<=":
                case "lte":
                    return actualNum <= expectedNum;
                default:
                    return false;
            }
        }

        // Handle booleans
        if (actual instanceof Boolean actualBool) {
            if (expected instanceof Boolean expBool) {
                switch (op.toLowerCase()) {
                    case "==":
                    case "eq":
                    case "equals":
                    case "equal":
                        return actualBool.equals(expBool);
                    case "!=":
                    case "ne":
                    case "not_equals":
                    case "not_equal":
                        return !actualBool.equals(expBool);
                    default:
                        return false;
                }
            }
            if (expected != null) {
                boolean parsedBool = Boolean.parseBoolean(expected.toString());
                switch (op.toLowerCase()) {
                    case "==":
                    case "eq":
                    case "equals":
                    case "equal":
                        return actualBool.equals(parsedBool);
                    case "!=":
                    case "ne":
                    case "not_equals":
                    case "not_equal":
                        return !actualBool.equals(parsedBool);
                    default:
                        return false;
                }
            }
        }

        switch (op.toLowerCase()) {
            case "==":
            case "eq":
            case "equals":
            case "equal":
                return Objects.equals(String.valueOf(actual), String.valueOf(expected));
            case "!=":
            case "ne":
            case "not_equals":
            case "not_equal":
                return !Objects.equals(String.valueOf(actual), String.valueOf(expected));
            case "contains":
                return String.valueOf(actual).contains(String.valueOf(expected));
            case "starts_with":
                return String.valueOf(actual).startsWith(String.valueOf(expected));
            case "ends_with":
                return String.valueOf(actual).endsWith(String.valueOf(expected));
            case "matches":
            case "regex":
            case "regex_match":
            case "pattern":
            case "=~":
                try {
                    return java.util.regex.Pattern.compile(String.valueOf(expected)).matcher(String.valueOf(actual)).find();
                } catch (Exception e) {
                    return false;
                }
            case "not_matches":
            case "not_regex":
            case "!~":
                try {
                    return !java.util.regex.Pattern.compile(String.valueOf(expected)).matcher(String.valueOf(actual)).find();
                } catch (Exception e) {
                    return true;
                }
            default:
                return false;
        }
    }

    private Double toDouble(Object val) {
        if (val instanceof Number) {
            return ((Number) val).doubleValue();
        }
        if (val != null) {
            try {
                return Double.parseDouble(val.toString());
            } catch (NumberFormatException ignored) {}
        }
        return null;
    }
}
