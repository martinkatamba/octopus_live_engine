"""Condition evaluation for live blueprints.

Live and Temporal blueprints share one condition language (decision D3 in
core/conformance/decisions.md): ``condition`` with field/variable, operator
and value; ``conditions`` groups with ``match: all | any``; and the Temporal
engines' operator list, whose semantics are ported from
octopus_workflow_engine_go/pkg/octopus/domain/strategies/operators.go.
``rule:`` strings are expressions with and, or, not, parentheses and
comparisons (D2); a rule that does not parse is refused at load.
"""

import json
import re
from dataclasses import dataclass
from datetime import datetime, timezone
from functools import lru_cache
from typing import Any, Dict, List, Optional, Tuple

from ..models.models import StepCondition, WorkflowBlueprint, WorkflowStep


class RuleSyntaxError(ValueError):
    """A ``rule:`` expression that does not parse."""


class ConditionEvaluator:
    """Evaluates branching rules and expressions in in-memory execution."""

    @classmethod
    def evaluate(cls, condition: Optional[StepCondition], data: Dict[str, Any]) -> bool:
        if not condition:
            return True
        return cls.evaluate_detailed(condition, data)["matched"]

    @classmethod
    def evaluate_detailed(cls, condition: Optional[StepCondition], data: Dict[str, Any]) -> Dict[str, Any]:
        """The verdict plus what was compared, for the step's condition trace."""
        if not condition:
            return {"matched": True, "evaluated_parameters": {}, "rules": [], "outcome": "TRUE (Default Match)"}

        details: List[Dict[str, Any]] = []
        is_any = (condition.match or condition.rule_type or "all").lower() in ("any", "or", "one_of")

        if condition.rules:
            results = []
            for r in condition.rules:
                if not isinstance(r, dict):
                    continue
                if r.get("rule"):
                    results.append(_eval_rule(r["rule"], data, details))
                    continue
                path = r.get("field") or r.get("variable") or r.get("var")
                if path:
                    results.append(_compare_leaf(path, r.get("operator") or r.get("op") or "==", r.get("value", r.get("val")), data, details))
            matched = (any(results) if is_any else all(results)) if results else True
            policy = "ANY (OR)" if is_any else "ALL (AND)"
        elif condition.rule:
            matched = _eval_rule(condition.rule, data, details)
            policy = "RULE"
        elif condition.field and condition.operator:
            matched = _compare_leaf(condition.field, condition.operator, condition.value, data, details)
            policy = "SINGLE"
        else:
            return {"matched": True, "evaluated_parameters": {}, "rules": [], "outcome": "TRUE (Default Match)"}

        return {
            "matched": matched,
            "match_policy": policy,
            "evaluated_parameters": {d["field"]: d["actual_value"] for d in details if d.get("field")},
            "rules": details,
            "outcome": "TRUE (Match)" if matched else "FALSE (Else)",
        }

    @classmethod
    def evaluate_expression(cls, expr: str, data: Dict[str, Any]) -> bool:
        """Evaluates a ``rule:`` expression; one that does not parse is false."""
        return _eval_rule(expr, data, [])

    @classmethod
    def compare(cls, actual: Any, op: str, expected: Any) -> bool:
        """Applies a named operator. An unknown operator is false."""
        return compare_at(actual, op, expected, datetime.now(timezone.utc))

    @classmethod
    def _get_nested_value(cls, data: Dict[str, Any], path: str) -> Any:
        return get_nested(data, path)


def validate_blueprint_conditions(bp: WorkflowBlueprint) -> None:
    """Checks every ``rule:`` in a blueprint, including branches and declared
    routes, so a rule that does not parse is refused when the blueprint loads.
    Raises RuleSyntaxError."""

    def rules_of(cond: StepCondition) -> List[str]:
        out = [cond.rule] if cond.rule else []
        out += [r["rule"] for r in cond.rules if isinstance(r, dict) and r.get("rule")]
        return out

    def walk(steps: List[WorkflowStep]) -> None:
        for step in steps:
            if step.condition:
                for rule in rules_of(step.condition):
                    _, err = _parsed_rule(rule)
                    if err:
                        raise RuleSyntaxError(f"blueprint '{bp.workflow_name}', step '{step.id}': invalid rule {rule!r}: {err}")
                walk(step.condition.then_steps)
                walk(step.condition.else_steps)

    walk(bp.steps)
    for ep in bp.api:
        walk(ep.steps)


def _compare_leaf(path: str, op: str, expected: Any, data: Dict[str, Any], details: List[Dict[str, Any]]) -> bool:
    actual = get_nested(data, path)
    matched = ConditionEvaluator.compare(actual, op, expected)
    details.append({"field": path, "actual_value": actual, "operator": op, "target_value": expected, "matched": matched})
    return matched


def _eval_rule(rule: str, data: Dict[str, Any], details: List[Dict[str, Any]]) -> bool:
    # Blueprints are validated at load, so an unparsable rule only reaches
    # here from direct callers; it is false.
    expr, err = _parsed_rule(rule)
    if err:
        details.append({"field": None, "actual_value": None, "operator": "rule", "target_value": rule, "matched": False, "error": err})
        return False
    return _eval_expr(expr, data, details)


def get_nested(data: Dict[str, Any], path: str) -> Any:
    """Reads a path such as ``applicant.credit_score`` or ``items[0].sku``."""
    cur: Any = data
    for part in path.split("."):
        split = _split_indexes(part)
        if split is None or not isinstance(cur, dict):
            return None
        name, indexes = split
        cur = cur.get(name)
        for idx in indexes:
            if not isinstance(cur, list) or idx >= len(cur):
                return None
            cur = cur[idx]
    return cur


def _split_indexes(part: str) -> Optional[Tuple[str, List[int]]]:
    open_at = part.find("[")
    if open_at < 0:
        return part, []
    indexes = []
    for chunk in part[open_at:].split("[")[1:]:
        if not chunk.endswith("]"):
            return None
        inner = chunk[:-1].strip()
        if not inner.isdigit():
            return None
        indexes.append(int(inner))
    return part[:open_at], indexes


# ---- operators ---------------------------------------------------------------

def _lossy(v: Any) -> str:
    """A value's display text: numbers without a trailing .0, None as <nil>,
    booleans as true/false, lists and objects as JSON."""
    if v is None:
        return "<nil>"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, float) and v.is_integer():
        return str(int(v))
    if isinstance(v, (list, dict)):
        return json.dumps(v, sort_keys=True, separators=(",", ":"), default=str)
    return str(v)


def _number(v: Any) -> Optional[float]:
    if isinstance(v, bool):
        return None
    if isinstance(v, (int, float)):
        return float(v)
    if isinstance(v, str):
        try:
            return float(v.strip())
        except ValueError:
            return None
    return None


def _values_equal(actual: Any, expected: Any) -> bool:
    if actual is None:
        return expected is None
    if expected is None:
        return False
    x, y = _number(actual), _number(expected)
    if x is not None and y is not None:
        return x == y
    # Same type only: in Python True == 1, which the other engines don't do.
    return (type(actual) is type(expected) and actual == expected) or _lossy(actual) == _lossy(expected)


def _ordering(actual: Any, expected: Any) -> Optional[int]:
    """-1, 0 or 1; None when either side is missing (or NaN)."""
    if actual is None or expected is None:
        return None
    x, y = _number(actual), _number(expected)
    if x is not None and y is not None:
        if x < y:
            return -1
        if x > y:
            return 1
        return 0 if x == y else None
    a, b = _lossy(actual), _lossy(expected)
    return (a > b) - (a < b)


def _is_in(actual: Any, expected: Any) -> bool:
    if expected is None:
        return False
    if isinstance(expected, list):
        return any(_values_equal(actual, item) for item in expected)
    if isinstance(expected, str) and "," in expected:
        a = _lossy(actual) if actual is not None else ""
        return any(p.strip() == a for p in expected.split(","))
    return _values_equal(actual, expected)


def _contains(actual: Any, expected: Any) -> bool:
    if actual is None or expected is None:
        return False
    if isinstance(actual, list):
        return any(_values_equal(item, expected) for item in actual)
    return _lossy(expected) in _lossy(actual)


def _is_empty(actual: Any) -> bool:
    return actual is None or actual == ""


def _truthiness(actual: Any) -> Optional[bool]:
    if actual is None:
        return None
    if isinstance(actual, bool):
        return actual
    s = _lossy(actual).lower()
    if s in ("true", "1", "yes"):
        return True
    if s in ("false", "0", "no"):
        return False
    return None


@lru_cache(maxsize=256)
def _compiled(pattern: str) -> Optional["re.Pattern[str]"]:
    try:
        return re.compile(pattern)
    except re.error:
        return None


def _parse_timestamp(s: str) -> Optional[datetime]:
    """YYYY-MM-DD or RFC 3339 (fractional seconds allowed); naive means UTC."""
    try:
        t = datetime.fromisoformat(s.strip())
    except ValueError:
        return None
    return t if t.tzinfo else t.replace(tzinfo=timezone.utc)


def _is_business_day(actual: Any, expected: Any, now: datetime) -> bool:
    t = now
    if actual is not None:
        s = _lossy(actual)
        if s not in ("", "now", "today"):
            parsed = _parse_timestamp(s)
            if parsed is None:
                return False
            t = parsed
    is_weekday = t.astimezone(timezone.utc).weekday() < 5
    want = expected is None or _truthiness(expected) is not False
    return is_weekday == want


def _older_than_days(actual: Any, expected: Any, now: datetime) -> bool:
    days = _number(expected)
    if actual is None or days is None:
        return False
    t = _parse_timestamp(_lossy(actual))
    return t is not None and (now - t).total_seconds() / 86400 >= days


def _regex_match(pattern: Any, text: Any) -> Optional[bool]:
    compiled = _compiled(_lossy(pattern))
    return None if compiled is None else compiled.search(_lossy(text)) is not None


def compare_at(actual: Any, op: str, expected: Any, now: datetime) -> bool:
    op = (op or "").strip().lower()
    if op in ("equals", "eq", "==", "equal"):
        return _values_equal(actual, expected)
    if op in ("not_equals", "ne", "!=", "neq", "not_equal"):
        return not _values_equal(actual, expected)
    if op in ("greater_than", "gt", ">"):
        c = _ordering(actual, expected)
        return c is not None and c > 0
    if op in ("greater_than_or_equal", "greater_or_equal", "gte", ">="):
        c = _ordering(actual, expected)
        return c is not None and c >= 0
    if op in ("less_than", "lt", "<"):
        c = _ordering(actual, expected)
        return c is not None and c < 0
    if op in ("less_than_or_equal", "less_or_equal", "lte", "<="):
        c = _ordering(actual, expected)
        return c is not None and c <= 0
    if op == "in":
        return _is_in(actual, expected)
    if op == "not_in":
        return not _is_in(actual, expected)
    if op == "contains":
        return _contains(actual, expected)
    if op == "starts_with":
        return actual is not None and _lossy(actual).startswith(_lossy(expected))
    if op == "ends_with":
        return actual is not None and _lossy(actual).endswith(_lossy(expected))
    if op in ("is_null", "is_none", "null", "none"):
        return _is_empty(actual)
    if op in ("is_not_null", "is_not_none", "not_null", "not_none", "exists"):
        return not _is_empty(actual)
    if op in ("is_true", "true"):
        return _truthiness(actual) is True
    if op in ("is_false", "false"):
        return _truthiness(actual) is False
    if op in ("regex_match", "regex", "matches_regex", "matches", "pattern", "=~"):
        return actual is not None and _regex_match(expected, actual) is True
    if op in ("not_matches", "not_regex", "!~"):
        return actual is None or _regex_match(expected, actual) is not True
    if op in ("is_business_day", "is_weekday"):
        return _is_business_day(actual, expected, now)
    if op in ("older_than_days", "days_older_than", "age_greater_than_days"):
        return _older_than_days(actual, expected, now)
    return False


# ---- rule expressions ----------------------------------------------------------

@dataclass(frozen=True)
class _Operand:
    path: Optional[str] = None  # a field path, or a literal when None
    literal: Any = None


@dataclass(frozen=True)
class _Expr:
    kind: str  # or, and, not, compare, truthy
    left: Optional["_Expr"] = None
    right: Optional["_Expr"] = None
    op: str = ""
    a: Optional[_Operand] = None
    b: Optional[_Operand] = None


def _eval_expr(e: _Expr, data: Dict[str, Any], details: List[Dict[str, Any]]) -> bool:
    if e.kind == "or":
        return _eval_expr(e.left, data, details) or _eval_expr(e.right, data, details)
    if e.kind == "and":
        return _eval_expr(e.left, data, details) and _eval_expr(e.right, data, details)
    if e.kind == "not":
        return not _eval_expr(e.left, data, details)
    left = _operand_value(e.a, data)
    if e.kind == "compare":
        right = _operand_value(e.b, data)
        matched = ConditionEvaluator.compare(left, e.op, right)
        details.append({"field": e.a.path, "actual_value": left, "operator": e.op, "target_value": right, "matched": matched})
        return matched
    matched = _truthiness(left) is True
    details.append({"field": e.a.path, "actual_value": left, "operator": "truthy", "target_value": True, "matched": matched})
    return matched


def _operand_value(o: _Operand, data: Dict[str, Any]) -> Any:
    return get_nested(data, o.path) if o.path is not None else o.literal


@lru_cache(maxsize=1024)
def _parsed_rule(rule: str) -> Tuple[Optional[_Expr], Optional[str]]:
    """The parsed rule, shared by every evaluation of the same rule text, or
    the reason it does not parse."""
    try:
        return _parse_rule(rule), None
    except RuleSyntaxError as err:
        return None, str(err)


def _parse_rule(rule: str) -> _Expr:
    tokens = _tokenize(rule)
    if not tokens:
        raise RuleSyntaxError("empty rule")
    p = _Parser(tokens)
    expr = p.or_expr()
    if p.pos < len(tokens):
        raise RuleSyntaxError(f"unexpected {tokens[p.pos][1]!r}")
    return expr


_TWO_CHAR_OPS = {"==": "==", "!=": "!=", "<=": "<=", ">=": ">=", "&&": "and", "||": "or"}
_ONE_CHAR_OPS = {"<": "<", ">": ">", "!": "not"}


def _tokenize(src: str) -> List[Tuple[str, Any]]:
    """Tokens are (kind, value): ident, str, num, op, lparen, rparen."""
    out: List[Tuple[str, Any]] = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c.isspace():
            i += 1
        elif c == "(":
            out.append(("lparen", "("))
            i += 1
        elif c == ")":
            out.append(("rparen", ")"))
            i += 1
        elif c in "\"'":
            buf, i = [], i + 1
            while True:
                if i >= n:
                    raise RuleSyntaxError("unterminated string")
                if src[i] == "\\" and i + 1 < n:
                    buf.append(src[i + 1])
                    i += 2
                    continue
                if src[i] == c:
                    i += 1
                    break
                buf.append(src[i])
                i += 1
            out.append(("str", "".join(buf)))
        elif c.isdigit() or (c == "-" and i + 1 < n and src[i + 1].isdigit()):
            start, i = i, i + 1
            while i < n and (src[i].isdigit() or src[i] == "."):
                i += 1
            text = src[start:i]
            try:
                out.append(("num", float(text) if "." in text else int(text)))
            except ValueError:
                raise RuleSyntaxError(f"bad number {text!r}") from None
        elif c.isalpha() or c == "_":
            start = i
            while i < n and (src[i].isalnum() or src[i] in "_.[]"):
                i += 1
            out.append(("ident", src[start:i]))
        elif src[i:i + 2] in _TWO_CHAR_OPS:
            out.append(("op", _TWO_CHAR_OPS[src[i:i + 2]]))
            i += 2
        elif c in _ONE_CHAR_OPS:
            out.append(("op", _ONE_CHAR_OPS[c]))
            i += 1
        else:
            raise RuleSyntaxError(f"unexpected character {c!r}")
    return out


class _Parser:
    def __init__(self, tokens: List[Tuple[str, Any]]):
        self.tokens = tokens
        self.pos = 0

    def _peek(self) -> Optional[Tuple[str, Any]]:
        return self.tokens[self.pos] if self.pos < len(self.tokens) else None

    def _keyword(self, word: str) -> bool:
        t = self._peek()
        return t is not None and ((t[0] == "ident" and t[1].lower() == word) or (t[0] == "op" and t[1] == word))

    def or_expr(self) -> _Expr:
        left = self.and_expr()
        while self._keyword("or"):
            self.pos += 1
            left = _Expr("or", left=left, right=self.and_expr())
        return left

    def and_expr(self) -> _Expr:
        left = self.not_expr()
        while self._keyword("and"):
            self.pos += 1
            left = _Expr("and", left=left, right=self.not_expr())
        return left

    def not_expr(self) -> _Expr:
        if self._keyword("not"):
            self.pos += 1
            return _Expr("not", left=self.not_expr())
        return self.primary()

    def primary(self) -> _Expr:
        t = self._peek()
        if t is not None and t[0] == "lparen":
            self.pos += 1
            e = self.or_expr()
            t = self._peek()
            if t is None or t[0] != "rparen":
                raise RuleSyntaxError("missing ')'")
            self.pos += 1
            return e
        left = self.operand()
        t = self._peek()
        if t is None or t[0] != "op" or t[1] not in ("==", "!=", "<", "<=", ">", ">="):
            return _Expr("truthy", a=left)
        self.pos += 1
        return _Expr("compare", op=t[1], a=left, b=self.operand())

    def operand(self) -> _Operand:
        t = self._peek()
        if t is None:
            raise RuleSyntaxError("expected a value, found the end of the rule")
        kind, value = t
        if kind in ("str", "num"):
            o = _Operand(literal=value)
        elif kind == "ident":
            word = value.lower()
            if word == "true":
                o = _Operand(literal=True)
            elif word == "false":
                o = _Operand(literal=False)
            elif word in ("null", "none", "nil"):
                o = _Operand(literal=None)
            elif word in ("and", "or", "not"):
                raise RuleSyntaxError(f"expected a value, found {value!r}")
            elif all(seg and _split_indexes(seg) is not None for seg in value.split(".")):
                o = _Operand(path=value)
            else:
                raise RuleSyntaxError(f"bad field path {value!r}")
        else:
            raise RuleSyntaxError(f"expected a value, found {value!r}")
        self.pos += 1
        return o
