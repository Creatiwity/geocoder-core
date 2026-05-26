use crate::text::compare_str;

const IMPORTANCE_WEIGHT: f32 = 0.1;
const STR_WEIGHT: f32 = 0.9;
const GEO_WEIGHT: f32 = 0.2;

/// Score a candidate document against the query.
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
        let name = doc.get("name").and_then(|v| v.as_str()).unwrap_or("");

        // Best token-level text similarity
        let token_score = self
            .query_tokens
            .iter()
            .map(|q| compare_str(q, name))
            .fold(0.0f32, f32::max);

        // Autocomplete distance: rewards exact/prefix/contains at the name level
        let auto_score = autocomplete_distance(self.query, name);

        let str_score = token_score.max(auto_score);

        // Base: text quality heavily weighted, importance lightly
        let base = str_score * STR_WEIGHT + importance * IMPORTANCE_WEIGHT;

        // With a geo hint, blend in geographic distance
        if let (Some(lat), Some(lon)) = (self.lat, self.lon) {
            let km = haversine_km(lat, lon, doc_lat, doc_lon);
            let geo_score = km_to_score(km);
            base * (1.0 - GEO_WEIGHT) + geo_score * GEO_WEIGHT
        } else {
            base
        }
    }
}

/// Score query string against a document name for autocomplete quality.
/// Returns 1.0 for exact match, 0.9 for prefix, 0.7 for contains, 0.0 otherwise.
pub fn autocomplete_distance(query: &str, name: &str) -> f32 {
    let q = query.to_lowercase();
    let n = name.to_lowercase();
    if n == q {
        1.0
    } else if n.starts_with(&q) {
        0.9
    } else if n.contains(q.as_str()) {
        0.7
    } else {
        0.0
    }
}

/// Convert km distance to a score in [0.0, 1.0].
/// 0km=1.0, 50km≈0.5, 200km≈0.2
pub fn km_to_score(km: f64) -> f32 {
    (1.0 / (1.0 + km / 50.0)) as f32
}

/// Haversine distance in km.
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
}
