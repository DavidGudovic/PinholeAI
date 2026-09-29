//! Lenient serde helpers for CivitAI responses.
//!
//! The public CivitAI API is loosely typed in practice: fields go missing, turn
//! `null`, switch between strings and numbers (`sizeKB` is a float, cursors are
//! strings or numbers, `allowCommercialUse` is a string or an array). Every
//! helper here accepts anything and falls back to a default instead of failing
//! the whole page. Use with `#[serde(default, deserialize_with = "...")]`.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// String-ish value → `String` (numbers and bools are stringified).
pub fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn value_to_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && *f >= 0.0)
                .map(|f| f.round() as u64)
        }),
        Value::String(s) => {
            let s = s.trim();
            s.parse::<u64>().ok().or_else(|| {
                s.parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite() && *f >= 0.0)
                    .map(|f| f.round() as u64)
            })
        }
        _ => None,
    }
}

pub fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

pub fn value_to_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" | "" | "none" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(value_to_string(&Value::deserialize(d)?).unwrap_or_default())
}

/// `None` for null, missing, empty or non-string values.
pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(value_to_string(&Value::deserialize(d)?).filter(|s| !s.trim().is_empty()))
}

pub fn u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(value_to_u64(&Value::deserialize(d)?).unwrap_or_default())
}

pub fn opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(value_to_u64(&Value::deserialize(d)?))
}

pub fn f64<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Ok(value_to_f64(&Value::deserialize(d)?).unwrap_or_default())
}

pub fn bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(value_to_bool(&Value::deserialize(d)?).unwrap_or_default())
}

pub fn opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(value_to_bool(&Value::deserialize(d)?))
}

/// Array of `T`; elements that fail to parse are skipped, non-arrays → empty.
pub fn vec<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// Nested object; null or malformed → `T::default()`.
pub fn obj<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(d: D) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
}

/// Nested object; null or malformed → `None`.
pub fn opt_obj<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<T>, D::Error> {
    let v = Value::deserialize(d)?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(v).ok())
}

/// Strings from an array of strings / numbers / `{ "name": ... }` objects, or a
/// single string. Used for `tags` and `trainedWords`.
pub fn strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    fn one(v: &Value) -> Option<String> {
        match v {
            Value::Object(o) => o.get("name").and_then(value_to_string),
            other => value_to_string(other),
        }
    }
    Ok(match Value::deserialize(d)? {
        Value::Array(items) => items
            .iter()
            .filter_map(one)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    })
}

/// Object with string values (`hashes`); non-string values are dropped.
pub fn string_map<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Object(o) => o
            .iter()
            .filter_map(|(k, v)| value_to_string(v).map(|s| (k.clone(), s)))
            .collect(),
        _ => BTreeMap::new(),
    })
}

/// CivitAI NSFW level. Numbers are the browsing-level bit flags (1 PG, 2 PG-13,
/// 4 R, 8 X, 16 XXX, 32 blocked); older responses use strings
/// (`None` / `Soft` / `Mature` / `X`).
pub fn nsfw_level<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let v = Value::deserialize(d)?;
    if let Some(n) = value_to_u64(&v) {
        return Ok(Some(n.min(u32::MAX as u64) as u32));
    }
    Ok(match &v {
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "none" | "pg" | "safe" => Some(1),
            "soft" | "pg13" | "pg-13" => Some(2),
            "mature" | "r" => Some(4),
            "x" => Some(8),
            "xxx" => Some(16),
            "blocked" => Some(32),
            _ => None,
        },
        _ => None,
    })
}

/// Older image `nsfw` field: a bool, or a level string / number (R and above → true).
pub fn nsfw_level_or_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let v = Value::deserialize(d)?;
    if let Value::Bool(b) = v {
        return Ok(Some(b));
    }
    let level = nsfw_level(v).map_err(serde::de::Error::custom)?;
    Ok(level.map(|l| l >= 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, Deserialize)]
    #[serde(default)]
    struct T {
        #[serde(deserialize_with = "u64")]
        a: u64,
        #[serde(deserialize_with = "opt_string")]
        b: Option<String>,
        #[serde(deserialize_with = "strings")]
        c: Vec<String>,
        #[serde(deserialize_with = "bool")]
        d: bool,
        #[serde(deserialize_with = "nsfw_level")]
        e: Option<u32>,
        #[serde(deserialize_with = "f64")]
        f: f64,
    }

    #[test]
    fn accepts_messy_values() {
        let t: T = serde_json::from_str(r#"{"a":"12","b":7,"c":[{"name":"anime"},"3d",null,""],"d":null,"e":"Mature","f":"1.5"}"#).unwrap();
        assert_eq!(t.a, 12);
        assert_eq!(t.b.as_deref(), Some("7"));
        assert_eq!(t.c, vec!["anime", "3d"]);
        assert!(!t.d);
        assert_eq!(t.e, Some(4));
        assert_eq!(t.f, 1.5);
        let t: T =
            serde_json::from_str(r#"{"a":3.0,"b":null,"c":"one","d":"true","e":8,"f":null}"#)
                .unwrap();
        assert_eq!(t.a, 3);
        assert_eq!(t.b, None);
        assert_eq!(t.c, vec!["one"]);
        assert!(t.d);
        assert_eq!(t.e, Some(8));
        assert_eq!(t.f, 0.0);
        let t: T = serde_json::from_str("{}").unwrap();
        assert_eq!(t.a, 0);
    }
}
