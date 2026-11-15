use serde_json::Value;

use crate::doc::{Document, first_str};
use crate::error::GeocoderError;

/// BANO is close to addok's format, with its own types and context fields.
/// Normalized to what the BAN publishes so both sources behave alike:
///
/// - `city`, `town`, `village` → `municipality`; `place` → `locality`;
/// - `context` built as "<department code>, <department>, <region>";
/// - `citycode` set on municipalities (their `id`).
pub fn parse_line(line: &str) -> Result<Document, GeocoderError> {
    let mut doc: Document = serde_json::from_str(line)?;

    let kind = doc
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let normalized = match kind.as_str() {
        "city" | "town" | "village" => "municipality",
        "place" => "locality",
        other => other,
    };
    doc.insert("type".into(), Value::String(normalized.to_string()));

    if normalized == "municipality"
        && !doc.contains_key("citycode")
        && let Some(id) = first_str(&doc, "id")
    {
        doc.insert("citycode".into(), Value::String(id));
    }

    if !doc.contains_key("context") {
        let code = first_str(&doc, "citycode").map(|c| department_code(&c));
        let parts: Vec<String> = [
            code,
            first_str(&doc, "departement"),
            first_str(&doc, "region"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !parts.is_empty() {
            doc.insert("context".into(), Value::String(parts.join(", ")));
        }
    }
    Ok(doc)
}

/// Department code out of an INSEE city code: 3 characters overseas.
fn department_code(citycode: &str) -> String {
    let len = if citycode.starts_with("97") || citycode.starts_with("98") {
        3
    } else {
        2
    };
    citycode.chars().take(len).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_like_the_ban() {
        let doc = parse_line(r#"{"id":"90010","type":"city","name":"Belfort","postcode":"90000","lat":47.6,"lon":6.8,"city":["Belfort"],"departement":"Territoire de Belfort","region":"Bourgogne-Franche-Comté","importance":0.74}"#).unwrap();
        assert_eq!(doc["type"], "municipality");
        assert_eq!(doc["citycode"], "90010");
        assert_eq!(
            doc["context"],
            "90, Territoire de Belfort, Bourgogne-Franche-Comté"
        );
    }
}
