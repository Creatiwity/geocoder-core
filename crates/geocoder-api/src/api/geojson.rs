//! Results as the BAN API returns them: a GeoJSON `FeatureCollection` whose
//! features carry the document fields, `label` and `score`.

use geocoder_core::SearchResult;
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::config::Country;

#[derive(Debug, Serialize, ToSchema)]
pub struct FeatureCollection {
    /// Always `FeatureCollection`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub version: &'static str,
    pub features: Vec<Feature>,
    pub attribution: &'static str,
    pub licence: &'static str,
    /// The `q` parameter (search only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub filters: Option<Map<String, Value>>,
    /// `[lon, lat]` of the `lat` / `lon` parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub center: Option<[f64; 2]>,
    pub limit: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Feature {
    /// Always `Feature`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub geometry: Point,
    /// `label`, `score`, `housenumber`, `id`, `name`, `postcode`,
    /// `citycode`, `city`, `context`, `type`, `importance`, `street`,
    /// `distance` (meters, with a center). With a CEDEX in the query,
    /// `cedex` (`code`, `label`: as written, normalized) and `postal_label`,
    /// the address to write on mail ("1 Avenue du Général de Gaulle 92074
    /// PARIS LA DEFENSE CEDEX"); the other fields are the geographic place.
    #[schema(value_type = Object)]
    pub properties: Map<String, Value>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Point {
    /// Always `Point`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// `[lon, lat]`.
    pub coordinates: [f64; 2],
}

/// Document keys that are not result properties.
const INTERNAL: &[&str] = &["housenumbers", "lat", "lon", "raw"];

/// addok keeps a list as "default value, then aliases": the API shows the
/// value the label uses (Belgium: the language of the query), else the
/// default.
fn shown(value: &Value, label: &str) -> Option<Value> {
    match value {
        Value::Array(values) => values
            .iter()
            .find(|v| {
                v.as_str()
                    .is_some_and(|s| !s.is_empty() && label.contains(s))
            })
            .or(values.first())
            .cloned(),
        Value::Null => None,
        other => Some(other.clone()),
    }
}

fn names(doc: &Value) -> Vec<String> {
    match doc.get("name") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

pub fn feature(result: &SearchResult, country: &Country) -> Feature {
    let mut properties = Map::new();
    properties.insert("label".into(), Value::String(result.label.clone()));
    properties.insert("score".into(), score(result.score));
    if let Some(hn) = &result.housenumber {
        properties.insert("housenumber".into(), Value::String(hn.clone()));
    }

    // The name in the language of the label (Belgium), else the default.
    let all_names = names(&result.doc);
    let street = all_names
        .iter()
        .find(|n| result.label.contains(n.as_str()))
        .or(all_names.first())
        .cloned();

    if let Value::Object(doc) = &result.doc {
        for (key, value) in doc {
            if INTERNAL.contains(&key.as_str()) || key == "name" {
                continue;
            }
            if let Some(value) = shown(value, &result.label) {
                properties.insert(key.clone(), value);
            }
        }
    }
    match (&result.housenumber, street) {
        (Some(hn), Some(street)) => {
            let name = if country.number_after_street {
                format!("{street} {hn}")
            } else {
                format!("{hn} {street}")
            };
            properties.insert("name".into(), Value::String(name));
            properties.insert("street".into(), Value::String(street));
        }
        (None, Some(name)) => {
            properties.insert("name".into(), Value::String(name));
        }
        _ => {}
    }
    if let Some(km) = result.distance_km {
        properties.insert("distance".into(), Value::from((km * 1000.0).round() as u64));
    }
    // The CEDEX line of the query, given back: the place was searched,
    // the mail address keeps it.
    if let (Some(line), Some(label)) = (&result.postal, &result.postal_label) {
        properties.insert(
            "cedex".into(),
            serde_json::to_value(line).unwrap_or(Value::Null),
        );
        properties.insert("postal_label".into(), Value::String(label.clone()));
    }
    Feature {
        kind: "Feature",
        geometry: Point {
            kind: "Point",
            coordinates: [round6(result.lon), round6(result.lat)],
        },
        properties,
    }
}

fn score(score: f32) -> Value {
    serde_json::Number::from_f64((score as f64 * 1e6).round() / 1e6)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn country(after: bool) -> Country {
        Country {
            code: "be".into(),
            source: geocoder_core::readers::Source::Be,
            profile: "be",
            urls: vec![],
            index_path: PathBuf::new(),
            attribution: "",
            licence: "",
            number_after_street: after,
            cedex: false,
        }
    }

    #[test]
    fn housenumber_feature_like_the_ban() {
        let result = SearchResult {
            doc: json!({"id": "be_bru_1", "type": "housenumber", "name": ["Rue de la Loi", "Wetstraat"],
                        "postcode": ["1000", "1040"], "city": ["Brussel", "Bruxelles"], "lat": 50.8, "lon": 4.3,
                        "housenumbers": {}}),
            label: "Wetstraat 16, 1000 Brussel".into(),
            score: 0.98,
            housenumber: Some("16".into()),
            lat: 50.84617,
            lon: 4.36654,
            distance_km: None,
            postal: None,
            postal_label: None,
        };
        let f = feature(&result, &country(true));
        let p = &f.properties;
        assert_eq!(p["name"], "Wetstraat 16");
        assert_eq!(p["street"], "Wetstraat");
        assert_eq!(p["postcode"], "1000");
        assert_eq!(p["city"], "Brussel");
        assert_eq!(p["score"], 0.98);
        assert!(p.get("housenumbers").is_none() && p.get("lat").is_none());
        assert_eq!(f.geometry.coordinates, [4.36654, 50.84617]);
        assert_eq!(
            feature(&result, &country(false)).properties["name"],
            "16 Wetstraat"
        );
    }
}
