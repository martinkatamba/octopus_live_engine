use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// A JSON object. Cloning shares the entries; the first write to a shared
/// map copies its top level (`Arc::make_mut`), so traces and snapshots can
/// hold a job's data without copying all of it. Reads and writes use the
/// `BTreeMap` API through `Deref`/`DerefMut`.
#[derive(Clone, Default)]
pub struct ValueMap(Arc<BTreeMap<String, Value>>);

impl ValueMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries, without copying when this is the only reference.
    pub fn into_inner(self) -> BTreeMap<String, Value> {
        Arc::unwrap_or_clone(self.0)
    }

    /// Whether both maps share the same entries.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    /// Whether another map shares these entries. A host converting values
    /// can convert a shared map once.
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }

    /// The address of the entries, the same for every map that shares them.
    pub fn as_ptr(&self) -> *const () {
        Arc::as_ptr(&self.0).cast()
    }
}

impl Deref for ValueMap {
    type Target = BTreeMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ValueMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl PartialEq for ValueMap {
    fn eq(&self, other: &Self) -> bool {
        Self::ptr_eq(self, other) || self.0 == other.0
    }
}

impl fmt::Debug for ValueMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<BTreeMap<String, Value>> for ValueMap {
    fn from(m: BTreeMap<String, Value>) -> Self {
        Self(Arc::new(m))
    }
}

impl From<ValueMap> for BTreeMap<String, Value> {
    fn from(m: ValueMap) -> Self {
        m.into_inner()
    }
}

impl FromIterator<(String, Value)> for ValueMap {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

impl Extend<(String, Value)> for ValueMap {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        (**self).extend(iter)
    }
}

impl IntoIterator for ValueMap {
    type Item = (String, Value);
    type IntoIter = std::collections::btree_map::IntoIter<String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_inner().into_iter()
    }
}

impl<'a> IntoIterator for &'a ValueMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut ValueMap {
    type Item = (&'a String, &'a mut Value);
    type IntoIter = std::collections::btree_map::IterMut<'a, String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        (**self).iter_mut()
    }
}

impl<const N: usize> From<[(String, Value); N]> for ValueMap {
    fn from(arr: [(String, Value); N]) -> Self {
        arr.into_iter().collect()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(ValueMap),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::String(s) => {
                if s.eq_ignore_ascii_case("true") || s == "1" {
                    Some(true)
                } else if s.eq_ignore_ascii_case("false") || s == "0" {
                    Some(false)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            Value::String(s) => s
                .trim()
                .parse::<i64>()
                .ok()
                .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    pub fn is_numeric_value(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(arr) => Some(arr),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&ValueMap> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut ValueMap> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn to_string_lossy(&self) -> String {
        self.to_display_string()
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Value::Null => "<nil>".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => {
                if f.fract() == 0.0 {
                    format!("{}", *f as i64)
                } else {
                    format!("{}", f)
                }
            }
            Value::String(s) => s.clone(),
            Value::Array(_) | Value::Object(_) => self.to_json_string(),
        }
    }

    pub fn to_json_string(&self) -> String {
        let mut buf = String::new();
        write_json_value(self, &mut buf);
        buf
    }

    pub fn parse_json(input: &str) -> Result<Value, String> {
        let mut parser = JsonParser::new(input);
        let val = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos < parser.chars.len() {
            return Err(format!(
                "Unexpected trailing characters at position {}",
                parser.pos
            ));
        }
        Ok(val)
    }
}

pub fn parse(input: &str) -> Result<Value, String> {
    Value::parse_json(input)
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_json_string())
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v as i64)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::Int(v as i64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<BTreeMap<String, T>> for Value {
    fn from(v: BTreeMap<String, T>) -> Self {
        let mut map = ValueMap::new();
        for (k, val) in v {
            map.insert(k, val.into());
        }
        Value::Object(map)
    }
}

impl<T: Into<Value>> From<HashMap<String, T>> for Value {
    fn from(v: HashMap<String, T>) -> Self {
        let mut map = ValueMap::new();
        for (k, val) in v {
            map.insert(k, val.into());
        }
        Value::Object(map)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => Value::Null,
        }
    }
}

pub fn map_to_json(map: &ValueMap) -> String {
    Value::Object(map.clone()).to_json_string()
}

pub fn parse_json_object(input: &str) -> Result<ValueMap, String> {
    match Value::parse_json(input)? {
        Value::Object(map) => Ok(map),
        _ => Err("Expected JSON object at root".to_string()),
    }
}

fn write_json_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                out.push_str("null");
            } else {
                let s = format!("{}", f);
                out.push_str(&s);
            }
        }
        Value::String(s) => write_json_string(s, out),
        Value::Array(arr) => {
            out.push('[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_json_value(val, out);
            }
            out.push('}');
        }
    }
}

pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct JsonParser {
    chars: Vec<char>,
    pos: usize,
}

impl JsonParser {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_whitespace();
        let ch = self.peek().ok_or_else(|| "Unexpected EOF".to_string())?;
        match ch {
            '{' => self.parse_object(),
            '[' => self.parse_array(),
            '"' => self.parse_string().map(Value::String),
            't' | 'f' => self.parse_bool(),
            'n' => self.parse_null(),
            '-' | '0'..='9' => self.parse_number(),
            other => Err(format!(
                "Unexpected character '{}' at position {}",
                other, self.pos
            )),
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.next(); // '{'
        self.skip_whitespace();
        let mut map = ValueMap::new();
        if self.peek() == Some('}') {
            self.next();
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some('"') {
                return Err(format!(
                    "Expected string key in object at position {}",
                    self.pos
                ));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.next() != Some(':') {
                return Err(format!(
                    "Expected ':' after object key at position {}",
                    self.pos
                ));
            }
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_whitespace();
            match self.next() {
                Some(',') => continue,
                Some('}') => break,
                other => {
                    return Err(format!(
                        "Expected ',' or '}}' in object at position {}, got {:?}",
                        self.pos, other
                    ))
                }
            }
        }
        Ok(Value::Object(map))
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.next(); // '['
        self.skip_whitespace();
        let mut arr = Vec::new();
        if self.peek() == Some(']') {
            self.next();
            return Ok(Value::Array(arr));
        }
        loop {
            let val = self.parse_value()?;
            arr.push(val);
            self.skip_whitespace();
            match self.next() {
                Some(',') => continue,
                Some(']') => break,
                other => {
                    return Err(format!(
                        "Expected ',' or ']' in array at position {}, got {:?}",
                        self.pos, other
                    ))
                }
            }
        }
        Ok(Value::Array(arr))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.next() != Some('"') {
            return Err("Expected '\"'".to_string());
        }
        let mut s = String::new();
        while let Some(ch) = self.next() {
            match ch {
                '"' => return Ok(s),
                '\\' => {
                    let esc = self
                        .next()
                        .ok_or_else(|| "Unexpected EOF in escape".to_string())?;
                    match esc {
                        '"' => s.push('"'),
                        '\\' => s.push('\\'),
                        '/' => s.push('/'),
                        'b' => s.push('\x08'),
                        'f' => s.push('\x0c'),
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'u' => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                let hc = self.next().ok_or_else(|| {
                                    "Unexpected EOF in unicode escape".to_string()
                                })?;
                                hex.push(hc);
                            }
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|_| format!("Invalid unicode escape \\u{}", hex))?;
                            if let Some(c) = char::from_u32(code) {
                                s.push(c);
                            }
                        }
                        other => return Err(format!("Invalid escape sequence \\{}", other)),
                    }
                }
                c if (c as u32) < 0x20 => {
                    return Err("Unescaped control character in string".to_string());
                }
                c => s.push(c),
            }
        }
        Err("Unterminated string literal".to_string())
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.chars[self.pos..].starts_with(&['t', 'r', 'u', 'e']) {
            self.pos += 4;
            Ok(Value::Bool(true))
        } else if self.chars[self.pos..].starts_with(&['f', 'a', 'l', 's', 'e']) {
            self.pos += 5;
            Ok(Value::Bool(false))
        } else {
            Err(format!("Invalid boolean literal at position {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        if self.chars[self.pos..].starts_with(&['n', 'u', 'l', 'l']) {
            self.pos += 4;
            Ok(Value::Null)
        } else {
            Err(format!("Invalid null literal at position {}", self.pos))
        }
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.next();
        }
        let mut is_float = false;
        let mut has_digits = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                has_digits = true;
                self.next();
            } else if ch == '.' || ch == 'e' || ch == 'E' || ch == '+' || ch == '-' {
                if ch == '.' || ch == 'e' || ch == 'E' {
                    is_float = true;
                }
                self.next();
            } else {
                break;
            }
        }
        if !has_digits {
            return Err(format!("Invalid number at position {}", start));
        }
        let num_str: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            let f = num_str
                .parse::<f64>()
                .map_err(|e| format!("Invalid float '{}': {}", num_str, e))?;
            Ok(Value::Float(f))
        } else if let Ok(i) = num_str.parse::<i64>() {
            Ok(Value::Int(i))
        } else {
            let f = num_str
                .parse::<f64>()
                .map_err(|e| format!("Invalid number '{}': {}", num_str, e))?;
            Ok(Value::Float(f))
        }
    }
}

#[macro_export]
macro_rules! vmap {
    () => {
        $crate::json::ValueMap::new()
    };
    ($($k:expr => $v:expr),+ $(,)?) => {{
        let mut m = $crate::json::ValueMap::new();
        $(
            m.insert(($k).to_string(), $crate::json::Value::from($v));
        )+
        m
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cloned_map_is_independent_of_writes() {
        let mut a = crate::vmap! { "items" => Value::Object(crate::vmap! { "n" => 1i64 }) };
        let b = a.clone();
        assert!(ValueMap::ptr_eq(&a, &b));

        a.insert("extra".to_string(), Value::Bool(true));
        if let Some(Value::Object(inner)) = a.get_mut("items") {
            inner.insert("n".to_string(), Value::Int(2));
        }

        assert!(!ValueMap::ptr_eq(&a, &b));
        assert_eq!(b.len(), 1);
        assert_eq!(b["items"].get("n"), Some(&Value::Int(1)));
        assert_eq!(a["items"].get("n"), Some(&Value::Int(2)));
    }
}
