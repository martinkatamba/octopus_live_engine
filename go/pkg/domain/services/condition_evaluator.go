package services

import (
	"fmt"
	"reflect"
	"regexp"
	"strings"

	"github.com/octopus-workflow/octopus_live_engine/go/pkg/domain/models"
)

type ConditionEvaluator struct{}

func NewConditionEvaluator() *ConditionEvaluator {
	return &ConditionEvaluator{}
}

func (e *ConditionEvaluator) EvaluateStep(step *models.WorkflowStep, context map[string]interface{}) bool {
	if step == nil {
		return true
	}
	if step.Conditions != nil && len(step.Conditions.Conditions) > 0 {
		matchAny := strings.EqualFold(step.Conditions.Match, "any")
		for _, c := range step.Conditions.Conditions {
			f := c.Variable
			if f == "" {
				f = c.Field
			}
			val := e.getNestedField(context, f)
			res := e.compareValues(val, c.Operator, c.Value)
			if matchAny && res {
				return true
			}
			if !matchAny && !res {
				return false
			}
		}
		return !matchAny
	}
	if step.Condition != nil {
		return e.Evaluate(step.Condition, context)
	}
	return true
}

func (e *ConditionEvaluator) Evaluate(condition *models.StepCondition, context map[string]interface{}) bool {
	if condition == nil {
		return true
	}
	if condition.Rule != "" {
		return e.evaluateRuleString(condition.Rule, context)
	}
	field := condition.Field
	if field == "" {
		field = condition.Variable
	}
	if field != "" && condition.Operator != "" {
		fieldVal := e.getNestedField(context, field)
		return e.compareValues(fieldVal, condition.Operator, condition.Value)
	}
	return true
}

func (e *ConditionEvaluator) evaluateRuleString(rule string, context map[string]interface{}) bool {
	parts := strings.Split(rule, "==")
	if len(parts) == 2 {
		fieldVal := e.getNestedField(context, strings.TrimSpace(parts[0]))
		expected := strings.Trim(strings.TrimSpace(parts[1]), "\"'")
		return fmt.Sprintf("%v", fieldVal) == expected
	}
	parts = strings.Split(rule, "!=")
	if len(parts) == 2 {
		fieldVal := e.getNestedField(context, strings.TrimSpace(parts[0]))
		expected := strings.Trim(strings.TrimSpace(parts[1]), "\"'")
		return fmt.Sprintf("%v", fieldVal) != expected
	}
	return true
}

func (e *ConditionEvaluator) getNestedField(data map[string]interface{}, fieldPath string) interface{} {
	parts := strings.Split(fieldPath, ".")
	var current interface{} = data
	for _, part := range parts {
		if m, ok := current.(map[string]interface{}); ok {
			current = m[part]
		} else {
			return nil
		}
	}
	return current
}

func (e *ConditionEvaluator) compareValues(actual interface{}, op string, expected interface{}) bool {
	if actual == nil {
		return expected == nil && (op == "==" || op == "eq" || op == "equals" || op == "equal")
	}

	actualFloat, aOk := toFloat(actual)
	expectedFloat, eOk := toFloat(expected)
	if aOk && eOk {
		switch strings.ToLower(op) {
		case "==", "eq", "equals", "equal":
			return actualFloat == expectedFloat
		case "!=", "ne", "not_equals", "not_equal":
			return actualFloat != expectedFloat
		case ">", "gt":
			return actualFloat > expectedFloat
		case ">=", "gte":
			return actualFloat >= expectedFloat
		case "<", "lt":
			return actualFloat < expectedFloat
		case "<=", "lte":
			return actualFloat <= expectedFloat
		}
	}

	// Handle booleans
	if actualBool, ok := actual.(bool); ok {
		if expectedBool, okExp := expected.(bool); okExp {
			switch strings.ToLower(op) {
			case "==", "eq", "equals", "equal":
				return actualBool == expectedBool
			case "!=", "ne", "not_equals", "not_equal":
				return actualBool != expectedBool
			}
		}
		if expectedStr, okStr := expected.(string); okStr {
			expBool := strings.EqualFold(expectedStr, "true")
			switch strings.ToLower(op) {
			case "==", "eq", "equals", "equal":
				return actualBool == expBool
			case "!=", "ne", "not_equals", "not_equal":
				return actualBool != expBool
			}
		}
	}

	switch strings.ToLower(op) {
	case "==", "eq", "equals", "equal":
		return reflect.DeepEqual(actual, expected) || fmt.Sprintf("%v", actual) == fmt.Sprintf("%v", expected)
	case "!=", "ne", "not_equals", "not_equal":
		return !reflect.DeepEqual(actual, expected) && fmt.Sprintf("%v", actual) != fmt.Sprintf("%v", expected)
	case "contains":
		return strings.Contains(fmt.Sprintf("%v", actual), fmt.Sprintf("%v", expected))
	case "starts_with":
		return strings.HasPrefix(fmt.Sprintf("%v", actual), fmt.Sprintf("%v", expected))
	case "ends_with":
		return strings.HasSuffix(fmt.Sprintf("%v", actual), fmt.Sprintf("%v", expected))
	case "matches", "regex", "regex_match", "pattern", "=~":
		re, err := regexp.Compile(fmt.Sprintf("%v", expected))
		if err != nil {
			return false
		}
		return re.MatchString(fmt.Sprintf("%v", actual))
	case "not_matches", "not_regex", "!~":
		re, err := regexp.Compile(fmt.Sprintf("%v", expected))
		if err != nil {
			return true
		}
		return !re.MatchString(fmt.Sprintf("%v", actual))
	}
	return false
}

func toFloat(val interface{}) (float64, bool) {
	switch v := val.(type) {
	case int:
		return float64(v), true
	case int32:
		return float64(v), true
	case int64:
		return float64(v), true
	case float32:
		return float64(v), true
	case float64:
		return v, true
	default:
		return 0, false
	}
}
