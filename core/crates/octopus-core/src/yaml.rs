use crate::json::{Value, ValueMap};

#[derive(Debug, Clone)]
struct YamlLine {
    line_num: usize,
    indent: usize,
    content: String,
}

pub fn parse_yaml(input: &str) -> Result<Value, String> {
    let mut lines = Vec::new();
    for (idx, raw_line) in input.lines().enumerate() {
        let stripped = strip_yaml_comment(raw_line);
        let trimmed_end = stripped.trim_end();
        if trimmed_end.trim().is_empty() {
            continue;
        }
        if trimmed_end.trim() == "---" || trimmed_end.trim() == "..." {
            continue;
        }
        let mut indent = 0;
        for ch in trimmed_end.chars() {
            if ch == ' ' {
                indent += 1;
            } else if ch == '\t' {
                return Err(format!(
                    "line {}: tabs are not allowed for YAML indentation",
                    idx + 1
                ));
            } else {
                break;
            }
        }
        let content = trimmed_end[indent..].to_string();
        lines.push(YamlLine {
            line_num: idx + 1,
            indent,
            content,
        });
    }

    if lines.is_empty() {
        return Ok(Value::Object(ValueMap::new()));
    }

    let mut pos = 0;
    let base_indent = lines[0].indent;
    let val = parse_block(&lines, &mut pos, base_indent)?;
    if pos < lines.len() {
        return Err(format!(
            "line {}: unexpected indentation or trailing content '{}'",
            lines[pos].line_num, lines[pos].content
        ));
    }
    Ok(val)
}

fn strip_yaml_comment(line: &str) -> String {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_backslash = false;
    let mut out = String::new();
    for (i, ch) in line.char_indices() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single && !prev_backslash {
            in_double = !in_double;
        } else if ch == '#'
            && !in_single
            && !in_double
            && (i == 0 || line[..i].ends_with(' ') || line[..i].ends_with('\t'))
        {
            break;
        }
        prev_backslash = ch == '\\' && !prev_backslash;
        out.push(ch);
    }
    out
}

fn parse_block(
    lines: &[YamlLine],
    pos: &mut usize,
    expected_indent: usize,
) -> Result<Value, String> {
    if *pos >= lines.len() {
        return Ok(Value::Null);
    }
    let line = &lines[*pos];
    if line.indent != expected_indent {
        return Err(format!(
            "line {}: inconsistent indentation (expected {}, got {})",
            line.line_num, expected_indent, line.indent
        ));
    }
    if line.content == "-" || line.content.starts_with("- ") {
        parse_sequence(lines, pos, expected_indent)
    } else {
        parse_mapping(lines, pos, expected_indent)
    }
}

fn parse_sequence(lines: &[YamlLine], pos: &mut usize, seq_indent: usize) -> Result<Value, String> {
    let mut items = Vec::new();
    while *pos < lines.len() {
        let line = &lines[*pos];
        if line.indent < seq_indent {
            break;
        }
        if line.indent > seq_indent {
            return Err(format!(
                "line {}: invalid indentation inside sequence (expected {}, got {})",
                line.line_num, seq_indent, line.indent
            ));
        }
        if line.content != "-" && !line.content.starts_with("- ") {
            break;
        }

        let after_dash = if line.content == "-" {
            ""
        } else {
            &line.content[2..]
        };
        let trimmed_after = after_dash.trim_start();
        let offset = after_dash.len() - trimmed_after.len();
        let item_indent = seq_indent + 2 + offset;
        let current_line_num = line.line_num;
        *pos += 1;

        if trimmed_after.is_empty() {
            if *pos < lines.len() && lines[*pos].indent > seq_indent {
                let child_indent = lines[*pos].indent;
                let item_val = parse_block(lines, pos, child_indent)?;
                items.push(item_val);
            } else {
                items.push(Value::Null);
            }
            continue;
        }

        if let Some((key, rest)) = split_key_value(trimmed_after) {
            let mut map = ValueMap::new();
            let first_val = if rest.is_empty() {
                if *pos < lines.len() && lines[*pos].indent > item_indent {
                    let child_indent = lines[*pos].indent;
                    parse_block(lines, pos, child_indent)?
                } else if *pos < lines.len()
                    && lines[*pos].indent > seq_indent
                    && (lines[*pos].content == "-" || lines[*pos].content.starts_with("- "))
                {
                    let child_indent = lines[*pos].indent;
                    parse_sequence(lines, pos, child_indent)?
                } else {
                    Value::Null
                }
            } else {
                parse_scalar_or_inline(&rest)
                    .map_err(|e| format!("line {}: {}", current_line_num, e))?
            };
            map.insert(key, first_val);

            // Parse remaining fields of this mapping item at `item_indent`
            while *pos < lines.len() {
                let next_line = &lines[*pos];
                if next_line.indent < item_indent {
                    if next_line.indent > seq_indent {
                        return Err(format!(
                            "line {}: misaligned indentation (expected {} or <= {}, got {})",
                            next_line.line_num, item_indent, seq_indent, next_line.indent
                        ));
                    }
                    break;
                }
                if next_line.indent > item_indent {
                    return Err(format!(
                        "line {}: unexpected indentation (expected {}, got {})",
                        next_line.line_num, item_indent, next_line.indent
                    ));
                }
                if next_line.content == "-" || next_line.content.starts_with("- ") {
                    break;
                }
                let (k, r) = split_key_value(&next_line.content).ok_or_else(|| {
                    format!(
                        "line {}: expected key: value in mapping, got '{}'",
                        next_line.line_num, next_line.content
                    )
                })?;
                let ln = next_line.line_num;
                *pos += 1;
                let v = if r.is_empty() {
                    if *pos < lines.len() && lines[*pos].indent > item_indent {
                        let child_indent = lines[*pos].indent;
                        parse_block(lines, pos, child_indent)?
                    } else if *pos < lines.len()
                        && lines[*pos].indent == item_indent
                        && (lines[*pos].content == "-" || lines[*pos].content.starts_with("- "))
                    {
                        parse_sequence(lines, pos, item_indent)?
                    } else {
                        Value::Null
                    }
                } else {
                    parse_scalar_or_inline(&r).map_err(|e| format!("line {}: {}", ln, e))?
                };
                map.insert(k, v);
            }
            items.push(Value::Object(map));
        } else {
            let val = parse_scalar_or_inline(trimmed_after)
                .map_err(|e| format!("line {}: {}", current_line_num, e))?;
            items.push(val);
        }
    }
    Ok(Value::Array(items))
}

fn parse_mapping(lines: &[YamlLine], pos: &mut usize, map_indent: usize) -> Result<Value, String> {
    let mut map: ValueMap = ValueMap::new();
    while *pos < lines.len() {
        let line = &lines[*pos];
        if line.indent < map_indent {
            break;
        }
        if line.indent > map_indent {
            return Err(format!(
                "line {}: unexpected indentation in mapping (expected {}, got {})",
                line.line_num, map_indent, line.indent
            ));
        }
        if line.content == "-" || line.content.starts_with("- ") {
            break;
        }
        let (key, rest) = split_key_value(&line.content).ok_or_else(|| {
            format!(
                "line {}: expected 'key: value', got '{}'",
                line.line_num, line.content
            )
        })?;
        let ln = line.line_num;
        *pos += 1;
        let val = if rest.is_empty() {
            if *pos < lines.len() && lines[*pos].indent > map_indent {
                let child_indent = lines[*pos].indent;
                parse_block(lines, pos, child_indent)?
            } else if *pos < lines.len()
                && lines[*pos].indent == map_indent
                && (lines[*pos].content == "-" || lines[*pos].content.starts_with("- "))
            {
                parse_sequence(lines, pos, map_indent)?
            } else {
                Value::Null
            }
        } else {
            parse_scalar_or_inline(&rest).map_err(|e| format!("line {}: {}", ln, e))?
        };
        map.insert(key, val);
    }
    Ok(Value::Object(map))
}

fn split_key_value(s: &str) -> Option<(String, String)> {
    let mut in_single = false;
    let mut in_double = false;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len() {
        let ch = chars[i];
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        } else if !in_single && !in_double {
            if ch == '[' {
                bracket_depth += 1;
            } else if ch == ']' {
                bracket_depth -= 1;
            } else if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth -= 1;
            } else if ch == ':'
                && bracket_depth == 0
                && brace_depth == 0
                && (i + 1 == chars.len() || chars[i + 1].is_whitespace())
            {
                let raw_key: String = chars[..i].iter().collect();
                let raw_val: String = chars[i + 1..].iter().collect();
                let key = unquote_yaml_str(raw_key.trim());
                return Some((key, raw_val.trim().to_string()));
            }
        }
    }
    None
}

fn unquote_yaml_str(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        if trimmed.starts_with('"') {
            return inner
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
                .replace("\\n", "\n")
                .replace("\\t", "\t");
        }
        return inner.to_string();
    }
    trimmed.to_string()
}

fn parse_scalar_or_inline(s: &str) -> Result<Value, String> {
    let trimmed = s.trim();
    if trimmed.is_empty()
        || trimmed == "null"
        || trimmed == "~"
        || trimmed == "Null"
        || trimmed == "NULL"
    {
        return Ok(Value::Null);
    }
    if trimmed == "true" || trimmed == "True" || trimmed == "TRUE" {
        return Ok(Value::Bool(true));
    }
    if trimmed == "false" || trimmed == "False" || trimmed == "FALSE" {
        return Ok(Value::Bool(false));
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = trimmed[1..trimmed.len() - 1].trim();
        if inner.is_empty() {
            return Ok(Value::Array(Vec::new()));
        }
        let parts = split_inline_collection(inner)?;
        let mut arr = Vec::new();
        for p in parts {
            arr.push(parse_scalar_or_inline(&p)?);
        }
        return Ok(Value::Array(arr));
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = trimmed[1..trimmed.len() - 1].trim();
        if inner.is_empty() {
            return Ok(Value::Object(ValueMap::new()));
        }
        let parts = split_inline_collection(inner)?;
        let mut map = ValueMap::new();
        for p in parts {
            let (k, v_str) = split_key_value(&p)
                .ok_or_else(|| format!("Invalid inline mapping entry '{}'", p))?;
            map.insert(k, parse_scalar_or_inline(&v_str)?);
        }
        return Ok(Value::Object(map));
    }
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        return Ok(Value::String(unquote_yaml_str(trimmed)));
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Ok(Value::Int(i));
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        return Ok(Value::Float(f));
    }
    Ok(Value::String(trimmed.to_string()))
}

fn split_inline_collection(s: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth = 0i32;

    for ch in s.chars() {
        if ch == '\'' && !in_double {
            in_single = !in_single;
            current.push(ch);
        } else if ch == '"' && !in_single {
            in_double = !in_double;
            current.push(ch);
        } else if !in_single && !in_double {
            if ch == '[' || ch == '{' {
                depth += 1;
                current.push(ch);
            } else if ch == ']' || ch == '}' {
                depth -= 1;
                current.push(ch);
            } else if ch == ',' && depth == 0 {
                parts.push(current.trim().to_string());
                current.clear();
            } else {
                current.push(ch);
            }
        } else {
            current.push(ch);
        }
    }
    if in_single || in_double || depth != 0 {
        return Err("Unbalanced quotes or brackets in inline collection".to_string());
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    Ok(parts)
}
