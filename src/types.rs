use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub type DocId = u32;
pub type TokenId = u32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub housenumbers: Vec<HouseNumber>,
    #[serde(default)]
    pub filters: HashMap<String, String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_importance() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseNumber {
    pub number: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub doc: serde_json::Value,
    pub score: f32,
    pub matched_housenumber: Option<String>,
    pub distance_km: Option<f64>,
}

impl SearchResult {
    /// Build a human-readable display label for the result.
    /// Format: "[housenumber] name[, city][ postcode]"
    pub fn label(&self) -> String {
        let doc = &self.doc;
        let name = doc.get("name").and_then(|v| v.as_str()).unwrap_or("");

        let mut parts: Vec<String> = Vec::new();

        // Housenumber prefix when matched
        if let Some(ref hn) = self.matched_housenumber {
            parts.push(format!("{} {}", hn, name));
        } else if !name.is_empty() {
            parts.push(name.to_string());
        }

        // City from extra fields or filters — take only primary value (before " / ")
        let city = doc
            .get("city")
            .and_then(|v| v.as_str())
            .map(|c| c.split(" / ").next().unwrap_or(c))
            .or_else(|| {
                doc.get("filters")
                    .and_then(|f| f.get("city"))
                    .and_then(|v| v.as_str())
            });

        // Postcode from filters
        let postcode = doc
            .get("postcode")
            .and_then(|v| v.as_str())
            .or_else(|| {
                doc.get("filters")
                    .and_then(|f| f.get("postcode"))
                    .and_then(|v| v.as_str())
            });

        match (city, postcode) {
            (Some(c), Some(p)) => parts.push(format!("{} {}", p, c)),
            (Some(c), None) => parts.push(c.to_string()),
            (None, Some(p)) => parts.push(p.to_string()),
            (None, None) => {}
        }

        parts.join(", ")
    }
}

#[derive(Debug, Default, Clone)]
pub struct SearchOpts {
    pub limit: Option<usize>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub filters: HashMap<String, Vec<String>>,
}

#[derive(Debug, Default, Clone)]
pub struct ReverseOpts {
    pub limit: Option<usize>,
    pub filters: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    /// Phonemicized form — used for FST lookup and scoring.
    pub normalized: String,
    /// Pre-phonemicize normalized form — used for synonym lookup.
    pub lemma: String,
    pub position: usize,
    pub is_housenumber: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result_from_doc(doc: serde_json::Value) -> SearchResult {
        SearchResult { doc, score: 1.0, matched_housenumber: None, distance_km: None }
    }

    #[test]
    fn test_label_name_only() {
        let r = result_from_doc(json!({"name": "Gare de Lyon"}));
        assert_eq!(r.label(), "Gare de Lyon");
    }

    #[test]
    fn test_label_with_postcode() {
        let r = result_from_doc(
            json!({"name": "Place de la République", "filters": {"postcode": "75011"}}),
        );
        assert_eq!(r.label(), "Place de la République, 75011");
    }

    #[test]
    fn test_label_with_housenumber() {
        let mut r = result_from_doc(json!({"name": "Rue de la Paix"}));
        r.matched_housenumber = Some("15".to_string());
        assert_eq!(r.label(), "15 Rue de la Paix");
    }

    #[test]
    fn test_label_with_city_and_postcode() {
        let r = result_from_doc(json!({
            "name": "Rue de la Paix",
            "filters": {"postcode": "75001", "city": "Paris"}
        }));
        assert_eq!(r.label(), "Rue de la Paix, 75001 Paris");
    }
}

