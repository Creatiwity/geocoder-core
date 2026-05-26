pub mod collect;
pub mod fuzzy;
pub mod housenumber;
pub mod query;
pub mod score;

use std::collections::HashSet;

use crate::index::store::Index;
use crate::types::{DocId, ReverseOpts, SearchOpts, SearchResult};

use crate::text::{synonyms_for, tokenize};
use crate::types::TokenId;
use collect::build_filter_mask;
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

/// Autocomplete search: treats the last word as a partial prefix and uses ngram_sets
/// to find matching tokens, combining with fully-resolved earlier words.
pub fn autocomplete(query: &str, opts: &SearchOpts, index: &Index) -> Vec<SearchResult> {
    let trimmed = query.trim();
    if trimmed.len() < MIN_QUERY_LEN || trimmed.len() > MAX_QUERY_LEN {
        return vec![];
    }

    let limit = opts.limit.unwrap_or(DEFAULT_LIMIT);
    let tokens = tokenize(trimmed);
    let word_tokens: Vec<_> = tokens.iter().filter(|t| !t.is_housenumber).collect();

    if word_tokens.is_empty() {
        return search(trimmed, opts, index);
    }

    let (prefix_words, last_word) = word_tokens.split_at(word_tokens.len() - 1);

    // Resolve complete words to token IDs
    let prefix_ids: Vec<TokenId> = prefix_words
        .iter()
        .filter_map(|t| {
            if let Some(id) = index.token_fst.get(&t.normalized) {
                return Some(id as TokenId);
            }
            synonyms_for(&t.normalized)
                .iter()
                .find_map(|s| index.token_fst.get(*s).map(|id| id as TokenId))
        })
        .collect();

    // Use ngram_sets to find token IDs matching the partial last word
    let partial = &last_word[0].normalized;
    let partial_token_ids: Vec<TokenId> = index
        .ngram_sets
        .get(partial.as_str())
        .map(|bm| bm.iter().collect())
        .unwrap_or_default();

    let filter_mask = build_filter_mask(&opts.filters, index);
    const AC_BUCKET: usize = 200;
    let mut candidates: Vec<(DocId, f32)> = Vec::new();

    for partial_tid in &partial_token_ids {
        let mut try_ids = prefix_ids.clone();
        try_ids.push(*partial_tid);
        let mut results = index.zinter_top_n(&try_ids, AC_BUCKET);
        if let Some(ref mask) = filter_mask {
            results.retain(|(doc_id, _)| mask.contains(*doc_id));
        }
        candidates.extend(results);
        if candidates.len() >= limit * 20 {
            break;
        }
    }

    // Dedup, then score and rank
    candidates.sort_unstable_by_key(|(id, _)| *id);
    candidates.dedup_by_key(|(id, _)| *id);

    let all_normalized: Vec<&str> =
        word_tokens.iter().map(|t| t.normalized.as_str()).collect();
    let hn_query = tokens.iter().find(|t| t.is_housenumber).map(|t| t.text.as_str());
    let scorer = Scorer {
        query: trimmed,
        query_tokens: &all_normalized,
        lat: opts.lat,
        lon: opts.lon,
    };

    let mut results: Vec<SearchResult> = candidates
        .into_iter()
        .filter_map(|(doc_id, _)| {
            let doc = index.get_document(doc_id)?;
            let (lat, lon, matched_hn) =
                match hn_query.and_then(|q| housenumber::match_housenumber(q, &doc)) {
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
