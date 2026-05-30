use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct CanonicalJsonError(String);

impl From<String> for CanonicalJsonError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<serde_json::Error> for CanonicalJsonError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}

pub fn canonical_json(value: &Value) -> Result<String, CanonicalJsonError> {
    write_value(value, 0)
}

fn write_value(value: &Value, depth: usize) -> Result<String, CanonicalJsonError> {
    if depth > 32 {
        return Err("canonical_json_depth_exceeded".to_string().into());
    }

    match value {
        Value::Null => Ok("null".to_string()),
        Value::Bool(v) => Ok(if *v { "true" } else { "false" }.to_string()),
        Value::Number(n) => {
            if n.is_f64() {
                let f = n
                    .as_f64()
                    .ok_or_else(|| CanonicalJsonError("non_finite_number".to_string()))?;
                if !f.is_finite() {
                    return Err("non_finite_number".to_string().into());
                }
            }
            Ok(n.to_string())
        }
        Value::String(s) => Ok(serde_json::to_string(s)?),
        Value::Array(items) => {
            let mut out = String::from("[");
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(&write_value(item, depth + 1)?);
            }
            out.push(']');
            Ok(out)
        }
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();

            let mut out = String::from("{");
            for (idx, key) in keys.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key)?);
                out.push(':');
                out.push_str(&write_value(&map[*key], depth + 1)?);
            }
            out.push('}');
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_object_keys_recursively() {
        let value = json!({"z": 1, "a": {"b": true, "a": [2, 1]}});
        assert_eq!(
            canonical_json(&value).unwrap(),
            r#"{"a":{"a":[2,1],"b":true},"z":1}"#
        );
    }
}
