//! addok 1.1's `SEARCH_RESULT_PROCESSORS` as configured by the Géoplateforme:
//! `match_housenumber`, `make_labels`, `score_by_importance`,
//! `score_by_autocomplete_distance`, `score_by_ngram_distance`,
//! `score_by_geo_distance`. (`adjust_scores` is not part of it.)

use serde_json::Value;

use crate::doc::{Document, first_f64};
use crate::profile::Profile;
use crate::settings::{IMPORTANCE_WEIGHT, MATCH_THRESHOLD};
use crate::text::{ascii, compare_ngrams};
use crate::types::DocId;

/// What the result processors need from the query.
pub struct Context<'a> {
    pub profile: &'a dyn Profile,
    /// ASCII folded raw query (not cleaned by the query processors).
    pub query: &'a str,
    pub autocomplete: bool,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub check_housenumber: bool,
    /// `type=housenumber` filter: results without a matching housenumber
    /// are dropped.
    pub only_housenumber: bool,
    /// Housenumber tokens joined in query order.
    pub housenumber: &'a str,
}

/// A scored candidate, `addok.core.Result`.
#[derive(Debug, Clone)]
pub struct Scored {
    pub id: DocId,
    pub doc: Document,
    pub labels: Vec<String>,
    pub housenumber: Option<String>,
    pub distance_km: Option<f64>,
    importance: Option<(f64, f64)>,
    str_distance: Option<(f64, f64)>,
    geo_distance: Option<(f64, f64)>,
}

/// `Result.add_score`: keep the best value seen.
fn add_score(slot: &mut Option<(f64, f64)>, score: f64, ceiling: f64) {
    if score >= slot.map_or(0.0, |s| s.0) {
        *slot = Some((score, ceiling));
    }
}

impl Scored {
    pub fn score(&self) -> f64 {
        let (mut sum, mut max) = (0.0, 0.0);
        for (score, ceiling) in [self.importance, self.str_distance, self.geo_distance]
            .into_iter()
            .flatten()
        {
            sum += score;
            max += ceiling;
        }
        if max == 0.0 { 0.0 } else { sum / max }
    }

    pub fn str_distance(&self) -> f64 {
        self.str_distance.map_or(0.0, |s| s.0)
    }

    /// `str(result)`: first label, else the name.
    pub fn label(&self) -> String {
        self.labels.first().cloned().unwrap_or_else(|| {
            crate::doc::str_values(&self.doc, "name")
                .into_iter()
                .next()
                .unwrap_or_default()
        })
    }

    pub fn lat(&self) -> f64 {
        first_f64(&self.doc, "lat").unwrap_or_default()
    }

    pub fn lon(&self) -> f64 {
        first_f64(&self.doc, "lon").unwrap_or_default()
    }
}

/// Run the processors on one document; `None` when one of them rejects it.
pub fn process(ctx: &Context, id: DocId, mut doc: Document) -> Option<Scored> {
    // match_housenumber
    let mut housenumber = None;
    if ctx.check_housenumber {
        if !ctx.housenumber.is_empty()
            && let Some(Value::Object(data)) = doc
                .get("housenumbers")
                .and_then(|h| h.get(ctx.housenumber))
                .cloned()
        {
            let mut data = data;
            housenumber = data.remove("raw").map(|v| crate::doc::py_str(&v));
            doc.insert("type".into(), Value::String("housenumber".into()));
            doc.extend(data);
        }
        if ctx.only_housenumber && housenumber.is_none() {
            return None;
        }
    }

    let labels = ctx.profile.make_labels(&doc, housenumber.as_deref());
    let mut result = Scored {
        id,
        doc,
        labels,
        housenumber,
        distance_km: None,
        importance: None,
        str_distance: None,
        geo_distance: None,
    };

    // score_by_importance
    let importance = first_f64(&result.doc, "importance").unwrap_or(0.0);
    add_score(
        &mut result.importance,
        importance * IMPORTANCE_WEIGHT,
        IMPORTANCE_WEIGHT,
    );

    let labels: Vec<String> = result.labels.iter().map(|l| ascii(l)).collect();
    if ctx.autocomplete {
        // score_by_autocomplete_distance: every label is tried (addok 1.1
        // dropped the early exit of 1.0), the best one wins.
        let query = ascii(ctx.query);
        let mut score = 0.0;
        for label in &labels {
            if *label == query {
                score = 1.0;
            } else if label.starts_with(query.as_str()) {
                score = 0.9;
            } else if label.contains(query.as_str()) {
                score = 0.7;
            }
            if score > 0.0 {
                add_score(&mut result.str_distance, score, 1.0);
            }
        }
        if score == 0.0 {
            score_by_ngram_distance(&mut result, &labels, ctx.query, 0.9);
        }
    } else {
        score_by_ngram_distance(&mut result, &labels, ctx.query, 1.0);
    }

    // score_by_geo_distance
    if let (Some(lat), Some(lon)) = (ctx.lat, ctx.lon) {
        let km = haversine_km(result.lat(), result.lon(), lat, lon);
        result.distance_km = Some(km);
        add_score(&mut result.geo_distance, km_to_score(km), 0.1);
    }
    Some(result)
}

fn score_by_ngram_distance(result: &mut Scored, labels: &[String], query: &str, scale: f64) {
    for label in labels {
        let score = compare_ngrams(label, query) * scale;
        add_score(&mut result.str_distance, score, 1.0);
        if score >= MATCH_THRESHOLD {
            break;
        }
    }
}

/// Reverse geocoding processors: `load_closer`, `make_labels`,
/// `score_by_geo_distance`.
pub fn process_reverse(
    profile: &dyn Profile,
    id: DocId,
    mut doc: Document,
    lat: f64,
    lon: f64,
    check_housenumber: bool,
    only_housenumber: bool,
) -> Option<Scored> {
    let mut housenumber = None;
    if check_housenumber {
        let center = (
            first_f64(&doc, "lat").unwrap_or_default(),
            first_f64(&doc, "lon").unwrap_or_default(),
        );
        let mut best: Option<(f64, &serde_json::Map<String, Value>)> = None;
        if let Some(Value::Object(hns)) = doc.get("housenumbers") {
            for data in hns.values().filter_map(|v| v.as_object()) {
                let (Some(hlat), Some(hlon)) = (first_f64(data, "lat"), first_f64(data, "lon"))
                else {
                    continue;
                };
                let d = haversine_km(hlat, hlon, lat, lon);
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, data));
                }
            }
        }
        // addok sorts the housenumbers then the street center (left out when
        // only housenumbers are wanted), stably.
        let street = if only_housenumber {
            f64::INFINITY
        } else {
            haversine_km(center.0, center.1, lat, lon)
        };
        if let Some((d, data)) = best
            && d <= street
        {
            let mut data = data.clone();
            housenumber = data.remove("raw").map(|v| crate::doc::py_str(&v));
            doc.insert("type".into(), Value::String("housenumber".into()));
            doc.extend(data);
        }
        if only_housenumber && housenumber.is_none() {
            return None;
        }
    }
    let labels = profile.make_labels(&doc, housenumber.as_deref());
    let mut result = Scored {
        id,
        doc,
        labels,
        housenumber,
        distance_km: None,
        importance: None,
        str_distance: None,
        geo_distance: None,
    };
    let km = haversine_km(result.lat(), result.lon(), lat, lon);
    result.distance_km = Some(km);
    add_score(&mut result.geo_distance, km_to_score(km), 0.1);
    Some(result)
}

/// Between 0 and 0.1: 0.1 close to 0 km, 0 at 100 km and beyond.
pub fn km_to_score(km: f64) -> f64 {
    if km > 100.0 {
        0.0
    } else {
        0.1 * (-(km / 50.0).powi(2)).exp()
    }
}

/// Great circle distance, with addok's 6367 km Earth radius.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1, lon1, lat2, lon2) = (
        lat1.to_radians(),
        lon1.to_radians(),
        lat2.to_radians(),
        lon2.to_radians(),
    );
    let a = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    2.0 * a.sqrt().asin() * 6367.0
}
