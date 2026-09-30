//! `addok.helpers.collectors`, `addok.fuzzy.fuzzy_collector` and the
//! `addok.autocomplete` collectors (addok 1.3).

use super::fuzzy::make_fuzzy;
use super::{Helper, Key};
use crate::settings::{BUCKET_MIN, COMMON_THRESHOLD, INTERSECT_LIMIT};
use crate::text::is_digit;
use crate::types::TokenId;

type Collector = fn(&mut Helper) -> bool;

const COLLECTORS: &[(&str, Collector)] = &[
    (
        "only_commons_but_geohash_try_autocomplete",
        only_commons_but_geohash_try_autocomplete,
    ),
    (
        "no_tokens_but_housenumbers_and_geohash",
        no_tokens_but_housenumbers_and_geohash,
    ),
    ("no_available_tokens_abort", no_available_tokens_abort),
    ("only_commons", only_commons),
    (
        "no_meaningful_but_common_try_autocomplete",
        no_meaningful_but_common_try_autocomplete,
    ),
    (
        "only_commons_try_autocomplete",
        only_commons_try_autocomplete,
    ),
    ("bucket_with_meaningful", bucket_with_meaningful),
    ("reduce_with_other_commons", reduce_with_other_commons),
    (
        "ensure_geohash_results_are_included_if_center_is_given",
        ensure_geohash_results_are_included_if_center_is_given,
    ),
    ("autocomplete_meaningful", autocomplete_meaningful),
    ("fuzzy", fuzzy),
    (
        "extend_results_extrapoling_relations",
        extend_results_extrapoling_relations,
    ),
    (
        "extend_results_reducing_tokens",
        extend_results_reducing_tokens,
    ),
];

pub(super) fn run(helper: &mut Helper) {
    // GEOCODER_DEBUG=1: trace the collectors on stderr, like addok's --verbose.
    let debug = std::env::var_os("GEOCODER_DEBUG").is_some();
    if debug {
        let describe = |list: &[usize]| {
            list.iter()
                .map(|&i| format!("{}({})", helper.value(i), helper.toks[i].frequency))
                .collect::<Vec<_>>()
        };
        eprintln!(
            "meaningful {:?} common {:?} not_found {:?} housenumbers {:?} threshold {}",
            describe(&helper.meaningful),
            describe(&helper.common),
            describe(&helper.not_found),
            describe(&helper.housenumbers),
            helper.should_match_threshold
        );
    }
    for (name, collector) in COLLECTORS {
        let stop = collector(helper);
        if debug {
            eprintln!(
                "  {name:<55} bucket {:>4}{}",
                helper.bucket.len(),
                if stop { "  STOP" } else { "" }
            );
        }
        if stop {
            break;
        }
    }
}

fn no_tokens_but_housenumbers_and_geohash(h: &mut Helper) -> bool {
    if h.tokens.is_empty() && !h.housenumbers.is_empty() && h.geohash_key() {
        h.new_bucket(&[Key::Geohash], BUCKET_MIN);
    }
    false
}

fn no_available_tokens_abort(h: &mut Helper) -> bool {
    h.tokens.is_empty()
}

fn only_commons(h: &mut Helper) -> bool {
    if !h.only_commons() {
        return false;
    }
    let mut keys = h.keys_of(&h.tokens.clone());
    let geohash = h.geohash_key();
    if geohash {
        keys.push(Key::Geohash);
    }
    if keys.len() == 1 || geohash {
        h.add_to_bucket(&keys, None);
    }
    if h.bucket_dry() && keys.len() > 1 {
        // Scan the less frequent token.
        let toks = &h.toks;
        h.tokens.sort_by_key(|&i| toks[i].frequency);
        let keys = h.keys_of(&h.tokens.clone());
        let first = h.toks[h.tokens[0]].clone();
        if first.frequency < INTERSECT_LIMIT {
            // Small enough: intersect.
            h.add_to_bucket(&keys, None);
        } else if !h.filters.is_empty() {
            // Large token but filters: intersect if the (combined) filter is
            // more selective than the token, else scan.
            let mut all = keys.clone();
            all.extend((0..h.filters.len()).map(Key::Filter));
            let mut combined = h.filters[0].clone();
            for f in &h.filters[1..] {
                combined &= f;
            }
            if (combined.len() as usize) < first.frequency {
                h.add_to_bucket(&all, None);
            } else {
                let ids = manual_scan(h, &all);
                h.extend_bucket(ids);
            }
        } else {
            let ids = manual_scan(h, &keys);
            h.extend_bucket(ids);
        }
    }
    false
}

/// `manual_scan.lua`: the first 501 documents of the first key that are in
/// every other key, up to `wanted`.
fn manual_scan(h: &Helper, keys: &[Key]) -> Vec<u32> {
    let Some(Key::Token(first)) = keys.first().copied() else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for doc in h.index.top(first, 501) {
        let in_all = keys[1..].iter().all(|k| match *k {
            Key::Token(t) => h.index.postings(t).find(doc).is_some(),
            Key::Filter(i) => h.filters[i].contains(doc),
            Key::Geohash => false,
        });
        if in_all {
            candidates.push(doc);
        }
        if candidates.len() == h.wanted {
            break;
        }
    }
    candidates
}

fn bucket_with_meaningful(h: &mut Helper) -> bool {
    if h.meaningful.is_empty() {
        return false;
    }
    if h.meaningful.len() == 1 && !h.common.is_empty() && h.filters.is_empty() {
        // Avoid running with too few tokens while having common terms.
        for &token in &h.common.clone() {
            if !h.in_list(&h.meaningful, token) {
                h.meaningful.push(token);
                break;
            }
        }
    }
    h.keys = h.keys_of(&h.meaningful.clone());
    let keys = h.keys.clone();
    if h.bucket_empty() {
        h.new_bucket(&keys, BUCKET_MIN);
        h.absorb_filters_into_keys();
        if h.bucket.len() == BUCKET_MIN {
            // Do not rerun if the bucket with limit 10 got fewer results.
            let keys = h.keys.clone();
            h.new_bucket(&keys, 0);
            h.absorb_filters_into_keys();
        }
        if !h.autocomplete && h.has_cream() && h.cream() < BUCKET_MIN {
            // Do not check cream before computing autocomplete when it is on.
            return true;
        }
    } else {
        h.add_to_bucket(&keys, None);
        h.absorb_filters_into_keys();
    }
    false
}

fn reduce_with_other_commons(h: &mut Helper) -> bool {
    if h.only_commons() {
        return false;
    }
    for &token in &h.common.clone() {
        // Already ordered by frequency asc.
        if !h.in_list(&h.meaningful, token) && h.bucket_overflow() {
            h.meaningful.push(token);
            h.keys = h.keys_of(&h.meaningful.clone());
            let keys = h.keys.clone();
            h.new_bucket(&keys, 0);
            h.absorb_filters_into_keys();
        }
    }
    false
}

fn ensure_geohash_results_are_included_if_center_is_given(h: &mut Helper) -> bool {
    if h.bucket_overflow() && h.geohash_key() {
        let mut keys = h.keys.clone();
        keys.push(Key::Geohash);
        let limit = h.wanted.max(10);
        h.add_to_bucket(&keys, Some(limit));
    }
    false
}

fn extend_results_reducing_tokens(h: &mut Helper) -> bool {
    if h.bucket_full() || h.has_cream() {
        return true;
    }
    if !h.bucket_dry() {
        return false;
    }
    // Only if the bucket is empty or there is margin on the threshold.
    if h.bucket_empty() || h.meaningful.len() as isize - 1 > h.should_match_threshold as isize {
        // Numbers first, then by frequency, most frequent first.
        let toks = &h.toks;
        h.meaningful.sort_by_key(|&i| {
            std::cmp::Reverse((
                if is_digit(&toks[i].token) { 2 } else { 1 },
                toks[i].frequency,
            ))
        });
        let meaningful = h.meaningful.clone();
        for &token in &meaningful {
            let Some(keys) = keys_without(h, &[token]) else {
                continue;
            };
            h.add_to_bucket(&keys, None);
            if h.bucket_overflow() {
                break;
            }
        }
        if h.bucket_empty() && meaningful.len() > 3 {
            // Still empty: remove 2 meaningful tokens.
            'pairs: for &a in &meaningful {
                for &b in &meaningful {
                    if h.value(a) != h.value(b) {
                        let Some(keys) = keys_without(h, &[a, b]) else {
                            continue;
                        };
                        h.add_to_bucket(&keys, None);
                        if h.bucket_overflow() {
                            break 'pairs;
                        }
                    }
                }
            }
        }
    }
    false
}

/// `helper.keys` minus the keys of `tokens` (first occurrence each). Python
/// raises when a key is missing; `None` then, and the caller skips.
fn keys_without(h: &Helper, tokens: &[usize]) -> Option<Vec<Key>> {
    let mut keys = h.keys.clone();
    for &t in tokens {
        let key = h.key_of(t)?;
        let pos = keys.iter().position(|k| *k == key)?;
        keys.remove(pos);
    }
    Some(keys)
}

/// Try to extract the biggest group of interlinked tokens.
fn extend_results_extrapoling_relations(h: &mut Helper) -> bool {
    if !h.bucket_dry() {
        return false;
    }
    // set(meaningful + common): unique by value.
    let mut tokens: Vec<usize> = Vec::new();
    for &i in h.meaningful.iter().chain(h.common.iter()) {
        if !h.in_list(&tokens, i) {
            tokens.push(i);
        }
    }
    // Sorted by average frequency, least frequent first (stable sort).
    let mut relations = extract_manytomany_relations(h, &tokens);
    let average = |r: &Vec<usize>| {
        r.iter().map(|&t| h.toks[t].frequency as f64).sum::<f64>() / r.len().max(1) as f64
    };
    relations.sort_by(|a, b| {
        average(a)
            .partial_cmp(&average(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for relation in relations {
        let keys = h.keys_of(&relation);
        h.add_to_bucket(&keys, None);
        if h.bucket_overflow() {
            break;
        }
    }
    false
}

fn extract_manytomany_relations(h: &Helper, tokens: &[usize]) -> Vec<Vec<usize>> {
    // _compute_onetomany_relations, keeping dict insertion order.
    let mut relations: Vec<(usize, Vec<usize>)> = Vec::new();
    fn entry(relations: &mut Vec<(usize, Vec<usize>)>, k: usize) -> usize {
        match relations.iter().position(|(t, _)| *t == k) {
            Some(p) => p,
            None => {
                relations.push((k, Vec::new()));
                relations.len() - 1
            }
        }
    }
    for &token in tokens {
        let pairs = h.toks[token]
            .id
            .map(|id| h.index.pairs(id))
            .unwrap_or_default();
        for &other in tokens {
            if other == token {
                continue;
            }
            let other_entry = entry(&mut relations, other);
            let related = relations[other_entry].1.contains(&token)
                || h.toks[other].id.is_some_and(|o| pairs.contains(o));
            if related {
                let e = entry(&mut relations, token);
                relations[e].1.push(other);
            }
        }
    }
    // _extrapolate_manytomany_relations
    let related_to = |relations: &Vec<(usize, Vec<usize>)>, o: usize, t: usize| {
        relations
            .iter()
            .find(|(k, _)| *k == o)
            .is_some_and(|(_, v)| v.contains(&t))
    };
    let mut m2m: Vec<Vec<usize>> = Vec::new();
    for (origin, others) in &relations {
        let mut relation = vec![*origin];
        for &token in others {
            if relation.iter().all(|&o| related_to(&relations, o, token)) {
                relation.push(token);
            }
        }
        // Common tokens are more likely to create false positives.
        let mut set: Vec<usize> = Vec::new();
        for t in relation {
            if !h.toks[t].is_common() && !set.contains(&t) {
                set.push(t);
            }
        }
        if set.len() > 1 {
            m2m.push(set);
        }
    }
    // _deduplicate_sets
    m2m.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut unique: Vec<Vec<usize>> = Vec::new();
    for set in m2m {
        if !unique.iter().any(|g| set.iter().all(|t| g.contains(t))) {
            unique.push(set);
        }
    }
    unique
}

// Fuzzy.

fn fuzzy(h: &mut Helper) -> bool {
    if h.fuzzy && !h.has_cream() {
        if !h.not_found.is_empty() {
            let tokens = h.not_found.clone();
            try_fuzzy(h, tokens, true, Which::NotFound);
        }
        if h.bucket_dry() && !h.has_cream() {
            let tokens = h.meaningful.clone();
            try_fuzzy(h, tokens, true, Which::Meaningful);
        }
        if h.bucket_dry() && !h.has_cream() && !h.common.is_empty() {
            let tokens = h.meaningful.clone();
            try_fuzzy(h, tokens, false, Which::Meaningful);
        }
    }
    false
}

enum Which {
    NotFound,
    Meaningful,
}

fn try_fuzzy(h: &mut Helper, mut tokens: Vec<usize>, include_common: bool, which: Which) {
    if !h.bucket_dry() || tokens.is_empty() {
        return;
    }
    // addok sorts the helper's own list in place.
    let toks = &h.toks;
    tokens.sort_by_key(|&i| std::cmp::Reverse(toks[i].token.value.chars().count()));
    match which {
        Which::NotFound => h.not_found = tokens.clone(),
        Which::Meaningful => h.meaningful = tokens.clone(),
    }
    let mut allkeys = h.keys.clone();
    if include_common {
        // Narrow as much as possible by adding unused common tokens.
        for &c in &h.common {
            if let Some(key) = h.key_of(c)
                && !h.keys.contains(&key)
            {
                allkeys.push(key);
            }
        }
    }
    for try_one in tokens {
        if h.bucket_full() {
            break;
        }
        let mut keys = allkeys.clone();
        if let Some(key) = h.key_of(try_one)
            && let Some(pos) = keys.iter().position(|k| *k == key)
        {
            keys.remove(pos);
        }
        let word = h.value(try_one).to_string();
        if is_digit(&word) {
            continue;
        }
        let neighbors = make_fuzzy(&word, h.profile.fuzzy_key_map());
        let mut fuzzy_words: Vec<TokenId> = Vec::new();
        if !keys.is_empty() {
            // Only keep words seen at least once with all the other tokens.
            // Filter keys have no pair set: the intersection is then empty.
            let pair_sets: Option<Vec<_>> = keys
                .iter()
                .map(|k| match *k {
                    Key::Token(t) => Some(h.index.pairs(t)),
                    _ => None,
                })
                .collect();
            if let Some(pair_sets) = pair_sets {
                for n in &neighbors {
                    if let Some(id) = h.index.token_id(n)
                        && pair_sets.iter().all(|p| p.contains(id))
                        && !fuzzy_words.contains(&id)
                    {
                        fuzzy_words.push(id);
                    }
                }
            }
        } else {
            for n in &neighbors {
                if let Some(id) = h.index.token_id(n)
                    && h.index.frequency(id) > 0
                {
                    fuzzy_words.push(id);
                }
            }
        }
        for id in fuzzy_words {
            if h.bucket_dry() {
                let mut k = keys.clone();
                k.push(Key::Token(id));
                h.add_to_bucket(&k, None);
            }
        }
    }
}

// Autocomplete.

fn only_commons_but_geohash_try_autocomplete(h: &mut Helper) -> bool {
    if h.geohash_key() && h.only_commons() {
        let tokens = h.tokens.clone();
        autocomplete(h, &tokens, false, true);
    }
    false
}

fn only_commons_try_autocomplete(h: &mut Helper) -> bool {
    if h.only_commons() {
        let tokens = h.tokens.clone();
        autocomplete(h, &tokens, true, false);
        if !h.bucket_empty() {
            return true;
        }
    }
    false
}

fn no_meaningful_but_common_try_autocomplete(h: &mut Helper) -> bool {
    if h.meaningful.is_empty() && !h.common.is_empty() {
        // Only common terms, try to reduce with autocomplete.
        let common = h.common.clone();
        autocomplete(h, &common, false, false);
        h.meaningful = h.common[..1].to_vec();
        if !h.pass_should_match_threshold() {
            return false;
        }
        if h.bucket_full() || h.bucket_overflow() || h.has_cream() {
            return true;
        }
    }
    false
}

fn autocomplete_meaningful(h: &mut Helper) -> bool {
    if h.bucket_overflow() || !h.autocomplete {
        return false;
    }
    let meaningful = h.meaningful.clone();
    if h.geohash_key() {
        autocomplete(h, &meaningful, false, true);
    }
    autocomplete(h, &meaningful, false, false);
    false
}

fn autocomplete(h: &mut Helper, tokens: &[usize], skip_commons: bool, use_geohash: bool) {
    let Some(last) = h.last_token else { return };
    let others: Vec<usize> = tokens
        .iter()
        .copied()
        .filter(|&i| !h.toks[i].token.is_last)
        .collect();
    let keys = h.keys_of(&others);
    // sinter of the pair sets of the other tokens and the last token's ngrams.
    let mut candidates = h.index.edge_ngram(h.value(last));
    for &i in &others {
        match h.toks[i].id {
            Some(id) => candidates &= h.index.pairs(id),
            None => candidates.clear(),
        }
    }
    if candidates.is_empty() {
        return;
    }
    // One word: candidates with the best documents first (order_by_max_score).
    // Several: the most frequent first (order_by_frequency).
    let mut ordered: Vec<(TokenId, f64)> = candidates
        .iter()
        .map(|id| {
            let weight = if tokens.len() == 1 {
                h.index
                    .top(id, 1)
                    .next()
                    .and_then(|doc| h.index.postings(id).find(doc))
                    .unwrap_or(0.0) as f64
            } else {
                h.index.frequency(id) as f64
            };
            (id, weight)
        })
        .collect();
    ordered.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let last_not_found = h.in_list(&h.not_found, last);
    for (id, _) in ordered {
        if skip_commons && h.index.frequency(id) > COMMON_THRESHOLD {
            continue;
        }
        if !h.bucket_overflow() || last_not_found {
            let mut k = keys.clone();
            k.push(Key::Token(id));
            if use_geohash && h.geohash_key() {
                k.push(Key::Geohash);
            }
            h.add_to_bucket(&k, None);
        }
    }
}
