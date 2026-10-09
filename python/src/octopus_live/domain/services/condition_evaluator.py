import operator
import re
from typing import Any, Dict, Optional
from ..models.models import StepCondition


class ConditionEvaluator:
    """Evaluates branching rules and expressions in in-memory execution."""

    OPERATORS = {
        "==": operator.eq,
        "equals": operator.eq,
        "eq": operator.eq,
        "!=": operator.ne,
        "ne": operator.ne,
        "neq": operator.ne,
        ">": operator.gt,
        "gt": operator.gt,
        ">=": operator.ge,
        "gte": operator.ge,
        "<": operator.lt,
        "lt": operator.lt,
        "<=": operator.le,
        "lte": operator.le,
        "contains": lambda a, b: b in a if a is not None else False,
        "in": lambda a, b: a in b if b is not None else False,
        "is_null": lambda a, _: a is None,
        "is_not_null": lambda a, _: a is not None,
        "starts_with": lambda a, b: str(a).startswith(str(b)) if a is not None else False,
        "ends_with": lambda a, b: str(a).endswith(str(b)) if a is not None else False,
        "matches": lambda a, b: bool(re.search(str(b), str(a))) if a is not None and b is not None else False,
        "regex": lambda a, b: bool(re.search(str(b), str(a))) if a is not None and b is not None else False,
        "regex_match": lambda a, b: bool(re.search(str(b), str(a))) if a is not None and b is not None else False,
        "pattern": lambda a, b: bool(re.search(str(b), str(a))) if a is not None and b is not None else False,
        "=~": lambda a, b: bool(re.search(str(b), str(a))) if a is not None and b is not None else False,
        "!~": lambda a, b: not bool(re.search(str(b), str(a))) if a is not None and b is not None else True,
        "not_matches": lambda a, b: not bool(re.search(str(b), str(a))) if a is not None and b is not None else True,
        "not_regex": lambda a, b: not bool(re.search(str(b), str(a))) if a is not None and b is not None else True,
    }

    @classmethod
    def evaluate(cls, condition: Optional[StepCondition], data: Dict[str, Any]) -> bool:
        if not condition:
            return True
        return cls.evaluate_detailed(condition, data)["matched"]

    @classmethod
    def evaluate_detailed(cls, condition: Optional[StepCondition], data: Dict[str, Any]) -> Dict[str, Any]:
        if not condition:
            return {
                "matched": True,
                "evaluated_parameters": {},
                "rules": [],
                "outcome": "TRUE (Default Match)"
            }

        evaluated_params = {}
        rules_eval = []

        match_policy = (condition.match or condition.rule_type or "all").lower()
        is_any_mode = match_policy in ("any", "or", "one_of")

        # 1. Multi-rule conditions array (e.g. `rules:` or `conditions:`)
        if hasattr(condition, "rules") and condition.rules and len(condition.rules) > 0:
            for r in condition.rules:
                if not isinstance(r, dict):
                    continue
                r_field = r.get("field") or r.get("variable") or r.get("var")
                r_op = r.get("operator") or r.get("op") or "=="
                r_val = r.get("value") if "value" in r else r.get("val")
                r_rule = r.get("rule") or r.get("raw")

                if r_rule:
                    res = cls._eval_single_expr(r_rule, data)
                    evaluated_params.update(res["evaluated_params"])
                    rules_eval.append(res["rule_detail"])
                elif r_field:
                    actual_v = cls._get_nested_value(data, r_field)
                    evaluated_params[r_field] = actual_v
                    op_fn = cls.OPERATORS.get(r_op)
                    r_matched = False
                    if op_fn:
                        try:
                            r_matched = bool(op_fn(actual_v, r_val))
                        except Exception:
                            r_matched = False
                    rules_eval.append({
                        "field": r_field,
                        "actual_value": actual_v,
                        "operator": r_op,
                        "target_value": r_val,
                        "matched": r_matched
                    })

            if is_any_mode:
                overall_matched = any(r["matched"] for r in rules_eval) if rules_eval else True
            else:
                overall_matched = all(r["matched"] for r in rules_eval) if rules_eval else True

            return {
                "matched": overall_matched,
                "match_policy": "ANY (OR)" if is_any_mode else "ALL (AND)",
                "evaluated_parameters": evaluated_params,
                "rules": rules_eval,
                "outcome": f"TRUE ({'ANY' if is_any_mode else 'ALL'} Match)" if overall_matched else "FALSE (Else)"
            }

        # 2. String expression rule with compound `and` / `or`
        if condition.rule:
            expr = condition.rule.strip()
            if " and " in expr or " or " in expr:
                is_or = " or " in expr
                delimiter = " or " if is_or else " and "
                parts = expr.split(delimiter)
                for part in parts:
                    res = cls._eval_single_expr(part.strip(), data)
                    evaluated_params.update(res["evaluated_params"])
                    rules_eval.append(res["rule_detail"])

                if is_or:
                    overall_matched = any(r["matched"] for r in rules_eval)
                else:
                    overall_matched = all(r["matched"] for r in rules_eval)

                return {
                    "matched": overall_matched,
                    "match_policy": "ANY (OR)" if is_or else "ALL (AND)",
                    "evaluated_parameters": evaluated_params,
                    "rules": rules_eval,
                    "outcome": f"TRUE ({'ANY' if is_or else 'ALL'} Match)" if overall_matched else "FALSE (Else)"
                }
            else:
                res = cls._eval_single_expr(expr, data)
                evaluated_params.update(res["evaluated_params"])
                rules_eval.append(res["rule_detail"])
                overall_matched = res["rule_detail"]["matched"]
                return {
                    "matched": overall_matched,
                    "match_policy": "SINGLE",
                    "evaluated_parameters": evaluated_params,
                    "rules": rules_eval,
                    "outcome": "TRUE (Match)" if overall_matched else "FALSE (Else)"
                }

        # 3. Single structured condition
        if condition.field:
            field_val = cls._get_nested_value(data, condition.field)
            evaluated_params[condition.field] = field_val
            op_name = condition.operator or "=="
            target_val = condition.value

            op_fn = cls.OPERATORS.get(op_name)
            rule_matched = False
            if op_fn:
                try:
                    rule_matched = bool(op_fn(field_val, target_val))
                except Exception:
                    rule_matched = False

            rules_eval.append({
                "field": condition.field,
                "actual_value": field_val,
                "operator": op_name,
                "target_value": target_val,
                "matched": rule_matched
            })

            return {
                "matched": rule_matched,
                "match_policy": "SINGLE",
                "evaluated_parameters": evaluated_params,
                "rules": rules_eval,
                "outcome": "TRUE (Match)" if rule_matched else "FALSE (Else)"
            }

        return {
            "matched": True,
            "evaluated_parameters": {},
            "rules": [],
            "outcome": "TRUE (Default Match)"
        }

    @classmethod
    def _eval_single_expr(cls, expr: str, data: Dict[str, Any]) -> Dict[str, Any]:
        match = re.match(r"^([\w\.]+)\s*(==|!=|>=|<=|>|<|=~|!~|contains|in|matches|regex|pattern|starts_with|ends_with)\s*(.+)$", expr)
        if match:
            field_path, op_sym, raw_val = match.groups()
            field_val = cls._get_nested_value(data, field_path)
            parsed_val = cls._parse_literal(raw_val.strip())
            op_fn = cls.OPERATORS.get(op_sym)
            rule_matched = False
            if op_fn:
                try:
                    rule_matched = bool(op_fn(field_val, parsed_val))
                except Exception:
                    rule_matched = False
            return {
                "evaluated_params": {field_path: field_val},
                "rule_detail": {
                    "field": field_path,
                    "actual_value": field_val,
                    "operator": op_sym,
                    "target_value": parsed_val,
                    "matched": rule_matched
                }
            }
        else:
            val = cls._get_nested_value(data, expr)
            matched = bool(val)
            return {
                "evaluated_params": {expr: val},
                "rule_detail": {
                    "field": expr,
                    "actual_value": val,
                    "operator": "truthy",
                    "target_value": True,
                    "matched": matched
                }
            }

    @classmethod
    def evaluate_expression(cls, expr: str, data: Dict[str, Any]) -> bool:
        expr = expr.strip()
        # Parse comparison patterns e.g. `amount >= 50000` or `status == 'VIP'`
        match = re.match(r"^([\w\.]+)\s*(==|!=|>=|<=|>|<|contains|in)\s*(.+)$", expr)
        if match:
            field_path, op_sym, raw_val = match.groups()
            field_val = cls._get_nested_value(data, field_path)
            parsed_val = cls._parse_literal(raw_val.strip())
            op_fn = cls.OPERATORS.get(op_sym)
            if op_fn:
                try:
                    return bool(op_fn(field_val, parsed_val))
                except Exception:
                    return False

        # Truthy evaluation of variable
        val = cls._get_nested_value(data, expr)
        return bool(val)

    @classmethod
    def _get_nested_value(cls, data: Dict[str, Any], path: str) -> Any:
        parts = path.split(".")
        cur: Any = data
        for p in parts:
            if isinstance(cur, dict):
                cur = cur.get(p)
            else:
                return None
        return cur

    @classmethod
    def _parse_literal(cls, val_str: str) -> Any:
        if val_str.lower() in ("true", "false"):
            return val_str.lower() == "true"
        if val_str.lower() in ("null", "none"):
            return None
        if (val_str.startswith("'") and val_str.endswith("'")) or (val_str.startswith('"') and val_str.endswith('"')):
            return val_str[1:-1]
        try:
            if "." in val_str:
                return float(val_str)
            return int(val_str)
        except ValueError:
            return val_str
