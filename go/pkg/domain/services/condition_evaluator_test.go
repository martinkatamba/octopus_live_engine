package services

import (
	"testing"
	"time"
)

func rule(r string, data map[string]interface{}) bool {
	return NewConditionEvaluator().evaluateRuleString(r, data)
}

func TestRuleExpressions(t *testing.T) {
	d := map[string]interface{}{"is_admin": false, "level": 7, "tier": "VIP", "promo": nil}
	cases := map[string]bool{
		"is_admin == true or level >= 5":                 true,
		"is_admin == true and level >= 5":                false,
		"not is_admin and (level > 10 or tier == 'VIP')": true,
		`level != 3 && tier == "VIP"`:                    true,
		"promo != null":                                  false,
		"missing == null":                                true,
		"is_admin":                                       false,
		"level < 7.5":                                    true,
		"level == 7 or level == 1 and tier == 'X'":       true, // and binds tighter than or
	}
	for r, want := range cases {
		if got := rule(r, d); got != want {
			t.Errorf("%q: got %v, want %v", r, got, want)
		}
	}
}

func TestInvalidRulesAreErrors(t *testing.T) {
	for _, bad := range []string{"", "amount >>> 5", "a = 1", "(a == 1", "a ==", "a == 1 orr b", "'open", "a == 1 and"} {
		if _, err := parseRule(bad); err == nil {
			t.Errorf("%q should not parse", bad)
		}
	}
}

func TestPathsWithIndexes(t *testing.T) {
	d := map[string]interface{}{"items": []interface{}{map[string]interface{}{"sku": "A-1"}}}
	e := NewConditionEvaluator()
	if got := e.getNestedField(d, "items[0].sku"); got != "A-1" {
		t.Errorf("items[0].sku: got %v", got)
	}
	if got := e.getNestedField(d, "items[3].sku"); got != nil {
		t.Errorf("items[3].sku: got %v, want nil", got)
	}
	if !rule("items[0].sku == 'A-1'", d) {
		t.Error("rule on an indexed path should hold")
	}
}

func TestTimestampsAndCalendarOperators(t *testing.T) {
	want := time.Date(2026, 10, 26, 12, 0, 0, 0, time.UTC)
	for _, s := range []string{"2026-10-26T12:00:00Z", "2026-10-26T14:00:00.123+02:00"} {
		got, ok := parseTimestamp(s)
		if !ok || !got.Truncate(time.Second).Equal(want) {
			t.Errorf("%q: got %v %v", s, got, ok)
		}
	}
	if _, ok := parseTimestamp("26/10/2026"); ok {
		t.Error("26/10/2026 should not parse")
	}
	monday, _ := parseTimestamp("2026-10-26")
	saturday, _ := parseTimestamp("2026-10-24")
	if !isBusinessDay(nil, nil, monday) || isBusinessDay(nil, nil, saturday) || !isBusinessDay(nil, false, saturday) {
		t.Error("is_business_day")
	}
	if !olderThanDays("2026-10-16", 10, monday) || olderThanDays("2026-10-16", 11, monday) {
		t.Error("older_than_days")
	}
}

func TestCollectionAndNullOperators(t *testing.T) {
	e := NewConditionEvaluator()
	checks := []struct {
		actual   interface{}
		op       string
		expected interface{}
		want     bool
	}{
		{[]interface{}{"new", "vip"}, "contains", "vip", true},
		{"FR", "in", "DE, FR", true},
		{"", "is_null", nil, true},
		{"yes", "is_true", nil, true},
		{"10", "greater_than", 9, true},
		{1, "no_such_operator", 1, false},
	}
	for _, c := range checks {
		if got := e.compareValues(c.actual, c.op, c.expected); got != c.want {
			t.Errorf("%v %s %v: got %v, want %v", c.actual, c.op, c.expected, got, c.want)
		}
	}
}
