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
        if !parser.at_end() {
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
    use std::fmt::Write;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => {
            let _ = write!(out, "{}", i);
        }
        Value::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                out.push_str("null");
            } else {
                let _ = write!(out, "{}", f);
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

/// Writes `s` as a JSON string. Runs of characters that need no escape are
/// copied whole.
pub fn write_json_string(s: &str, out: &mut String) {
    use std::fmt::Write;
    out.push('"');
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            0x08 => "\\b",
            0x0c => "\\f",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => "",
            _ => continue,
        };
        // `b` is ASCII, so `start..i` ends on a character boundary.
        out.push_str(&s[start..i]);
        if esc.is_empty() {
            let _ = write!(out, "\\u{:04x}", b);
        } else {
            out.push_str(esc);
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
    out.push('"');
}

/// A JSON parser over the input's bytes. Positions in its errors are byte
/// offsets. It splits the input only at ASCII bytes, so every slice it takes
/// is valid UTF-8.
struct JsonParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            text: input,
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Skips JSON whitespace, and any other Unicode whitespace as before.
    fn skip_whitespace(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c => self.pos += 1,
                0x80.. => match self.text[self.pos..].chars().next() {
                    Some(c) if c.is_whitespace() => self.pos += c.len_utf8(),
                    _ => break,
                },
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    /// The character at the cursor, for error messages.
    fn char_here(&self) -> char {
        self.text[self.pos..].chars().next().unwrap_or('\0')
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_whitespace();
        let b = self.peek().ok_or_else(|| "Unexpected EOF".to_string())?;
        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(Value::String),
            b't' | b'f' => self.parse_bool(),
            b'n' => self.parse_null(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(format!(
                "Unexpected character '{}' at position {}",
                self.char_here(),
                self.pos
            )),
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.next(); // '{'
        self.skip_whitespace();
        let mut map = BTreeMap::new();
        if self.peek() == Some(b'}') {
            self.next();
            return Ok(Value::Object(ValueMap::from(map)));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(format!(
                    "Expected string key in object at position {}",
                    self.pos
                ));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.next() != Some(b':') {
                return Err(format!(
                    "Expected ':' after object key at position {}",
                    self.pos
                ));
            }
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_whitespace();
            match self.next() {
                Some(b',') => continue,
                Some(b'}') => break,
                other => {
                    return Err(format!(
                        "Expected ',' or '}}' in object at position {}, got {:?}",
                        self.pos,
                        other.map(char::from)
                    ))
                }
            }
        }
        Ok(Value::Object(ValueMap::from(map)))
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.next(); // '['
        self.skip_whitespace();
        let mut arr = Vec::new();
        if self.peek() == Some(b']') {
            self.next();
            return Ok(Value::Array(arr));
        }
        loop {
            let val = self.parse_value()?;
            arr.push(val);
            self.skip_whitespace();
            match self.next() {
                Some(b',') => continue,
                Some(b']') => break,
                other => {
                    return Err(format!(
                        "Expected ',' or ']' in array at position {}, got {:?}",
                        self.pos,
                        other.map(char::from)
                    ))
                }
            }
        }
        Ok(Value::Array(arr))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.next() != Some(b'"') {
            return Err("Expected '\"'".to_string());
        }
        let mut s = String::new();
        let mut first = true;
        loop {
            // Copy the run up to the next quote, backslash or control byte.
            let start = self.pos;
            while let Some(&b) = self.bytes.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            // Most strings have no escapes: allocate them once, at size.
            if first && self.peek() == Some(b'"') {
                self.pos += 1;
                return Ok(self.text[start..self.pos - 1].to_string());
            }
            first = false;
            s.push_str(&self.text[start..self.pos]);
            match self.next() {
                None => return Err("Unterminated string literal".to_string()),
                Some(b'"') => return Ok(s),
                Some(b'\\') => {
                    let esc = self
                        .next()
                        .ok_or_else(|| "Unexpected EOF in escape".to_string())?;
                    match esc {
                        b'"' => s.push('"'),
                        b'\\' => s.push('\\'),
                        b'/' => s.push('/'),
                        b'b' => s.push('\x08'),
                        b'f' => s.push('\x0c'),
                        b'n' => s.push('\n'),
                        b'r' => s.push('\r'),
                        b't' => s.push('\t'),
                        b'u' => s.push(self.parse_unicode_escape()?),
                        _ => {
                            self.pos -= 1;
                            return Err(format!(
                                "Invalid escape sequence \\{}",
                                self.char_here()
                            ));
                        }
                    }
                }
                Some(_) => return Err("Unescaped control character in string".to_string()),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let end = self.pos + 4;
        let hex = self
            .text
            .get(self.pos..end)
            .ok_or_else(|| "Unexpected EOF in unicode escape".to_string())?;
        let code = u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| format!("Invalid unicode escape \\u{}", hex))?;
        self.pos = end;
        Ok(code)
    }

    /// The character after `\u`. A surrogate pair (`😀`) makes one
    /// character; a lone surrogate becomes U+FFFD.
    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let code = self.hex4()?;
        if (0xD800..0xDC00).contains(&code)
            && self.bytes.get(self.pos) == Some(&b'\\')
            && self.bytes.get(self.pos + 1) == Some(&b'u')
        {
            let save = self.pos;
            self.pos += 2;
            let low = self.hex4()?;
            if (0xDC00..0xE000).contains(&low) {
                let c = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                return Ok(char::from_u32(c).unwrap_or('\u{FFFD}'));
            }
            // Not a low surrogate: leave it to be read as its own escape.
            self.pos = save;
        }
        Ok(char::from_u32(code).unwrap_or('\u{FFFD}'))
    }

    fn literal(&mut self, word: &str, v: Value, what: &str) -> Result<Value, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(format!("Invalid {} literal at position {}", what, self.pos))
        }
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.peek() == Some(b't') {
            self.literal("true", Value::Bool(true), "boolean")
        } else {
            self.literal("false", Value::Bool(false), "boolean")
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        self.literal("null", Value::Null, "null")
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut is_float = false;
        let mut has_digits = false;
        while let Some(b) = self.peek() {
            match b {
                b'0'..=b'9' => has_digits = true,
                b'.' | b'e' | b'E' => is_float = true,
                b'+' | b'-' => {}
                _ => break,
            }
            self.pos += 1;
        }
        if !has_digits {
            return Err(format!("Invalid number at position {}", start));
        }
        let num_str = &self.text[start..self.pos];
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

    fn parsed(text: &str) -> Value {
        parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    #[test]
    fn strings_round_trip() {
        for s in [
            "",
            "plain",
            "quote \" backslash \\ slash /",
            "\n\r\t\x08\x0c\x00\x1f\x7f",
            "héllo wörld",
            "日本語",
            "😀 emoji 👍🏽",
            "mixed\"😀\\日\n",
        ] {
            let text = Value::String(s.to_string()).to_json_string();
            assert_eq!(parsed(&text), Value::String(s.to_string()), "{text}");
            let reference: String = serde_json::from_str(&text).unwrap();
            assert_eq!(reference, s);
        }
    }

    #[test]
    fn writes_the_same_escapes_as_before() {
        let text = Value::String("a\"b\\c\n\x01é😀".to_string()).to_json_string();
        assert_eq!(text, "\"a\\\"b\\\\c\\n\\u0001é😀\"");
    }

    #[test]
    fn unicode_escapes_and_surrogate_pairs() {
        // Python's json.dumps and .NET's System.Text.Json escape like this.
        assert_eq!(parsed(r#""\ud83d\ude00""#), Value::from("😀"));
        assert_eq!(parsed(r#""\u00e9\u65e5""#), Value::from("é日"));
        assert_eq!(parsed(r#""\uD83D\uDE00x""#), Value::from("😀x"));
        // Lone surrogates become U+FFFD; what follows is still read.
        assert_eq!(parsed(r#""\ud83d""#), Value::from("\u{FFFD}"));
        assert_eq!(parsed(r#""\ude00a""#), Value::from("\u{FFFD}a"));
        assert_eq!(parsed(r#""\ud83d\u0041""#), Value::from("\u{FFFD}A"));
        assert!(parse(r#""\u12""#).is_err());
        assert!(parse(r#""\u12g4""#).is_err());
        assert!(parse(r#""\u+123""#).is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(parsed("0"), Value::Int(0));
        assert_eq!(parsed("-12"), Value::Int(-12));
        assert_eq!(parsed("9223372036854775807"), Value::Int(i64::MAX));
        assert_eq!(parsed("9223372036854775808"), Value::Float(9223372036854775808.0));
        assert_eq!(parsed("1.5"), Value::Float(1.5));
        assert_eq!(parsed("-2.5e3"), Value::Float(-2500.0));
        assert_eq!(parsed("1E-2"), Value::Float(0.01));
        assert!(parse("-").is_err());
        assert!(parse("1-2").is_err());
        assert_eq!(Value::Float(f64::NAN).to_json_string(), "null");
        assert_eq!(Value::Float(0.1).to_json_string(), "0.1");
        assert_eq!(Value::Int(-7).to_json_string(), "-7");
    }

    #[test]
    fn whitespace_literals_and_errors() {
        let v = parsed(" \t\r\n{ \"a\" : [ true , false , null ] }\u{a0}\n");
        assert_eq!(v.get("a"), Some(&Value::from(vec![Value::Bool(true), Value::Bool(false), Value::Null])));
        for bad in [
            "", "{", "[1,", "{\"a\" 1}", "{a:1}", "[1 2]", "tru", "nul", "\"open",
            "\"bad \\x escape\"", "\"ctrl \x01\"", "{} x", "[1]]", "é",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} parsed");
        }
        assert_eq!(
            parse("{} x").unwrap_err(),
            "Unexpected trailing characters at position 3"
        );
        assert_eq!(
            parse("[é]").unwrap_err(),
            "Unexpected character 'é' at position 1"
        );
        assert_eq!(parse("\"\\é\"").unwrap_err(), "Invalid escape sequence \\é");
    }

    /// A small deterministic generator, so the test needs no `rand`.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn string(&mut self) -> String {
            const PIECES: &[&str] = &["a", "Z", "0", " ", "\"", "\\", "/", "\n", "\t", "\x01", "é", "日", "😀", "\u{2028}"];
            (0..self.below(8)).map(|_| PIECES[self.below(PIECES.len() as u64) as usize]).collect()
        }
        fn value(&mut self, depth: u32) -> Value {
            match self.below(if depth == 0 { 5 } else { 7 }) {
                0 => Value::Null,
                1 => Value::Bool(self.below(2) == 1),
                2 => Value::Int(self.next() as i64 >> self.below(64)),
                3 => Value::Float((self.next() as i64 >> 20) as f64 / 1024.0),
                4 => Value::String(self.string()),
                5 => Value::Array((0..self.below(5)).map(|_| self.value(depth - 1)).collect()),
                _ => Value::Object((0..self.below(5)).map(|_| (self.string(), self.value(depth - 1))).collect()),
            }
        }
    }

    /// Our value against serde_json's, with numbers compared by value: the
    /// writer prints 2.0 as `2`, which reads back as an integer.
    fn same(ours: &Value, theirs: &serde_json::Value) -> bool {
        use serde_json::Value as J;
        match (ours, theirs) {
            (Value::Null, J::Null) => true,
            (Value::Bool(a), J::Bool(b)) => a == b,
            (Value::Int(_) | Value::Float(_), J::Number(n)) => {
                ours.as_f64() == n.as_f64()
            }
            (Value::String(a), J::String(b)) => a == b,
            (Value::Array(a), J::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
            }
            (Value::Object(a), J::Object(b)) => {
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| same(v, w)))
            }
            _ => false,
        }
    }

    #[test]
    fn agrees_with_serde_json_both_ways() {
        let mut rng = Rng(0x9E3779B97F4A7C15);
        for _ in 0..2000 {
            let v = rng.value(4);
            let ours = v.to_json_string();
            let theirs: serde_json::Value = serde_json::from_str(&ours)
                .unwrap_or_else(|e| panic!("serde_json refused {ours}: {e}"));
            assert!(same(&v, &theirs), "{ours}");
            assert!(same(&parsed(&ours), &theirs), "{ours}");
            // serde_json's text, compact and pretty, read by our parser.
            for text in [theirs.to_string(), serde_json::to_string_pretty(&theirs).unwrap()] {
                assert!(same(&parsed(&text), &theirs), "{text}");
            }
        }
    }

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
