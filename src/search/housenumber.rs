/// Try to match a housenumber query token against the housenumbers stored in a document.
/// Returns `(lat, lon, matched_number)` on success.
pub fn match_housenumber(
    query: &str,
    doc: &serde_json::Value,
) -> Option<(f64, f64, String)> {
    let housenumbers = doc.get("housenumbers")?.as_array()?;
    if housenumbers.is_empty() {
        return None;
    }

    let normalized_query = normalize_number(query);

    // 1. Exact match
    for hn in housenumbers {
        let number = hn.get("number")?.as_str()?;
        if normalize_number(number) == normalized_query {
            let lat = hn.get("lat")?.as_f64()?;
            let lon = hn.get("lon")?.as_f64()?;
            return Some((lat, lon, number.to_string()));
        }
    }

    // 2. Prefix match: "15" matches "15bis", "15b", "15 bis"
    for hn in housenumbers {
        let number = hn.get("number")?.as_str()?;
        let normalized_number = normalize_number(number);
        if normalized_number.starts_with(&normalized_query)
            // Ensure the prefix ends on a digit boundary (avoid "1" matching "15")
            && normalized_number[normalized_query.len()..]
                .chars()
                .next()
                .map(|c| !c.is_ascii_digit())
                .unwrap_or(true)
        {
            let lat = hn.get("lat")?.as_f64()?;
            let lon = hn.get("lon")?.as_f64()?;
            return Some((lat, lon, number.to_string()));
        }
    }

    None
}

/// Normalize a housenumber for comparison: lowercase, collapse spaces.
fn normalize_number(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc_with_housenumbers() -> serde_json::Value {
        json!({
            "name": "Rue de la Paix",
            "lat": 48.8698,
            "lon": 2.3310,
            "housenumbers": [
                {"number": "15", "lat": 48.8700, "lon": 2.3312},
                {"number": "15bis", "lat": 48.8701, "lon": 2.3313},
                {"number": "17", "lat": 48.8702, "lon": 2.3314},
            ]
        })
    }

    #[test]
    fn test_exact_match() {
        let doc = doc_with_housenumbers();
        let result = match_housenumber("15", &doc).unwrap();
        assert_eq!(result.2, "15");
        assert!((result.0 - 48.8700).abs() < 1e-6);
    }

    #[test]
    fn test_prefix_match() {
        let doc = doc_with_housenumbers();
        // "15" matches "15bis" if "15" itself not present... but here "15" is present
        // Test prefix: "15b" should match "15bis"
        let result = match_housenumber("15b", &doc).unwrap();
        assert_eq!(result.2, "15bis");
    }

    #[test]
    fn test_no_false_prefix() {
        let doc = doc_with_housenumbers();
        // "1" should NOT match "15" (digit boundary check)
        let result = match_housenumber("1", &doc);
        assert!(result.is_none());
    }

    #[test]
    fn test_not_found() {
        let doc = doc_with_housenumbers();
        assert!(match_housenumber("99", &doc).is_none());
    }

    #[test]
    fn test_no_housenumbers_field() {
        let doc = json!({"name": "Rue de la Paix", "lat": 48.87, "lon": 2.33});
        assert!(match_housenumber("15", &doc).is_none());
    }
}
