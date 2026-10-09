using Octopus.LiveEngine.Domain.Models;

namespace Octopus.LiveEngine.Domain.Services;

public class ConditionEvaluator
{
    public bool EvaluateStep(WorkflowStep? step, Dictionary<string, object> context)
    {
        if (step == null) return true;

        if (step.Conditions != null && step.Conditions.Conditions.Count > 0)
        {
            var matchAny = string.Equals(step.Conditions.Match, "any", StringComparison.OrdinalIgnoreCase);
            foreach (var cond in step.Conditions.Conditions)
            {
                var f = !string.IsNullOrWhiteSpace(cond.Variable) ? cond.Variable : cond.Field;
                var val = !string.IsNullOrWhiteSpace(f) ? GetNestedField(context, f) : null;
                var res = CompareValues(val, cond.Operator ?? "==", cond.Value);
                if (matchAny && res) return true;
                if (!matchAny && !res) return false;
            }
            return !matchAny;
        }

        if (step.Condition != null)
        {
            return Evaluate(step.Condition, context);
        }

        return true;
    }

    public bool Evaluate(StepCondition? condition, Dictionary<string, object> context)
    {
        if (condition == null) return true;

        if (!string.IsNullOrWhiteSpace(condition.Rule))
        {
            return EvaluateRuleString(condition.Rule, context);
        }

        var field = !string.IsNullOrWhiteSpace(condition.Variable) ? condition.Variable : condition.Field;
        if (!string.IsNullOrWhiteSpace(field) && !string.IsNullOrWhiteSpace(condition.Operator))
        {
            var fieldVal = GetNestedField(context, field);
            return CompareValues(fieldVal, condition.Operator, condition.Value);
        }

        return true;
    }

    private bool EvaluateRuleString(string rule, Dictionary<string, object> context)
    {
        var partsEq = rule.Split("==");
        if (partsEq.Length == 2)
        {
            var fieldVal = GetNestedField(context, partsEq[0].Trim());
            var expected = partsEq[1].Trim().Trim('"', '\'');
            return string.Equals(fieldVal?.ToString(), expected, StringComparison.OrdinalIgnoreCase);
        }

        var partsNeq = rule.Split("!=");
        if (partsNeq.Length == 2)
        {
            var fieldVal = GetNestedField(context, partsNeq[0].Trim());
            var expected = partsNeq[1].Trim().Trim('"', '\'');
            return !string.Equals(fieldVal?.ToString(), expected, StringComparison.OrdinalIgnoreCase);
        }

        return true;
    }

    public object? GetNestedField(Dictionary<string, object> data, string fieldPath)
    {
        var parts = fieldPath.Split('.');
        object? current = data;
        foreach (var part in parts)
        {
            if (current is Dictionary<string, object> dict && dict.TryGetValue(part, out var nextVal))
            {
                current = nextVal;
            }
            else
            {
                return null;
            }
        }
        return current;
    }

    private bool CompareValues(object? actual, string op, object? expected)
    {
        if (actual == null)
        {
            return expected == null && (op == "==" || string.Equals(op, "eq", StringComparison.OrdinalIgnoreCase) || string.Equals(op, "equals", StringComparison.OrdinalIgnoreCase));
        }

        if (TryToDouble(actual, out var aNum) && TryToDouble(expected, out var eNum))
        {
            return op.ToLowerInvariant() switch
            {
                "==" or "eq" or "equals" or "equal" => Math.Abs(aNum - eNum) < 0.0000001,
                "!=" or "ne" or "not_equals" or "not_equal" => Math.Abs(aNum - eNum) >= 0.0000001,
                ">" or "gt" => aNum > eNum,
                ">=" or "gte" => aNum >= eNum,
                "<" or "lt" => aNum < eNum,
                "<=" or "lte" => aNum <= eNum,
                _ => false
            };
        }

        // Handle booleans
        if (actual is bool actualBool)
        {
            if (expected is bool expBool)
            {
                return op.ToLowerInvariant() switch
                {
                    "==" or "eq" or "equals" or "equal" => actualBool == expBool,
                    "!=" or "ne" or "not_equals" or "not_equal" => actualBool != expBool,
                    _ => false
                };
            }
            if (expected != null && bool.TryParse(expected.ToString(), out var parsedBool))
            {
                return op.ToLowerInvariant() switch
                {
                    "==" or "eq" or "equals" or "equal" => actualBool == parsedBool,
                    "!=" or "ne" or "not_equals" or "not_equal" => actualBool != parsedBool,
                    _ => false
                };
            }
        }

        return op.ToLowerInvariant() switch
        {
            "==" or "eq" or "equals" or "equal" => string.Equals(actual?.ToString(), expected?.ToString(), StringComparison.OrdinalIgnoreCase),
            "!=" or "ne" or "not_equals" or "not_equal" => !string.Equals(actual?.ToString(), expected?.ToString(), StringComparison.OrdinalIgnoreCase),
            "contains" => actual.ToString()?.Contains(expected?.ToString() ?? "", StringComparison.OrdinalIgnoreCase) ?? false,
            "starts_with" => actual.ToString()?.StartsWith(expected?.ToString() ?? "", StringComparison.OrdinalIgnoreCase) ?? false,
            "ends_with" => actual.ToString()?.EndsWith(expected?.ToString() ?? "", StringComparison.OrdinalIgnoreCase) ?? false,
            "matches" or "regex" or "regex_match" or "pattern" or "=~" => System.Text.RegularExpressions.Regex.IsMatch(actual?.ToString() ?? "", expected?.ToString() ?? ""),
            "not_matches" or "not_regex" or "!~" => !System.Text.RegularExpressions.Regex.IsMatch(actual?.ToString() ?? "", expected?.ToString() ?? ""),
            _ => false
        };
    }

    private bool TryToDouble(object? val, out double result)
    {
        if (val is double d) { result = d; return true; }
        if (val is int i) { result = i; return true; }
        if (val is long l) { result = l; return true; }
        if (val is float f) { result = f; return true; }
        if (val is decimal dec) { result = (double)dec; return true; }
        if (val != null && double.TryParse(val.ToString(), out var parsed))
        {
            result = parsed;
            return true;
        }
        result = 0;
        return false;
    }
}
