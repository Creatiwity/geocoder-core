pub mod collect;
pub mod fuzzy;
pub mod housenumber;
pub mod query;
pub mod score;

use std::collections::HashSet;

use crate::index::store::Index;
use crate::types::{DocId, ReverseOpts, SearchOpts, SearchResult};

use score::{haversine_km, Scorer};

const MIN_SCORE: f32 = 0.1;
const DEFAULT_LIMIT: usize = 5;
const GEOHASH_PRECISION: usize = 7;
/// Number of neighbor rings to expand when reverse geocoding has insufficient results.
const MAX_REVERSE_RINGS: usize = 3;

const MIN_QUERY_LEN: usize = 2;
const MAX_QUERY_LEN: usize = 200;

pub fn search(query: &str, opts: &SearchOpts, index: &Index) -> Vec<SearchResult> {
    let trimmed = query.trim();
    if trimmed.len() < MIN_QUERY_LEN || trimmed.len() > MAX_QUERY_LEN {
        return vec![];
    }

    let limit = opts.limit.unwrap_or(DEFAULT_LIMIT);
    let classified = query::preprocess(trimmed, index);

    let candidates = collect::collect(&classified, index, limit, &opts.filters);

    // Include not_found tokens so fuzzy-matched results still get a text similarity score
    let query_tokens: Vec<&str> = classified
        .meaningful
        .iter()
        .map(|(_, t)| t.normalized.as_str())
        .chain(classified.not_found.iter().map(|t| t.normalized.as_str()))
        .collect();
    let scorer = Scorer { query: trimmed, query_tokens: &query_tokens, lat: opts.lat, lon: opts.lon };

    let hn_query = classified.housenumbers.first().map(|t| t.text.as_str());

    let mut results: Vec<SearchResult> = candidates
        .into_iter()
        .filter_map(|(doc_id, _)| {
            let doc = index.get_document(doc_id)?;

            // Try housenumber matching — overrides coordinates if found
            let (lat, lon, matched_hn) = match hn_query
                .and_then(|q| housenumber::match_housenumber(q, &doc))
            {
                Some((hn_lat, hn_lon, hn_num)) => (hn_lat, hn_lon, Some(hn_num)),
                None => {
                    let lat = doc.get("lat")?.as_f64()?;
                    let lon = doc.get("lon")?.as_f64()?;
                    (lat, lon, None)
                }
            };

            let score = scorer.score(&doc, lat, lon);
            let distance_km =
                opts.lat.zip(opts.lon).map(|(la, lo)| haversine_km(la, lo, lat, lon));
            Some(SearchResult { doc, score, matched_housenumber: matched_hn, distance_km })
        })
        .filter(|r| r.score >= MIN_SCORE)
        .collect();

    results.sort_unstable_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results.truncate(limit);
    results
}

pub fn reverse(lat: f64, lon: f64, opts: &ReverseOpts, index: &Index) -> Vec<SearchResult> {
    let limit = opts.limit.unwrap_or(DEFAULT_LIMIT);
    let coord = geohash::Coord { x: lon, y: lat };
    let center = geohash::encode(coord, GEOHASH_PRECISION).unwrap_or_default();

    // Expand neighbor rings at constant precision until we have enough candidates.
    // Ring 0: center (1 cell)
    // Ring 1: center + 8 neighbors (9 cells)
    // Ring N: all cells bordering ring N-1
    let mut searched: HashSet<String> = HashSet::new();
    let mut ring: Vec<String> = vec![center];
    let mut candidates: HashSet<DocId> = HashSet::new();

    for _ in 0..=MAX_REVERSE_RINGS {
        let mut next_ring: Vec<String> = Vec::new();

        for cell in &ring {
            if !searched.insert(cell.clone()) {
                continue;
            }
            if let Some(bm) = index.geohash_sets.get(cell) {
                for doc_id in bm.iter() {
                    candidates.insert(doc_id);
                }
            }
            if let Ok(n) = geohash::neighbors(cell) {
                next_ring.extend_from_slice(&[n.n, n.ne, n.e, n.se, n.s, n.sw, n.w, n.nw]);
            }
        }

        if candidates.len() >= limit {
            break;
        }
        ring = next_ring;
    }

    let mut results: Vec<SearchResult> = candidates
        .into_iter()
        .filter_map(|doc_id| {
            let doc = index.get_document(doc_id)?;
            let dlat = doc.get("lat")?.as_f64()?;
            let dlon = doc.get("lon")?.as_f64()?;
            let km = haversine_km(lat, lon, dlat, dlon);
            Some(SearchResult {
                score: score::km_to_score(km),
                doc,
                matched_housenumber: None,
                distance_km: Some(km),
            })
        })
        .collect();

    results.sort_unstable_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    results.truncate(limit);
    results
}
