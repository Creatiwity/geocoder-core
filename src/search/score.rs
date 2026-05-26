use crate::text::{compare_label, compare_str};

const IMPORTANCE_WEIGHT: f32 = 0.1;
const GEO_DISTANCE_WEIGHT: f32 = 0.1;

pub struct Scorer<'a> {
    pub query: &'a str,
    pub query_tokens: &'a [&'a str],
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

impl<'a> Scorer<'a> {
    pub fn score(&self, doc: &serde_json::Value, doc_lat: f64, doc_lon: f64) -> f32 {
        let importance = doc
            .get("importance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;

        // Build the searchable label (name + postcode + city), mirroring addok's make_labels
        let label = build_label(doc);

        // 1. Full-label vs full-query bigram Dice (addok's compare_ngrams N=2)
        let label_score = compare_label(self.query, &label);

        // 2. Autocomplete distance (exact / prefix / contains) on the label
        let auto_score = autocomplete_distance(self.query, &label);

        // 3. Token-level fallback for short/partial queries
        let name = doc.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let token_score = self
            .query_tokens
            .iter()
            .map(|q| compare_str(q, name))
            .fold(0.0f32, f32::max);

        let str_score = label_score.max(auto_score).max(token_score);

        // Addok formula: sum(scores) / sum(ceilings)
        // Without geo: (str_score * 1.0 + importance * IMPORTANCE_WEIGHT) / (1.0 + IMPORTANCE_WEIGHT)
        let mut score_sum = str_score + importance * IMPORTANCE_WEIGHT;
        let mut ceil_sum = 1.0 + IMPORTANCE_WEIGHT;

        if let (Some(lat), Some(lon)) = (self.lat, self.lon) {
            // With geo hint: str_score * 0.9, importance contribution * 0.1 (adjust_scores)
            score_sum = str_score * 0.9 + importance * IMPORTANCE_WEIGHT * 0.1;
            ceil_sum = 0.9 + IMPORTANCE_WEIGHT * 0.1;
            // Add geo component
            let km = haversine_km(lat, lon, doc_lat, doc_lon);
            score_sum += km_to_score(km) * GEO_DISTANCE_WEIGHT;
            ceil_sum += GEO_DISTANCE_WEIGHT;
        }

        score_sum / ceil_sum
    }
}

/// Build a searchable label from a document, matching addok's make_labels output.
/// Format: "{name} {postcode} {city}"
fn build_label(doc: &serde_json::Value) -> String {
    let name = doc.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let postcode = doc
        .get("filters")
        .and_then(|f| f.get("postcode"))
        .and_then(|v| v.as_str());
    let city = doc.get("city").and_then(|v| v.as_str());

    match (postcode, city) {
        (Some(p), Some(c)) => format!("{} {} {}", name, p, c),
        (Some(p), None) => format!("{} {}", name, p),
        (None, Some(c)) => format!("{} {}", name, c),
        (None, None) => name.to_string(),
    }
}

/// Exact / prefix / contains match of query against label.
/// Returns 1.0 / 0.9 / 0.7 / 0.0.
pub fn autocomplete_distance(query: &str, label: &str) -> f32 {
    let q = query.to_lowercase();
    let l = label.to_lowercase();
    if l == q { 1.0 } else if l.starts_with(&q) { 0.9 } else if l.contains(q.as_str()) { 0.7 } else { 0.0 }
}

pub fn km_to_score(km: f64) -> f32 {
    (1.0 / (1.0 + km / 50.0)) as f32
}

pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autocomplete_distance_exact() {
        assert_eq!(autocomplete_distance("rue de la paix", "Rue de la Paix"), 1.0);
    }

    #[test]
    fn test_autocomplete_distance_prefix() {
        assert_eq!(autocomplete_distance("rue de", "Rue de la Paix"), 0.9);
    }

    #[test]
    fn test_autocomplete_distance_contains() {
        assert_eq!(autocomplete_distance("paix", "Rue de la Paix"), 0.7);
    }

    #[test]
    fn test_autocomplete_distance_none() {
        assert_eq!(autocomplete_distance("lyon", "Rue de la Paix"), 0.0);
    }

    #[test]
    fn test_km_to_score() {
        assert!((km_to_score(0.0) - 1.0).abs() < 0.01);
        assert!(km_to_score(50.0) > 0.4 && km_to_score(50.0) < 0.6);
        assert!(km_to_score(200.0) <= 0.2);
    }

    #[test]
    fn test_haversine_paris_lyon() {
        let km = haversine_km(48.8566, 2.3522, 45.7640, 4.8357);
        assert!(km > 390.0 && km < 420.0);
    }

    #[test]
    fn test_compare_label_bigram() {
        // Full label vs full query similarity
        let score = compare_label(
            "20 avenue des champs elysees paris",
            "Avenue des Champs Elysees 75008 Paris",
        );
        assert!(score > 0.6, "score was {}", score);
    }
}
