//! Documents are plain JSON objects in addok's import format (`name`, `type`,
//! `postcode`, `city`, `context`, `lat`, `lon`, `importance`, `housenumbers`…).
//! Readers in `crate::readers` convert each data source into this format.
//!
//! The helpers below reproduce how addok reads a document: a list holds a
//! default value followed by aliases, missing keys read as an empty string,
//! and values are stringified like Python's `str()`.

use serde_json::Value;

pub type Document = serde_json::Map<String, Value>;

/// `Result._rawattr`: the list of values of a key (`[""]` when missing).
pub fn raw_values(doc: &Document, key: &str) -> Vec<Value> {
    match doc.get(key) {
        None => vec![Value::String(String::new())],
        Some(Value::Array(values)) => values.clone(),
        Some(value) => vec![value.clone()],
    }
}

/// All values of a key, stringified.
pub fn str_values(doc: &Document, key: &str) -> Vec<String> {
    raw_values(doc, key).iter().map(py_str).collect()
}

/// First value of a key, stringified; `None` when it is null or falsy-empty.
pub fn first_str(doc: &Document, key: &str) -> Option<String> {
    let value = raw_values(doc, key).into_iter().next()?;
    if !truthy(&value) {
        return None;
    }
    Some(py_str(&value))
}

/// First value of a key as a float (`importance`, `lat`, `lon`).
pub fn first_f64(doc: &Document, key: &str) -> Option<f64> {
    match raw_values(doc, key).into_iter().next()? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python's `str()` on a decoded JSON value.
pub fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if n.is_f64() {
                let f = n.as_f64().unwrap_or_default();
                if f.fract() == 0.0 && f.abs() < 1e16 {
                    format!("{f:.1}")
                } else {
                    format!("{f}")
                }
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_like_addok() {
        let doc: Document = serde_json::from_value(
            json!({"city": ["Paris", "Paname"], "postcode": 75001, "x": 1.0, "n": null}),
        )
        .unwrap();
        assert_eq!(first_str(&doc, "city").as_deref(), Some("Paris"));
        assert_eq!(first_str(&doc, "postcode").as_deref(), Some("75001"));
        assert_eq!(py_str(&doc["x"]), "1.0");
        assert_eq!(first_str(&doc, "n"), None);
        assert_eq!(first_str(&doc, "missing"), None);
    }
}
