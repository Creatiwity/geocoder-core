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
    fn overflows(&self) -> bool { self.candidates.len() >= BUCKET_MAX }
}

/// Main collection pipeline. Returns candidate (DocId, score) pairs.
///
/// Mirrors addok's RESULTS_COLLECTORS chain:
///   only_commons → bucket_with_meaningful → reduce_with_other_commons →
///   extend_results_reducing_tokens → extend_results_extrapoling_relations
pub fn collect(
    query: &ClassifiedQuery,
    index: &Index,
    wanted: usize,
    filters: &HashMap<String, Vec<String>>,
) -> Vec<(DocId, f32)> {
    let mut bucket = Bucket { candidates: vec![] };
    let token_ids: Vec<TokenId> = query.meaningful.iter().map(|(tid, _)| *tid).collect();
    let mut common_ids: Vec<TokenId> = query.common.iter().map(|(tid, _)| *tid).collect();
    let filter_mask = build_filter_mask(filters, index);

    // Sort common tokens by frequency ascending (most selective first).
    // Mirrors addok's `helper.common.sort(key=lambda x: x.frequency)`.
    common_ids.sort_unstable_by_key(|&tid| index.token_frequency(tid));

    let common_fallback = token_ids.is_empty() && !common_ids.is_empty();

    if common_fallback {
        // only_commons: all tokens are common (e.g. a city name like "bordeaux").
        // Fetch top docs from the intersection — importance scores ensure cities rank first.
        let mut results = index.zinter_top_n(&common_ids, BUCKET_MAX);
        if let Some(ref mask) = filter_mask {
            results.retain(|(doc_id, _)| mask.contains(*doc_id));
        }
        bucket.candidates.extend(results);
    } else if !token_ids.is_empty() {
        // bucket_with_meaningful: when only 1 meaningful token and commons exist (no explicit
        // filters), extend the search with the least-frequent common token so we get
        // geographically-narrow results from the start.
        // Mirrors addok's `bucket_with_meaningful` single-token special case.
        let search_ids: Vec<TokenId> =
            if token_ids.len() == 1 && !common_ids.is_empty() && filters.is_empty() {
                let mut v = token_ids.clone();
                v.push(common_ids[0]); // common_ids[0] = least frequent after sort
                v
            } else {
                token_ids.clone()
            };

        // Phase 1: try with all search_ids, then relax down to the 2/3 threshold.
        for min_tokens in (query.threshold..=search_ids.len()).rev() {
            let subset = &search_ids[..min_tokens];
            let mut results = index.zinter_top_n(subset, BUCKET_MAX);
            if let Some(ref mask) = filter_mask {
                results.retain(|(doc_id, _)| mask.contains(*doc_id));
            }
            bucket.candidates.extend(results);
            if bucket.is_full(wanted) {
                break;
            }
        }

        // Phase 2: extend below threshold (meaningful tokens only) when still dry.
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

        // reduce_with_other_commons: while the bucket overflows, add common tokens one by one
        // to the intersection and REPLACE the bucket with the narrowed result.
        // e.g. [paix] → 100+ results; then [paix, pari] → 10 Paris results.
        // Mirrors addok's `reduce_with_other_commons` collector.
        let mut accumulated = search_ids.clone();
        for &common_tid in &common_ids {
            if accumulated.contains(&common_tid) {
                continue; // already used in this intersection
            }
            if !bucket.overflows() {
                break; // no longer overflowing, stop narrowing
            }
            accumulated.push(common_tid);
            let mut narrowed = index.zinter_top_n(&accumulated, BUCKET_MAX);
            if let Some(ref mask) = filter_mask {
                narrowed.retain(|(doc_id, _)| mask.contains(*doc_id));
            }
            if !narrowed.is_empty() {
                bucket.candidates = narrowed; // REPLACE (not extend)
            }
        }
    }

    // Fuzzy fallback on first not-found token when bucket is still dry.
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

    // Extrapolate via token pair co-occurrence when bucket is still empty.
    // Mirrors addok's `extend_results_extrapoling_relations` collector.
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

    // Deduplicate: a doc_id can appear across multiple relaxation passes; keep max score.
    let mut seen: HashMap<DocId, f32> = HashMap::new();
    for (id, score) in bucket.candidates {
        seen.entry(id).and_modify(|s| *s = s.max(score)).or_insert(score);
    }
    seen.into_iter().collect()
}

pub fn build_filter_mask(
    filters: &HashMap<String, Vec<String>>,
    index: &Index,
) -> Option<RoaringBitmap> {
    if filters.is_empty() {
        return None;
    }
    let mut mask: Option<RoaringBitmap> = None;
    for (key, values) in filters {
        let mut union = RoaringBitmap::new();
        for value in values {
            let filter_key = format!("{}:{}", key, value);
            if let Some(bm) = index.filter_sets.get(&filter_key) {
                union |= bm;
            }
        }
        mask = Some(match mask {
            None => union,
            Some(existing) => existing & union,
        });
    }
    mask
}
