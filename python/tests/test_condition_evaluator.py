from datetime import datetime, timezone

import pytest

from octopus_live.domain.services.condition_evaluator import (
    ConditionEvaluator,
    RuleSyntaxError,
    _is_business_day,
    _older_than_days,
    _parse_rule,
    _parse_timestamp,
    get_nested,
)


def rule(r, data):
    return ConditionEvaluator.evaluate_expression(r, data)


@pytest.mark.parametrize("expr, want", [
    ("is_admin == true or level >= 5", True),
    ("is_admin == true and level >= 5", False),
    ("not is_admin and (level > 10 or tier == 'VIP')", True),
    ('level != 3 && tier == "VIP"', True),
    ("promo != null", False),
    ("missing == null", True),
    ("is_admin", False),
    ("level < 7.5", True),
    ("level == 7 or level == 1 and tier == 'X'", True),  # and binds tighter than or
])
def test_rule_expressions(expr, want):
    d = {"is_admin": False, "level": 7, "tier": "VIP", "promo": None}
    assert rule(expr, d) is want


@pytest.mark.parametrize("bad", ["", "amount >>> 5", "a = 1", "(a == 1", "a ==", "a == 1 orr b", "'open", "a == 1 and"])
def test_invalid_rules_are_errors(bad):
    with pytest.raises(RuleSyntaxError):
        _parse_rule(bad)


def test_paths_with_indexes():
    d = {"items": [{"sku": "A-1"}]}
    assert get_nested(d, "items[0].sku") == "A-1"
    assert get_nested(d, "items[3].sku") is None
    assert rule("items[0].sku == 'A-1'", d)


def test_timestamps_and_calendar_operators():
    want = datetime(2026, 10, 26, 12, 0, tzinfo=timezone.utc)
    assert _parse_timestamp("2026-10-26T12:00:00Z") == want
    assert _parse_timestamp("2026-10-26T14:00:00.123+02:00").replace(microsecond=0) == want
    assert _parse_timestamp("26/10/2026") is None
    monday, saturday = _parse_timestamp("2026-10-26"), _parse_timestamp("2026-10-24")
    assert _is_business_day(None, None, monday)
    assert not _is_business_day(None, None, saturday)
    assert _is_business_day(None, False, saturday)
    assert _older_than_days("2026-10-16", 10, monday)
    assert not _older_than_days("2026-10-16", 11, monday)


@pytest.mark.parametrize("actual, op, expected, want", [
    (["new", "vip"], "contains", "vip", True),
    ("FR", "in", "DE, FR", True),
    ("", "is_null", None, True),
    ("yes", "is_true", None, True),
    ("10", "greater_than", 9, True),
    (True, "equals", 1, False),  # booleans are not numbers
    (1, "no_such_operator", 1, False),
])
def test_collection_and_null_operators(actual, op, expected, want):
    assert ConditionEvaluator.compare(actual, op, expected) is want
