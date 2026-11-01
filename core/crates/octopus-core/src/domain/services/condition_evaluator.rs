//! Condition evaluation for live blueprints.
//!
//! Live and Temporal blueprints share one condition language (decision D3 in
//! `core/conformance/decisions.md`): `condition` with field/variable,
//! operator and value; `conditions` groups with `match: all | any`; and the
//! Temporal engines' operator list, whose semantics are ported from
//! `octopus_workflow_engine_go/pkg/octopus/domain/strategies/operators.go`.
//! `rule:` strings are expressions with `and`, `or`, `not`, parentheses and
//! comparisons (D2); a rule that does not parse is refused at load.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::models::{StepCondition, WorkflowBlueprint, WorkflowStep};
use crate::json::{Value, ValueMap};

#[derive(Clone, Debug, Default)]
pub struct ConditionEvaluator;

impl ConditionEvaluator {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate_step(&self, step: &WorkflowStep, context: &ValueMap) -> bool {
        if let Some(ref group) = step.conditions {
            if !group.conditions.is_empty() {
                let match_any = group.r#match.eq_ignore_ascii_case("any");
                for c in &group.conditions {
                    let f = if !c.variable.is_empty() {
                        &c.variable
                    } else {
                        &c.field
                    };
                    let val = self.get_nested_field(context, f);
                    let res = self.compare_values(val.as_ref(), &c.operator, &c.value);
                    if match_any && res {
                        return true;
                    }
                    if !match_any && !res {
                        return false;
                    }
                }
                return !match_any;
            }
        }
        if let Some(ref cond) = step.condition {
            return self.evaluate(cond, context);
        }
        true
    }

    pub fn evaluate(&self, condition: &StepCondition, context: &ValueMap) -> bool {
        if !condition.rule.is_empty() {
            return self.evaluate_rule_string(&condition.rule, context);
        }
        let field = if !condition.field.is_empty() {
            &condition.field
        } else {
            &condition.variable
        };
        if !field.is_empty() && !condition.operator.is_empty() {
            let field_val = self.get_nested_field(context, field);
            return self.compare_values(field_val.as_ref(), &condition.operator, &condition.value);
        }
        true
    }

    /// Evaluates a `rule:` expression. Blueprints are validated at load, so
    /// an unparsable rule only reaches here from direct callers; it is false.
    pub fn evaluate_rule_string(&self, rule: &str, context: &ValueMap) -> bool {
        match parsed_rule(rule) {
            Ok(expr) => self.eval_expr(&expr, context),
            Err(_) => false,
        }
    }

    /// Every field path the step's condition names, each once and in order,
    /// with its value in `context` (null when missing). This is the input of
    /// the step's condition trace (decision D20), so it includes fields that
    /// a short-circuited `and`/`or` never reached.
    pub fn condition_fields(&self, step: &WorkflowStep, context: &ValueMap) -> ValueMap {
        let mut paths: Vec<String> = Vec::new();
        let mut add = |p: &str| {
            if !p.is_empty() && !paths.iter().any(|q| q == p) {
                paths.push(p.to_string());
            }
        };
        match (&step.conditions, &step.condition) {
            (Some(group), _) if !group.conditions.is_empty() => {
                for c in &group.conditions {
                    add(if !c.variable.is_empty() {
                        &c.variable
                    } else {
                        &c.field
                    });
                }
            }
            (_, Some(cond)) if !cond.rule.is_empty() => {
                if let Ok(expr) = parsed_rule(&cond.rule) {
                    expr_paths(&expr, &mut add);
                }
            }
            (_, Some(cond)) if !cond.operator.is_empty() => {
                add(if !cond.field.is_empty() {
                    &cond.field
                } else {
                    &cond.variable
                });
            }
            _ => {}
        }
        let mut out = ValueMap::new();
        for p in paths {
            let v = self.get_nested_field(context, &p).unwrap_or(Value::Null);
            out.insert(p, v);
        }
        out
    }

    /// Reads a path such as `applicant.credit_score` or `items[0].sku`.
    pub fn get_nested_field(&self, data: &ValueMap, field_path: &str) -> Option<Value> {
        let mut current: Option<&Value> = None;
        for (i, part) in field_path.split('.').enumerate() {
            let (name, indexes) = split_indexes(part)?;
            let mut v = if i == 0 {
                data.get(name)?
            } else {
                match current? {
                    Value::Object(m) => m.get(name)?,
                    _ => return None,
                }
            };
            for idx in indexes {
                v = v.as_array()?.get(idx)?;
            }
            current = Some(v);
        }
        current.cloned()
    }

    /// Applies a named operator. An unknown operator is false.
    pub fn compare_values(&self, actual: Option<&Value>, op: &str, expected: &Value) -> bool {
        let actual = actual.filter(|v| !v.is_null());
        match op.trim().to_lowercase().as_str() {
            "equals" | "eq" | "==" | "equal" => values_equal(actual, expected),
            "not_equals" | "ne" | "!=" | "neq" | "not_equal" => !values_equal(actual, expected),
            "greater_than" | "gt" | ">" => ordering(actual, expected).is_some_and(|o| o.is_gt()),
            "greater_than_or_equal" | "greater_or_equal" | "gte" | ">=" => {
                ordering(actual, expected).is_some_and(|o| o.is_ge())
            }
            "less_than" | "lt" | "<" => ordering(actual, expected).is_some_and(|o| o.is_lt()),
            "less_than_or_equal" | "less_or_equal" | "lte" | "<=" => {
                ordering(actual, expected).is_some_and(|o| o.is_le())
            }
            "in" => is_in(actual, expected),
            "not_in" => !is_in(actual, expected),
            "contains" => contains(actual, expected),
            "starts_with" => {
                actual.is_some_and(|a| a.to_string_lossy().starts_with(&expected.to_string_lossy()))
            }
            "ends_with" => {
                actual.is_some_and(|a| a.to_string_lossy().ends_with(&expected.to_string_lossy()))
            }
            "is_null" | "is_none" | "null" | "none" => is_empty(actual),
            "is_not_null" | "is_not_none" | "not_null" | "not_none" | "exists" => !is_empty(actual),
            "is_true" | "true" => truthiness(actual) == Some(true),
            "is_false" | "false" => truthiness(actual) == Some(false),
            "regex_match" | "regex" | "matches_regex" | "matches" | "pattern" | "=~" => actual
                .is_some_and(|a| {
                    simple_regex_match(&expected.to_string_lossy(), &a.to_string_lossy())
                        .unwrap_or(false)
                }),
            "not_matches" | "not_regex" | "!~" => match actual {
                Some(a) => simple_regex_match(&expected.to_string_lossy(), &a.to_string_lossy())
                    .map(|m| !m)
                    .unwrap_or(true),
                None => true,
            },
            "is_business_day" | "is_weekday" => is_business_day(actual, expected, now_unix()),
            "older_than_days" | "days_older_than" | "age_greater_than_days" => {
                older_than_days(actual, expected, now_unix())
            }
            _ => false,
        }
    }

    fn eval_expr(&self, expr: &Expr, context: &ValueMap) -> bool {
        match expr {
            Expr::Or(a, b) => self.eval_expr(a, context) || self.eval_expr(b, context),
            Expr::And(a, b) => self.eval_expr(a, context) && self.eval_expr(b, context),
            Expr::Not(a) => !self.eval_expr(a, context),
            Expr::Compare(l, op, r) => {
                let l = self.operand(l, context);
                let r = self.operand(r, context).unwrap_or(Value::Null);
                self.compare_values(l.as_ref(), op, &r)
            }
            Expr::Truthy(o) => truthiness(self.operand(o, context).as_ref()) == Some(true),
        }
    }

    fn operand(&self, o: &Operand, context: &ValueMap) -> Option<Value> {
        match o {
            Operand::Literal(v) => Some(v.clone()),
            Operand::Path(p) => self.get_nested_field(context, p),
        }
    }
}

/// Checks every `rule:` in a blueprint, including branches and declared
/// routes, so a rule that does not parse is refused when the blueprint loads.
pub fn validate_blueprint_conditions(bp: &WorkflowBlueprint) -> Result<(), String> {
    fn walk(steps: &[WorkflowStep], workflow: &str) -> Result<(), String> {
        for step in steps {
            if let Some(ref cond) = step.condition {
                if !cond.rule.is_empty() {
                    parsed_rule(&cond.rule).map_err(|e| {
                        format!(
                            "blueprint '{}', step '{}': invalid rule {:?}: {}",
                            workflow, step.id, cond.rule, e
                        )
                    })?;
                }
            }
            walk(&step.get_then_steps(), workflow)?;
            walk(&step.get_else_steps(), workflow)?;
        }
        Ok(())
    }
    walk(&bp.steps, &bp.workflow_name)?;
    for ep in &bp.api {
        walk(&ep.steps, &bp.workflow_name)?;
    }
    Ok(())
}

fn split_indexes(part: &str) -> Option<(&str, Vec<usize>)> {
    let Some(open) = part.find('[') else {
        return Some((part, Vec::new()));
    };
    let name = &part[..open];
    let mut indexes = Vec::new();
    for chunk in part[open..].split('[').skip(1) {
        indexes.push(chunk.strip_suffix(']')?.trim().parse().ok()?);
    }
    Some((name, indexes))
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Bool(_) | Value::Null | Value::Array(_) | Value::Object(_) => None,
        other => other.as_f64(),
    }
}

fn values_equal(actual: Option<&Value>, expected: &Value) -> bool {
    let Some(a) = actual else {
        return expected.is_null();
    };
    if expected.is_null() {
        return false;
    }
    if let (Some(x), Some(y)) = (number(a), number(expected)) {
        return x == y;
    }
    a == expected || a.to_string_lossy() == expected.to_string_lossy()
}

fn ordering(actual: Option<&Value>, expected: &Value) -> Option<std::cmp::Ordering> {
    let a = actual?;
    if expected.is_null() {
        return None;
    }
    if let (Some(x), Some(y)) = (number(a), number(expected)) {
        return x.partial_cmp(&y);
    }
    Some(a.to_string_lossy().cmp(&expected.to_string_lossy()))
}

fn is_in(actual: Option<&Value>, expected: &Value) -> bool {
    match expected {
        Value::Null => false,
        Value::Array(items) => items.iter().any(|item| values_equal(actual, item)),
        Value::String(s) if s.contains(',') => {
            let a = actual.map(|v| v.to_string_lossy()).unwrap_or_default();
            s.split(',').any(|p| p.trim() == a)
        }
        other => values_equal(actual, other),
    }
}

fn contains(actual: Option<&Value>, expected: &Value) -> bool {
    match actual {
        None => false,
        _ if expected.is_null() => false,
        Some(Value::Array(items)) => items.iter().any(|item| values_equal(Some(item), expected)),
        Some(a) => a.to_string_lossy().contains(&expected.to_string_lossy()),
    }
}

fn is_empty(actual: Option<&Value>) -> bool {
    match actual {
        None => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(_) => false,
    }
}

fn truthiness(actual: Option<&Value>) -> Option<bool> {
    match actual? {
        Value::Bool(b) => Some(*b),
        v => match v.to_string_lossy().to_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds since the epoch for `YYYY-MM-DD` or RFC 3339 (`...T12:00:00Z`,
/// `...+02:00`, fractional seconds allowed).
fn parse_timestamp(s: &str) -> Option<i64> {
    let s = s.trim();
    let date = s.get(..10)?;
    let mut parts = date.split('-');
    let (y, m, d): (i64, i64, i64) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let mut secs = days * 86400;
    let rest = &s[10..];
    if rest.is_empty() {
        return Some(secs);
    }
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let (h, mi, se) = (
        rest.get(0..2)?.parse::<i64>().ok()?,
        rest.get(3..5)?.parse::<i64>().ok()?,
        rest.get(6..8)?.parse::<i64>().ok()?,
    );
    secs += h * 3600 + mi * 60 + se;
    let mut tz = &rest[8..];
    if let Some(frac) = tz.strip_prefix('.') {
        tz = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    match tz {
        "Z" | "z" => Some(secs),
        _ => {
            let sign = match tz.get(0..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let oh = tz.get(1..3)?.parse::<i64>().ok()?;
            let om = tz.get(4..6)?.parse::<i64>().ok()?;
            Some(secs - sign * (oh * 3600 + om * 60))
        }
    }
}

fn is_business_day(actual: Option<&Value>, expected: &Value, now: i64) -> bool {
    let t = match actual.map(|v| v.to_string_lossy()) {
        None => now,
        Some(s) if s.is_empty() || s == "now" || s == "today" => now,
        Some(s) => match parse_timestamp(&s) {
            Some(t) => t,
            None => return false,
        },
    };
    // 1970-01-01 was a Thursday: day 0 -> weekday index 3 (Mon = 0).
    let weekday = (t.div_euclid(86400) + 3).rem_euclid(7);
    let is_weekday = weekday < 5;
    let want = if expected.is_null() {
        true
    } else {
        truthiness(Some(expected)) != Some(false)
    };
    is_weekday == want
}

fn older_than_days(actual: Option<&Value>, expected: &Value, now: i64) -> bool {
    let (Some(a), Some(days)) = (actual, number(expected)) else {
        return false;
    };
    match parse_timestamp(&a.to_string_lossy()) {
        Some(t) => (now - t) as f64 / 86400.0 >= days,
        None => false,
    }
}

// ---- rule expressions ----------------------------------------------------

#[derive(Debug)]
enum Operand {
    Literal(Value),
    Path(String),
}

#[derive(Debug)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Compare(Operand, &'static str, Operand),
    Truthy(Operand),
}

#[derive(Debug, PartialEq)]
enum Token {
    Ident(String),
    Str(String),
    Num(f64, bool),
    Op(&'static str),
    LParen,
    RParen,
}

/// Calls `add` with each field path in a rule, left to right.
fn expr_paths(expr: &Expr, add: &mut dyn FnMut(&str)) {
    let mut operand = |o: &Operand| {
        if let Operand::Path(p) = o {
            add(p);
        }
    };
    match expr {
        Expr::Or(a, b) | Expr::And(a, b) => {
            expr_paths(a, add);
            expr_paths(b, add);
        }
        Expr::Not(a) => expr_paths(a, add),
        Expr::Compare(l, _, r) => {
            operand(l);
            operand(r);
        }
        Expr::Truthy(o) => operand(o),
    }
}

/// Parsed rules, shared by every evaluation of the same rule text.
fn parsed_rule(rule: &str) -> Result<Arc<Expr>, String> {
    const MAX_CACHED: usize = 1024;
    type Cache = Mutex<HashMap<String, Result<Arc<Expr>, String>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(rule) {
        return hit.clone();
    }
    if cache.len() >= MAX_CACHED {
        cache.clear();
    }
    let parsed = parse_rule(rule).map(Arc::new);
    cache.insert(rule.to_string(), parsed.clone());
    parsed
}

fn parse_rule(rule: &str) -> Result<Expr, String> {
    let tokens = tokenize(rule)?;
    if tokens.is_empty() {
        return Err("empty rule".to_string());
    }
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.or_expr()?;
    match p.tokens.get(p.pos) {
        None => Ok(expr),
        Some(t) => Err(format!("unexpected {:?}", t)),
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '(' {
            out.push(Token::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Token::RParen);
            i += 1;
        } else if c == '"' || c == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match chars.get(i) {
                    None => return Err("unterminated string".to_string()),
                    Some('\\') if chars.get(i + 1).is_some() => {
                        s.push(chars[i + 1]);
                        i += 2;
                    }
                    Some(&q) if q == c => {
                        i += 1;
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        i += 1;
                    }
                }
            }
            out.push(Token::Str(s));
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let n: f64 = text.parse().map_err(|_| format!("bad number {:?}", text))?;
            out.push(Token::Num(n, !text.contains('.')));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '[' | ']'))
            {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
        } else {
            let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let (op, len) = match two.as_str() {
                "==" => ("==", 2),
                "!=" => ("!=", 2),
                "<=" => ("<=", 2),
                ">=" => (">=", 2),
                "&&" => ("and", 2),
                "||" => ("or", 2),
                _ => match c {
                    '<' => ("<", 1),
                    '>' => (">", 1),
                    '!' => ("not", 1),
                    _ => return Err(format!("unexpected character {:?}", c)),
                },
            };
            i += len;
            out.push(Token::Op(op));
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn keyword(&self, word: &str) -> bool {
        match self.tokens.get(self.pos) {
            Some(Token::Ident(w)) => w.eq_ignore_ascii_case(word),
            Some(Token::Op(op)) => *op == word,
            _ => false,
        }
    }

    fn or_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.and_expr()?;
        while self.keyword("or") {
            self.pos += 1;
            left = Expr::Or(Box::new(left), Box::new(self.and_expr()?));
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, String> {
        let mut left = self.not_expr()?;
        while self.keyword("and") {
            self.pos += 1;
            left = Expr::And(Box::new(left), Box::new(self.not_expr()?));
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<Expr, String> {
        if self.keyword("not") {
            self.pos += 1;
            return Ok(Expr::Not(Box::new(self.not_expr()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        if self.tokens.get(self.pos) == Some(&Token::LParen) {
            self.pos += 1;
            let e = self.or_expr()?;
            if self.tokens.get(self.pos) != Some(&Token::RParen) {
                return Err("missing ')'".to_string());
            }
            self.pos += 1;
            return Ok(e);
        }
        let left = self.operand()?;
        let op = match self.tokens.get(self.pos) {
            Some(Token::Op(op)) if matches!(*op, "==" | "!=" | "<" | "<=" | ">" | ">=") => *op,
            _ => return Ok(Expr::Truthy(left)),
        };
        self.pos += 1;
        let right = self.operand()?;
        Ok(Expr::Compare(left, op, right))
    }

    fn operand(&mut self) -> Result<Operand, String> {
        let tok = self
            .tokens
            .get(self.pos)
            .ok_or("expected a value, found the end of the rule")?;
        let operand = match tok {
            Token::Str(s) => Operand::Literal(Value::String(s.clone())),
            Token::Num(n, true) => Operand::Literal(Value::Int(*n as i64)),
            Token::Num(n, false) => Operand::Literal(Value::Float(*n)),
            Token::Ident(w) => match w.to_lowercase().as_str() {
                "true" => Operand::Literal(Value::Bool(true)),
                "false" => Operand::Literal(Value::Bool(false)),
                "null" | "none" | "nil" => Operand::Literal(Value::Null),
                "and" | "or" | "not" => return Err(format!("expected a value, found {:?}", w)),
                _ => {
                    if !w
                        .split('.')
                        .all(|p| !p.is_empty() && split_indexes(p).is_some())
                    {
                        return Err(format!("bad field path {:?}", w));
                    }
                    Operand::Path(w.clone())
                }
            },
            other => return Err(format!("expected a value, found {:?}", other)),
        };
        self.pos += 1;
        Ok(operand)
    }
}

/// Matches with RE2 semantics, like Go's `regexp`. Compiled patterns are
/// cached, since blueprints evaluate the same few patterns on every request.
/// An invalid pattern is an `Err`.
fn simple_regex_match(pattern: &str, text: &str) -> Result<bool, ()> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    const MAX_CACHED: usize = 256;
    static CACHE: OnceLock<Mutex<HashMap<String, Option<regex::Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if !cache.contains_key(pattern) {
        if cache.len() >= MAX_CACHED {
            cache.clear();
        }
        cache.insert(pattern.to_string(), regex::Regex::new(pattern).ok());
    }
    match &cache[pattern] {
        Some(re) => Ok(re.is_match(text)),
        None => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmap;

    fn rule(r: &str, data: &ValueMap) -> bool {
        ConditionEvaluator::new().evaluate_rule_string(r, data)
    }

    #[test]
    fn rule_expressions() {
        let d =
            vmap! { "is_admin" => false, "level" => 7i64, "tier" => "VIP", "promo" => Value::Null };
        assert!(rule("is_admin == true or level >= 5", &d));
        assert!(!rule("is_admin == true and level >= 5", &d));
        assert!(rule("not is_admin and (level > 10 or tier == 'VIP')", &d));
        assert!(rule("level != 3 && tier == \"VIP\"", &d));
        assert!(!rule("promo != null", &d));
        assert!(rule("missing == null", &d));
        assert!(!rule("is_admin", &d));
        assert!(rule("level < 7.5", &d));
        // `and` binds tighter than `or`.
        assert!(rule("level == 7 or level == 1 and tier == 'X'", &d));
    }

    #[test]
    fn invalid_rules_are_errors() {
        for bad in [
            "",
            "amount >>> 5",
            "a = 1",
            "(a == 1",
            "a ==",
            "a == 1 orr b",
            "'open",
            "a == 1 and",
        ] {
            assert!(parse_rule(bad).is_err(), "{:?} should not parse", bad);
        }
    }

    #[test]
    fn paths_with_indexes() {
        let d = vmap! { "items" => Value::Array(vec![Value::Object(vmap! { "sku" => "A-1" })]) };
        let e = ConditionEvaluator::new();
        assert_eq!(
            e.get_nested_field(&d, "items[0].sku"),
            Some(Value::String("A-1".into()))
        );
        assert_eq!(e.get_nested_field(&d, "items[3].sku"), None);
        assert!(rule("items[0].sku == 'A-1'", &d));
    }

    #[test]
    fn timestamps_and_calendar_operators() {
        assert_eq!(parse_timestamp("1970-01-02"), Some(86400));
        assert_eq!(parse_timestamp("2026-10-26T12:00:00Z"), Some(1_793_016_000));
        assert_eq!(
            parse_timestamp("2026-10-26T14:00:00.123+02:00"),
            Some(1_793_016_000)
        );
        assert_eq!(parse_timestamp("26/10/2026"), None);
        let monday = parse_timestamp("2026-10-26").unwrap();
        let saturday = parse_timestamp("2026-10-24").unwrap();
        assert!(is_business_day(None, &Value::Null, monday));
        assert!(!is_business_day(None, &Value::Null, saturday));
        assert!(is_business_day(None, &Value::Bool(false), saturday));
        let ten_days_ago = Value::String("2026-10-16".into());
        assert!(older_than_days(
            Some(&ten_days_ago),
            &Value::Int(10),
            monday
        ));
        assert!(!older_than_days(
            Some(&ten_days_ago),
            &Value::Int(11),
            monday
        ));
    }

    #[test]
    fn collection_and_null_operators() {
        let e = ConditionEvaluator::new();
        let tags = Value::Array(vec!["new".into(), "vip".into()]);
        assert!(e.compare_values(Some(&tags), "contains", &Value::String("vip".into())));
        assert!(e.compare_values(Some(&"FR".into()), "in", &Value::String("DE, FR".into())));
        assert!(e.compare_values(Some(&Value::String(String::new())), "is_null", &Value::Null));
        assert!(e.compare_values(Some(&"yes".into()), "is_true", &Value::Null));
        assert!(e.compare_values(Some(&"10".into()), "greater_than", &Value::Int(9)));
        assert!(!e.compare_values(Some(&Value::Int(1)), "no_such_operator", &Value::Int(1)));
    }
}
