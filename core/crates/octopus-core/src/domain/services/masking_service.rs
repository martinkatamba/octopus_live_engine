use std::collections::HashSet;

use crate::json::{Value, ValueMap};

/// Returns `data` with the values of sensitive keys replaced, at any depth.
/// Maps and arrays with nothing to mask are shared with `data`, not copied;
/// `ValueMap` is copy-on-write, so the result is still independent of later
/// writes to `data`.
pub fn mask_payload(data: &ValueMap, fields_to_mask: &[String], replacement: &str) -> ValueMap {
    let replacement = if replacement.is_empty() {
        "********"
    } else {
        replacement
    };

    let mut mask_set = HashSet::with_capacity(fields_to_mask.len());
    for f in fields_to_mask {
        let low = f.trim().to_lowercase();
        if !low.is_empty() {
            mask_set.insert(low);
        }
    }
    if mask_set.is_empty() {
        return data.clone();
    }

    sanitize_map(data, &mask_set, replacement).unwrap_or_else(|| data.clone())
}

pub fn copy_map_recursive(data: &ValueMap) -> ValueMap {
    data.clone()
}

fn is_masked_key(key: &str, mask_set: &HashSet<String>) -> bool {
    let lower;
    let key = if key
        .bytes()
        .any(|b| !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'_')
    {
        lower = key.to_lowercase();
        lower.as_str()
    } else {
        key
    };
    if mask_set.contains(key) {
        return true;
    }
    if let Some(stripped) = key.strip_prefix("_injected_") {
        if mask_set.contains(stripped) {
            return true;
        }
    }
    false
}

/// The masked copy of `m`, or `None` when nothing in it needs masking.
fn sanitize_map(m: &ValueMap, mask_set: &HashSet<String>, replacement: &str) -> Option<ValueMap> {
    let mut out: Option<ValueMap> = None;
    for (k, v) in m {
        let changed = if is_masked_key(k, mask_set) {
            Some(Value::String(replacement.to_string()))
        } else {
            sanitize_value(v, mask_set, replacement)
        };
        if let Some(nv) = changed {
            out.get_or_insert_with(|| m.clone()).insert(k.clone(), nv);
        }
    }
    out
}

fn sanitize_value(val: &Value, mask_set: &HashSet<String>, replacement: &str) -> Option<Value> {
    match val {
        Value::Object(m) => sanitize_map(m, mask_set, replacement).map(Value::Object),
        Value::Array(arr) => {
            let mut out: Option<Vec<Value>> = None;
            for (i, item) in arr.iter().enumerate() {
                if let Some(nv) = sanitize_value(item, mask_set, replacement) {
                    out.get_or_insert_with(|| arr.clone())[i] = nv;
                }
            }
            out.map(Value::Array)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmap;

    #[test]
    fn masks_at_any_depth_and_shares_the_rest() {
        let public = vmap! { "sku" => "A1" };
        let data = vmap! {
            "Password" => "hunter2",
            "_injected_token" => "t",
            "user" => Value::Object(vmap! { "password" => "x", "name" => "Ann" }),
            "list" => Value::Array(vec![Value::Object(vmap! { "token" => "y" })]),
            "public" => Value::Object(public.clone()),
        };
        let fields = vec!["password".to_string(), "Token".to_string()];
        let out = mask_payload(&data, &fields, "");

        assert_eq!(out["Password"], Value::from("********"));
        assert_eq!(out["_injected_token"], Value::from("********"));
        assert_eq!(out["user"].get("password"), Some(&Value::from("********")));
        assert_eq!(out["user"].get("name"), Some(&Value::from("Ann")));
        assert_eq!(
            out["list"].as_array().unwrap()[0].get("token"),
            Some(&Value::from("********"))
        );
        // The original is untouched, and what needed no masking is shared.
        assert_eq!(data["Password"], Value::from("hunter2"));
        assert!(ValueMap::ptr_eq(
            out["public"].as_object().unwrap(),
            data["public"].as_object().unwrap()
        ));
        assert!(ValueMap::ptr_eq(&mask_payload(&data, &[], ""), &data));
    }
}
