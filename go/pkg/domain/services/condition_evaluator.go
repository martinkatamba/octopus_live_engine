// Condition evaluation for live blueprints.
//
// Live and Temporal blueprints share one condition language (decision D3 in
// core/conformance/decisions.md): condition with field/variable, operator
// and value; conditions groups with match: all | any; and the Temporal
// engines' operator list, whose semantics are ported from
// octopus_workflow_engine_go/pkg/octopus/domain/strategies/operators.go.
// rule: strings are expressions with and, or, not, parentheses and
// comparisons (D2); a rule that does not parse is refused at load.
package services

import (
	"encoding/json"
	"fmt"
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/martinkatamba/octopus_live_engine/go/pkg/domain/models"
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

// evaluateRuleString evaluates a rule: expression. Blueprints are validated
// at load, so an unparsable rule only reaches here from direct callers; it
// is false.
func (e *ConditionEvaluator) evaluateRuleString(rule string, context map[string]interface{}) bool {
	expr, err := parsedRule(rule)
	if err != nil {
		return false
	}
	return e.evalExpr(expr, context)
}

// ConditionFields returns every field path the step's condition names, each
// once, with its value in context (nil when missing). It is the input of the
// step's condition trace (decision D20), so it includes fields that a
// short-circuited and/or never reached.
func (e *ConditionEvaluator) ConditionFields(step *models.WorkflowStep, context map[string]interface{}) map[string]interface{} {
	out := map[string]interface{}{}
	add := func(p string) {
		if p != "" {
			out[p] = e.getNestedField(context, p)
		}
	}
	switch {
	case step.Conditions != nil && len(step.Conditions.Conditions) > 0:
		for _, c := range step.Conditions.Conditions {
			if c.Variable != "" {
				add(c.Variable)
			} else {
				add(c.Field)
			}
		}
	case step.Condition != nil && step.Condition.Rule != "":
		if expr, err := parsedRule(step.Condition.Rule); err == nil {
			exprPaths(expr, add)
		}
	case step.Condition != nil && step.Condition.Operator != "":
		if step.Condition.Field != "" {
			add(step.Condition.Field)
		} else {
			add(step.Condition.Variable)
		}
	}
	return out
}

// exprPaths calls add with each field path in a rule.
func exprPaths(expr *ruleExpr, add func(string)) {
	switch expr.kind {
	case exprOr, exprAnd:
		exprPaths(expr.left, add)
		exprPaths(expr.right, add)
	case exprNot:
		exprPaths(expr.left, add)
	default:
		add(expr.a.path)
		add(expr.b.path)
	}
}

// getNestedField reads a path such as applicant.credit_score or items[0].sku.
func (e *ConditionEvaluator) getNestedField(data map[string]interface{}, fieldPath string) interface{} {
	var current interface{} = data
	for _, part := range strings.Split(fieldPath, ".") {
		name, indexes, ok := splitIndexes(part)
		if !ok {
			return nil
		}
		m, isMap := current.(map[string]interface{})
		if !isMap {
			return nil
		}
		current = m[name]
		for _, idx := range indexes {
			list, isList := current.([]interface{})
			if !isList || idx >= len(list) {
				return nil
			}
			current = list[idx]
		}
	}
	return current
}

// compareValues applies a named operator. An unknown operator is false.
func (e *ConditionEvaluator) compareValues(actual interface{}, op string, expected interface{}) bool {
	return compareAt(actual, op, expected, time.Now())
}

func compareAt(actual interface{}, op string, expected interface{}, now time.Time) bool {
	switch strings.ToLower(strings.TrimSpace(op)) {
	case "equals", "eq", "==", "equal":
		return valuesEqual(actual, expected)
	case "not_equals", "ne", "!=", "neq", "not_equal":
		return !valuesEqual(actual, expected)
	case "greater_than", "gt", ">":
		c, ok := ordering(actual, expected)
		return ok && c > 0
	case "greater_than_or_equal", "greater_or_equal", "gte", ">=":
		c, ok := ordering(actual, expected)
		return ok && c >= 0
	case "less_than", "lt", "<":
		c, ok := ordering(actual, expected)
		return ok && c < 0
	case "less_than_or_equal", "less_or_equal", "lte", "<=":
		c, ok := ordering(actual, expected)
		return ok && c <= 0
	case "in":
		return isIn(actual, expected)
	case "not_in":
		return !isIn(actual, expected)
	case "contains":
		return contains(actual, expected)
	case "starts_with":
		return actual != nil && strings.HasPrefix(lossy(actual), lossy(expected))
	case "ends_with":
		return actual != nil && strings.HasSuffix(lossy(actual), lossy(expected))
	case "is_null", "is_none", "null", "none":
		return isEmpty(actual)
	case "is_not_null", "is_not_none", "not_null", "not_none", "exists":
		return !isEmpty(actual)
	case "is_true", "true":
		b, ok := truthiness(actual)
		return ok && b
	case "is_false", "false":
		b, ok := truthiness(actual)
		return ok && !b
	case "regex_match", "regex", "matches_regex", "matches", "pattern", "=~":
		if actual == nil {
			return false
		}
		m, ok := regexMatch(lossy(expected), lossy(actual))
		return ok && m
	case "not_matches", "not_regex", "!~":
		if actual == nil {
			return true
		}
		m, ok := regexMatch(lossy(expected), lossy(actual))
		return !ok || !m
	case "is_business_day", "is_weekday":
		return isBusinessDay(actual, expected, now)
	case "older_than_days", "days_older_than", "age_greater_than_days":
		return olderThanDays(actual, expected, now)
	}
	return false
}

func (e *ConditionEvaluator) evalExpr(expr *ruleExpr, context map[string]interface{}) bool {
	switch expr.kind {
	case exprOr:
		return e.evalExpr(expr.left, context) || e.evalExpr(expr.right, context)
	case exprAnd:
		return e.evalExpr(expr.left, context) && e.evalExpr(expr.right, context)
	case exprNot:
		return !e.evalExpr(expr.left, context)
	case exprCompare:
		return e.compareValues(e.operand(expr.a, context), expr.op, e.operand(expr.b, context))
	default:
		b, ok := truthiness(e.operand(expr.a, context))
		return ok && b
	}
}

func (e *ConditionEvaluator) operand(o ruleOperand, context map[string]interface{}) interface{} {
	if o.path != "" {
		return e.getNestedField(context, o.path)
	}
	return o.literal
}

// ValidateBlueprintConditions checks every rule: in a blueprint, including
// branches and declared routes, so a rule that does not parse is refused
// when the blueprint loads.
func ValidateBlueprintConditions(bp *models.WorkflowBlueprint) error {
	var walk func(steps []models.WorkflowStep) error
	walk = func(steps []models.WorkflowStep) error {
		for i := range steps {
			step := &steps[i]
			if step.Condition != nil && step.Condition.Rule != "" {
				if _, err := parsedRule(step.Condition.Rule); err != nil {
					return fmt.Errorf("blueprint '%s', step '%s': invalid rule %q: %v", bp.WorkflowName, step.ID, step.Condition.Rule, err)
				}
			}
			if err := walk(step.GetThenSteps()); err != nil {
				return err
			}
			if err := walk(step.GetElseSteps()); err != nil {
				return err
			}
		}
		return nil
	}
	if err := walk(bp.Steps); err != nil {
		return err
	}
	for _, ep := range bp.API {
		if err := walk(ep.Steps); err != nil {
			return err
		}
	}
	return nil
}

func splitIndexes(part string) (string, []int, bool) {
	open := strings.IndexByte(part, '[')
	if open < 0 {
		return part, nil, true
	}
	var indexes []int
	for _, chunk := range strings.Split(part[open:], "[")[1:] {
		inner, found := strings.CutSuffix(chunk, "]")
		if !found {
			return "", nil, false
		}
		idx, err := strconv.Atoi(strings.TrimSpace(inner))
		if err != nil || idx < 0 {
			return "", nil, false
		}
		indexes = append(indexes, idx)
	}
	return part[:open], indexes, true
}

// lossy is a value's display text: numbers without a trailing .0, nil as
// <nil>, lists and objects as JSON.
func lossy(v interface{}) string {
	switch x := v.(type) {
	case nil:
		return "<nil>"
	case string:
		return x
	case []interface{}, map[string]interface{}:
		b, err := json.Marshal(x)
		if err == nil {
			return string(b)
		}
	}
	return fmt.Sprintf("%v", v)
}

func number(v interface{}) (float64, bool) {
	switch x := v.(type) {
	case int:
		return float64(x), true
	case int32:
		return float64(x), true
	case int64:
		return float64(x), true
	case uint64:
		return float64(x), true
	case float32:
		return float64(x), true
	case float64:
		return x, true
	case json.Number:
		f, err := x.Float64()
		return f, err == nil
	case string:
		f, err := strconv.ParseFloat(strings.TrimSpace(x), 64)
		return f, err == nil
	}
	return 0, false
}

func valuesEqual(actual, expected interface{}) bool {
	if actual == nil {
		return expected == nil
	}
	if expected == nil {
		return false
	}
	if x, ok := number(actual); ok {
		if y, ok := number(expected); ok {
			return x == y
		}
	}
	return reflect.DeepEqual(actual, expected) || lossy(actual) == lossy(expected)
}

// ordering compares numerically when both sides are numbers, otherwise as
// text. ok is false when either side is missing.
func ordering(actual, expected interface{}) (int, bool) {
	if actual == nil || expected == nil {
		return 0, false
	}
	if x, ok := number(actual); ok {
		if y, ok := number(expected); ok {
			switch {
			case x < y:
				return -1, true
			case x > y:
				return 1, true
			case x == y:
				return 0, true
			}
			return 0, false // NaN
		}
	}
	return strings.Compare(lossy(actual), lossy(expected)), true
}

func isIn(actual, expected interface{}) bool {
	switch x := expected.(type) {
	case nil:
		return false
	case []interface{}:
		for _, item := range x {
			if valuesEqual(actual, item) {
				return true
			}
		}
		return false
	case string:
		if strings.Contains(x, ",") {
			a := ""
			if actual != nil {
				a = lossy(actual)
			}
			for _, p := range strings.Split(x, ",") {
				if strings.TrimSpace(p) == a {
					return true
				}
			}
			return false
		}
	}
	return valuesEqual(actual, expected)
}

func contains(actual, expected interface{}) bool {
	if actual == nil || expected == nil {
		return false
	}
	if list, ok := actual.([]interface{}); ok {
		for _, item := range list {
			if valuesEqual(item, expected) {
				return true
			}
		}
		return false
	}
	return strings.Contains(lossy(actual), lossy(expected))
}

func isEmpty(actual interface{}) bool {
	if actual == nil {
		return true
	}
	s, ok := actual.(string)
	return ok && s == ""
}

func truthiness(actual interface{}) (bool, bool) {
	if actual == nil {
		return false, false
	}
	if b, ok := actual.(bool); ok {
		return b, true
	}
	switch strings.ToLower(lossy(actual)) {
	case "true", "1", "yes":
		return true, true
	case "false", "0", "no":
		return false, true
	}
	return false, false
}

// parseTimestamp reads YYYY-MM-DD or RFC 3339 (fractional seconds allowed).
func parseTimestamp(s string) (time.Time, bool) {
	s = strings.TrimSpace(s)
	for _, layout := range []string{time.RFC3339Nano, "2006-01-02 15:04:05Z07:00", "2006-01-02"} {
		if t, err := time.Parse(layout, s); err == nil {
			return t, true
		}
	}
	return time.Time{}, false
}

func isBusinessDay(actual, expected interface{}, now time.Time) bool {
	t := now
	if actual != nil {
		if s := lossy(actual); s != "" && s != "now" && s != "today" {
			parsed, ok := parseTimestamp(s)
			if !ok {
				return false
			}
			t = parsed
		}
	}
	wd := t.UTC().Weekday()
	isWeekday := wd != time.Saturday && wd != time.Sunday
	want := true
	if expected != nil {
		if b, ok := truthiness(expected); ok && !b {
			want = false
		}
	}
	return isWeekday == want
}

func olderThanDays(actual, expected interface{}, now time.Time) bool {
	days, ok := number(expected)
	if actual == nil || !ok {
		return false
	}
	t, ok := parseTimestamp(lossy(actual))
	return ok && now.Sub(t).Hours()/24 >= days
}

var (
	regexMu    sync.Mutex
	regexCache = map[string]*regexp.Regexp{}
)

// regexMatch caches compiled patterns, since blueprints evaluate the same
// few patterns on every request. ok is false for an invalid pattern.
func regexMatch(pattern, text string) (matched, ok bool) {
	const maxCached = 256
	regexMu.Lock()
	re, hit := regexCache[pattern]
	if !hit {
		if len(regexCache) >= maxCached {
			clear(regexCache)
		}
		re, _ = regexp.Compile(pattern)
		regexCache[pattern] = re
	}
	regexMu.Unlock()
	if re == nil {
		return false, false
	}
	return re.MatchString(text), true
}

// ---- rule expressions ------------------------------------------------------

type exprKind int

const (
	exprOr exprKind = iota
	exprAnd
	exprNot
	exprCompare
	exprTruthy
)

// ruleOperand is a field path, or a literal when path is empty.
type ruleOperand struct {
	path    string
	literal interface{}
}

type ruleExpr struct {
	kind        exprKind
	left, right *ruleExpr
	op          string
	a, b        ruleOperand
}

type tokenKind int

const (
	tokIdent tokenKind = iota
	tokStr
	tokNum
	tokOp
	tokLParen
	tokRParen
)

type ruleToken struct {
	kind tokenKind
	text string
	num  interface{}
}

type parsedEntry struct {
	expr *ruleExpr
	err  error
}

var (
	rulesMu    sync.Mutex
	rulesCache = map[string]parsedEntry{}
)

// parsedRule returns the parsed rule, shared by every evaluation of the same
// rule text.
func parsedRule(rule string) (*ruleExpr, error) {
	const maxCached = 1024
	rulesMu.Lock()
	defer rulesMu.Unlock()
	if hit, ok := rulesCache[rule]; ok {
		return hit.expr, hit.err
	}
	if len(rulesCache) >= maxCached {
		clear(rulesCache)
	}
	expr, err := parseRule(rule)
	rulesCache[rule] = parsedEntry{expr, err}
	return expr, err
}

func parseRule(rule string) (*ruleExpr, error) {
	tokens, err := tokenize(rule)
	if err != nil {
		return nil, err
	}
	if len(tokens) == 0 {
		return nil, fmt.Errorf("empty rule")
	}
	p := &ruleParser{tokens: tokens}
	expr, err := p.orExpr()
	if err != nil {
		return nil, err
	}
	if p.pos < len(p.tokens) {
		return nil, fmt.Errorf("unexpected %q", p.tokens[p.pos].text)
	}
	return expr, nil
}

func tokenize(src string) ([]ruleToken, error) {
	chars := []rune(src)
	var out []ruleToken
	for i := 0; i < len(chars); {
		c := chars[i]
		switch {
		case unicode.IsSpace(c):
			i++
		case c == '(':
			out = append(out, ruleToken{kind: tokLParen, text: "("})
			i++
		case c == ')':
			out = append(out, ruleToken{kind: tokRParen, text: ")"})
			i++
		case c == '"' || c == '\'':
			var sb strings.Builder
			i++
			for {
				if i >= len(chars) {
					return nil, fmt.Errorf("unterminated string")
				}
				if chars[i] == '\\' && i+1 < len(chars) {
					sb.WriteRune(chars[i+1])
					i += 2
					continue
				}
				if chars[i] == c {
					i++
					break
				}
				sb.WriteRune(chars[i])
				i++
			}
			out = append(out, ruleToken{kind: tokStr, text: sb.String()})
		case unicode.IsDigit(c) || (c == '-' && i+1 < len(chars) && unicode.IsDigit(chars[i+1])):
			start := i
			i++
			for i < len(chars) && (unicode.IsDigit(chars[i]) || chars[i] == '.') {
				i++
			}
			text := string(chars[start:i])
			var num interface{}
			if strings.Contains(text, ".") {
				f, err := strconv.ParseFloat(text, 64)
				if err != nil {
					return nil, fmt.Errorf("bad number %q", text)
				}
				num = f
			} else {
				n, err := strconv.ParseInt(text, 10, 64)
				if err != nil {
					return nil, fmt.Errorf("bad number %q", text)
				}
				num = n
			}
			out = append(out, ruleToken{kind: tokNum, text: text, num: num})
		case unicode.IsLetter(c) || c == '_':
			start := i
			for i < len(chars) && (unicode.IsLetter(chars[i]) || unicode.IsDigit(chars[i]) || strings.ContainsRune("_.[]", chars[i])) {
				i++
			}
			out = append(out, ruleToken{kind: tokIdent, text: string(chars[start:i])})
		default:
			two := string(chars[i:min(i+2, len(chars))])
			op, n := "", 2
			switch two {
			case "==", "!=", "<=", ">=":
				op = two
			case "&&":
				op = "and"
			case "||":
				op = "or"
			default:
				n = 1
				switch c {
				case '<', '>':
					op = string(c)
				case '!':
					op = "not"
				default:
					return nil, fmt.Errorf("unexpected character %q", c)
				}
			}
			i += n
			out = append(out, ruleToken{kind: tokOp, text: op})
		}
	}
	return out, nil
}

type ruleParser struct {
	tokens []ruleToken
	pos    int
}

func (p *ruleParser) keyword(word string) bool {
	if p.pos >= len(p.tokens) {
		return false
	}
	t := p.tokens[p.pos]
	return (t.kind == tokIdent && strings.EqualFold(t.text, word)) || (t.kind == tokOp && t.text == word)
}

func (p *ruleParser) orExpr() (*ruleExpr, error) {
	left, err := p.andExpr()
	for err == nil && p.keyword("or") {
		p.pos++
		var right *ruleExpr
		if right, err = p.andExpr(); err == nil {
			left = &ruleExpr{kind: exprOr, left: left, right: right}
		}
	}
	return left, err
}

func (p *ruleParser) andExpr() (*ruleExpr, error) {
	left, err := p.notExpr()
	for err == nil && p.keyword("and") {
		p.pos++
		var right *ruleExpr
		if right, err = p.notExpr(); err == nil {
			left = &ruleExpr{kind: exprAnd, left: left, right: right}
		}
	}
	return left, err
}

func (p *ruleParser) notExpr() (*ruleExpr, error) {
	if p.keyword("not") {
		p.pos++
		inner, err := p.notExpr()
		if err != nil {
			return nil, err
		}
		return &ruleExpr{kind: exprNot, left: inner}, nil
	}
	return p.primary()
}

func (p *ruleParser) primary() (*ruleExpr, error) {
	if p.pos < len(p.tokens) && p.tokens[p.pos].kind == tokLParen {
		p.pos++
		e, err := p.orExpr()
		if err != nil {
			return nil, err
		}
		if p.pos >= len(p.tokens) || p.tokens[p.pos].kind != tokRParen {
			return nil, fmt.Errorf("missing ')'")
		}
		p.pos++
		return e, nil
	}
	left, err := p.operand()
	if err != nil {
		return nil, err
	}
	if p.pos >= len(p.tokens) || p.tokens[p.pos].kind != tokOp {
		return &ruleExpr{kind: exprTruthy, a: left}, nil
	}
	op := p.tokens[p.pos].text
	switch op {
	case "==", "!=", "<", "<=", ">", ">=":
	default:
		return &ruleExpr{kind: exprTruthy, a: left}, nil
	}
	p.pos++
	right, err := p.operand()
	if err != nil {
		return nil, err
	}
	return &ruleExpr{kind: exprCompare, op: op, a: left, b: right}, nil
}

func (p *ruleParser) operand() (ruleOperand, error) {
	if p.pos >= len(p.tokens) {
		return ruleOperand{}, fmt.Errorf("expected a value, found the end of the rule")
	}
	t := p.tokens[p.pos]
	var o ruleOperand
	switch t.kind {
	case tokStr:
		o.literal = t.text
	case tokNum:
		o.literal = t.num
	case tokIdent:
		switch strings.ToLower(t.text) {
		case "true":
			o.literal = true
		case "false":
			o.literal = false
		case "null", "none", "nil":
		case "and", "or", "not":
			return o, fmt.Errorf("expected a value, found %q", t.text)
		default:
			for _, seg := range strings.Split(t.text, ".") {
				if _, _, ok := splitIndexes(seg); seg == "" || !ok {
					return o, fmt.Errorf("bad field path %q", t.text)
				}
			}
			o.path = t.text
		}
	default:
		return o, fmt.Errorf("expected a value, found %q", t.text)
	}
	p.pos++
	return o, nil
}
