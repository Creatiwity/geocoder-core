use crate::text::{compare_label, compare_str};
use crate::text::normalize::normalize;

const IMPORTANCE_WEIGHT: f32 = 0.1;
const GEO_DISTANCE_WEIGHT: f32 = 0.1;

pub struct Scorer<'a> {
    pub query: &'a str,
    pub query_tokens: &'a [&'a str],
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

impl<'a> Scorer<'a> {
    pub fn score(&self, doc: &serde_json::Value, doc_lat: f64, doc_lon: f64, matched_hn: Option<&str>) -> f32 {
        let importance = doc
            .get("importance")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32;

        // Build the searchable label, mirroring addok's make_labels (prepend housenumber when matched)
        let base_label = build_label(doc);
        let label = match matched_hn {
            Some(hn) => format!("{} {}", hn, base_label),
            None => base_label,
        };

        // 1. Full-label vs full-query bigram Dice (addok's compare_ngrams N=2, pad_len=0)
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
        // Ceilings are NEVER modified (only values change in adjust_scores).
        let mut score_sum = str_score + importance * IMPORTANCE_WEIGHT;
        let mut ceil_sum = 1.0_f32 + IMPORTANCE_WEIGHT;

        if let (Some(lat), Some(lon)) = (self.lat, self.lon) {
            // adjust_scores: multiply values by 0.9 (str) and 0.1 (importance), ceilings unchanged
            score_sum = str_score * 0.9 + importance * IMPORTANCE_WEIGHT * 0.1;
            // ceil_sum stays: 1.0 (str_distance) + IMPORTANCE_WEIGHT (importance) = 1.1
            // then geo ceiling is added below → 1.2 total
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
    let city = doc.get("city").and_then(|v| v.as_str()).map(|c| c.split(" / ").next().unwrap_or(c));

    match (postcode, city) {
        (Some(p), Some(c)) => format!("{} {} {}", name, p, c),
        (Some(p), None) => format!("{} {}", name, p),
        (None, Some(c)) => format!("{} {}", name, c),
        (None, None) => name.to_string(),
    }
}

/// Exact / prefix / contains match of ASCII-folded query against ASCII-folded label.
/// Returns 1.0 / 0.9 / 0.7 / 0.0 per addok's score_by_autocomplete_distance.
pub fn autocomplete_distance(query: &str, label: &str) -> f32 {
    let q = normalize(query);
    let l = normalize(label);
    if l == q {
        1.0
    } else if l.starts_with(q.as_str()) {
        0.9
    } else if l.contains(q.as_str()) {
        0.7
    } else {
        0.0
    }
}

/// Gaussian decay: exp(-(km/50)²), clipped to 0.0 beyond 100 km.
/// Mirrors addok's km_to_score() from helpers/__init__.py.
pub fn km_to_score(km: f64) -> f32 {
    if km > 100.0 {
        return 0.0;
    }
    f64::exp(-(km / 50.0).powi(2)) as f32
}

/// Haversine distance in km. Earth radius = 6367 km (per addok spec).
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6367.0;
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
    fn test_autocomplete_distance_exact_accents() {
        // ASCII folding: accented chars must match
        assert_eq!(autocomplete_distance("ile de france", "Île-de-France"), 1.0);
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
    fn test_km_to_score_zero() {
        assert!((km_to_score(0.0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_km_to_score_gaussian() {
        // exp(-(50/50)²) = exp(-1) ≈ 0.368
        let s = km_to_score(50.0);
        assert!((s - 0.368).abs() < 0.01, "score at 50km was {}", s);
    }

    #[test]
    fn test_km_to_score_cutoff() {
        assert_eq!(km_to_score(100.1), 0.0);
        assert_eq!(km_to_score(200.0), 0.0);
    }

    #[test]
    fn test_km_to_score_table() {
        // Values from SCORING.md table
        assert!((km_to_score(10.0) - 0.960).abs() < 0.01);
        assert!((km_to_score(25.0) - 0.779).abs() < 0.01);
        assert!((km_to_score(75.0) - 0.105).abs() < 0.01);
    }

    #[test]
    fn test_haversine_paris_lyon() {
        let km = haversine_km(48.8566, 2.3522, 45.7640, 4.8357);
        assert!(km > 390.0 && km < 420.0);
    }
}
