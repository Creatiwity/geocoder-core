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

    // Phase 1: progressively relax from all tokens down to the 2/3 threshold
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

        // Phase 2: extend below threshold down to 1 token when still insufficient
        if !bucket.is_full(wanted) && query.threshold > 1 {
            for min_tokens in (1..query.threshold).rev() {
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
    }

    // Use common tokens to narrow results when we have more than needed
    if bucket.candidates.len() > wanted && !query.common.is_empty() {
        let common_ids: Vec<TokenId> = query.common.iter().map(|(tid, _)| *tid).collect();
        let common_mask = build_common_mask(&common_ids, index);
        if let Some(ref mask) = common_mask {
            let narrowed: Vec<_> =
                bucket.candidates.iter().filter(|(id, _)| mask.contains(*id)).cloned().collect();
            if narrowed.len() >= wanted {
                bucket.candidates = narrowed;
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

    // Phase 4: extrapolate via token pair relations when bucket is still empty
    if bucket.candidates.is_empty() && !token_ids.is_empty() {
        let mut related: Vec<TokenId> = token_ids
            .iter()
            .flat_map(|tid| {
                index
                    .pair_sets
                    .get(tid)
                    .map(|bm| bm.iter().collect::<Vec<_>>())
                    .unwrap_or_default()
            })
            .collect();
        related.sort_unstable();
        related.dedup();

        for related_tid in related {
            let mut try_ids = token_ids.clone();
            try_ids.push(related_tid);
            let mut results = index.zinter_top_n(&try_ids, BUCKET_MAX);
            if let Some(ref mask) = filter_mask {
                results.retain(|(doc_id, _)| mask.contains(*doc_id));
            }
            bucket.candidates.extend(results);
            if bucket.is_full(wanted) {
                break;
            }
        }
    }

    bucket.candidates
}

/// Build a union bitmap of all documents that contain at least one common token.
/// Used as a secondary filter to narrow overflow results.
fn build_common_mask(common_ids: &[TokenId], index: &Index) -> Option<RoaringBitmap> {
    let mut union = RoaringBitmap::new();
    for &tid in common_ids {
        if let Some(entries) = index.token_scores.get(tid as usize) {
            for (doc_id, _) in entries {
                union.insert(*doc_id);
            }
        }
    }
    if union.is_empty() { None } else { Some(union) }
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
