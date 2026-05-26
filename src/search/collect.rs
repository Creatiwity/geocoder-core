use std::collections::HashMap;

use roaring::RoaringBitmap;

use crate::index::store::Index;
use crate::types::{DocId, TokenId};

use super::fuzzy::fuzzy_lookup;
use super::query::ClassifiedQuery;

const BUCKET_MAX: usize = 100;

struct Bucket {
    candidates: Vec<(DocId, f32)>,
}

impl Bucket {
    fn is_full(&self, wanted: usize) -> bool { self.candidates.len() >= wanted }
}

/// Main collection pipeline. Returns candidate (DocId, score) pairs.
pub fn collect(
    query: &ClassifiedQuery,
    index: &Index,
    wanted: usize,
    filters: &HashMap<String, Vec<String>>,
) -> Vec<(DocId, f32)> {
    let mut bucket = Bucket { candidates: vec![] };
    let token_ids: Vec<TokenId> = query.meaningful.iter().map(|(tid, _)| *tid).collect();
    let filter_mask = build_filter_mask(filters, index);

    // Search with meaningful tokens, progressively relaxing threshold
    if !token_ids.is_empty() {
        for min_tokens in (query.threshold..=token_ids.len()).rev() {
            let subset = &token_ids[..min_tokens];
            let mut results = index.zinter_top_n(subset, BUCKET_MAX);
            if let Some(ref mask) = filter_mask {
                results.retain(|(doc_id, _)| mask.contains(*doc_id));
            }
            bucket.candidates.extend(results);
            if bucket.is_full(wanted) {
                break;
            }
        }
    }

    // Fuzzy fallback on first not-found token (runs also when meaningful is empty)
    if !bucket.is_full(wanted) {
        if let Some(first_not_found) = query.not_found.first() {
            let fuzzy_ids = fuzzy_lookup(&index.token_fst, &first_not_found.normalized, 1);
            if let Some(&best_fuzzy) = fuzzy_ids.first() {
                let mut fuzzy_token_ids = token_ids.clone();
                fuzzy_token_ids.push(best_fuzzy);
                let mut results = index.zinter_top_n(&fuzzy_token_ids, BUCKET_MAX);
                if let Some(ref mask) = filter_mask {
                    results.retain(|(doc_id, _)| mask.contains(*doc_id));
                }
                bucket.candidates.extend(results);
            }
        }
    }

    bucket.candidates
}

fn build_filter_mask(
    filters: &HashMap<String, Vec<String>>,
    index: &Index,
) -> Option<RoaringBitmap> {
    if filters.is_empty() {
        return None;
    }
    let mut mask: Option<RoaringBitmap> = None;
    for (key, values) in filters {
        // OR within same filter key
        let mut union = RoaringBitmap::new();
        for value in values {
            let filter_key = format!("{}:{}", key, value);
            if let Some(bm) = index.filter_sets.get(&filter_key) {
                union |= bm;
            }
        }
        // AND across different filter keys
        mask = Some(match mask {
            None => union,
            Some(existing) => existing & union,
        });
    }
    mask
}
