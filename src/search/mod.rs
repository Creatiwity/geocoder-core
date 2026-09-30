//! Port of addok 1.3's `Search` helper and its default results collectors:
//!
//! 1. only_commons_but_geohash_try_autocomplete
//! 2. no_tokens_but_housenumbers_and_geohash
//! 3. no_available_tokens_abort
//! 4. only_commons
//! 5. no_meaningful_but_common_try_autocomplete
//! 6. only_commons_try_autocomplete
//! 7. bucket_with_meaningful
//! 8. reduce_with_other_commons
//! 9. ensure_geohash_results_are_included_if_center_is_given
//! 10. autocomplete_meaningful
//! 11. fuzzy
//! 12. extend_results_extrapoling_relations
//! 13. extend_results_reducing_tokens
//!
//! A collector returning `true` stops the chain.

mod collectors;
pub mod fuzzy;
pub mod results;

use std::collections::{HashMap, HashSet};

use roaring::RoaringBitmap;

use crate::index::Index;
use crate::index::builder::rank;
use crate::profile::{Profile, preprocess_query};
use crate::settings::{
    BUCKET_MAX, BUCKET_MIN, COMMON_THRESHOLD, GEOHASH_PRECISION, MATCH_THRESHOLD, MAX_MEANINGFUL,
};
use crate::text::{Token, ascii};
use crate::types::{DocId, TokenId};
use results::{Context, Scored};

/// A query token once looked up in the index.
#[derive(Debug, Clone)]
pub(crate) struct QToken {
    pub token: Token,
    pub id: Option<TokenId>,
    pub frequency: usize,
}

impl QToken {
    pub fn is_common(&self) -> bool {
        self.frequency > COMMON_THRESHOLD
    }
}

/// A Redis key taking part in an intersection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Token(TokenId),
    Geohash,
    Filter(usize),
}

pub(crate) struct Helper<'a> {
    pub index: &'a Index,
    pub profile: &'a dyn Profile,
    pub query: String,
    pub wanted: usize,
    pub autocomplete: bool,
    pub fuzzy: bool,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub filters: Vec<RoaringBitmap>,
    pub check_housenumber: bool,
    pub only_housenumber: bool,
    /// All query tokens; the lists below index into it.
    pub toks: Vec<QToken>,
    pub tokens: Vec<usize>,
    pub meaningful: Vec<usize>,
    pub common: Vec<usize>,
    pub not_found: Vec<usize>,
    pub housenumbers: Vec<usize>,
    pub last_token: Option<usize>,
    pub should_match_threshold: usize,
    pub keys: Vec<Key>,
    pub matched_keys: HashSet<TokenId>,
    pub bucket: Vec<DocId>,
    bucket_set: HashSet<DocId>,
    pub results: HashMap<DocId, Scored>,
    results_order: Vec<DocId>,
    geohash: Option<Option<RoaringBitmap>>,
}

pub(crate) struct Query<'q> {
    pub text: &'q str,
    pub limit: usize,
    pub autocomplete: bool,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// (key, values) pairs already restricted to the profile's filters.
    pub filters: Vec<(String, Vec<String>)>,
}

impl<'a> Helper<'a> {
    pub fn new(index: &'a Index, profile: &'a dyn Profile, q: &Query) -> Self {
        let filters = q
            .filters
            .iter()
            .map(|(key, values)| {
                let mut union = RoaringBitmap::new();
                for value in values {
                    union |= index.filter(key, value);
                }
                union
            })
            .collect();
        let type_filter = q.filters.iter().find(|(k, _)| k == "type").map(|(_, v)| v);
        let check_housenumber =
            type_filter.is_none_or(|values| values.iter().any(|v| v == "housenumber"));
        let only_housenumber =
            type_filter.is_some_and(|values| values.iter().all(|v| v == "housenumber"));
        Self {
            index,
            profile,
            query: ascii(q.text.trim()),
            wanted: q.limit,
            autocomplete: q.autocomplete,
            fuzzy: true,
            lat: q.lat,
            lon: q.lon,
            filters,
            check_housenumber,
            only_housenumber,
            toks: Vec::new(),
            tokens: Vec::new(),
            meaningful: Vec::new(),
            common: Vec::new(),
            not_found: Vec::new(),
            housenumbers: Vec::new(),
            last_token: None,
            should_match_threshold: 0,
            keys: Vec::new(),
            matched_keys: HashSet::new(),
            bucket: Vec::new(),
            bucket_set: HashSet::new(),
            results: HashMap::new(),
            results_order: Vec::new(),
            geohash: None,
        }
    }

    /// SEARCH_PREPROCESSORS: tokenize, search_tokens, select_tokens,
    /// set_should_match_threshold.
    pub fn preprocess(&mut self) {
        let mut tokens = preprocess_query(self.profile, &self.query);
        if let Some(last) = tokens.last_mut() {
            last.is_last = true;
        }
        // Stable sort, longest first.
        tokens.sort_by_key(|t| std::cmp::Reverse(t.value.chars().count()));
        self.toks = tokens
            .into_iter()
            .map(|token| {
                let id = self.index.token_id(&token.value);
                let frequency = id.map(|id| self.index.frequency(id)).unwrap_or(0);
                QToken {
                    token,
                    id,
                    frequency,
                }
            })
            .collect();
        self.last_token = self.toks.iter().position(|t| t.token.is_last);

        for i in 0..self.toks.len() {
            let t = &self.toks[i];
            if t.token.is_housenumber() {
                self.housenumbers.push(i);
                continue;
            } else if t.is_common() {
                self.common.push(i);
            } else if t.id.is_some() {
                self.meaningful.push(i);
            } else {
                self.not_found.push(i);
            }
            self.tokens.push(i);
        }
        let toks = &self.toks;
        self.common.sort_by_key(|&i| toks[i].frequency);
        self.meaningful.sort_by_key(|&i| toks[i].frequency);
        if self.meaningful.len() > MAX_MEANINGFUL {
            let extra = self.meaningful.split_off(MAX_MEANINGFUL);
            self.common.extend(extra);
        }
        self.should_match_threshold = (2.0 / 3.0 * self.tokens.len() as f64).ceil() as usize;
    }

    pub fn value(&self, i: usize) -> &str {
        &self.toks[i].token.value
    }

    /// Python's `token in some_list`, comparing by value.
    pub fn in_list(&self, list: &[usize], i: usize) -> bool {
        list.iter().any(|&j| self.value(j) == self.value(i))
    }

    pub fn key_of(&self, i: usize) -> Option<Key> {
        self.toks[i].id.map(Key::Token)
    }

    pub fn keys_of(&self, list: &[usize]) -> Vec<Key> {
        list.iter().filter_map(|&i| self.key_of(i)).collect()
    }

    /// Housenumber tokens joined in query order, like `_match_housenumber`.
    pub fn housenumber_key(&self) -> String {
        let mut hns: Vec<&QToken> = self.housenumbers.iter().map(|&i| &self.toks[i]).collect();
        hns.sort_by(|a, b| a.token.positions.cmp(&b.token.positions));
        hns.iter().map(|t| t.token.value.as_str()).collect()
    }

    /// Nearby documents, the center cell and its 8 neighbours.
    pub fn geohash_key(&mut self) -> bool {
        if self.geohash.is_none() {
            let computed = match (self.lat, self.lon) {
                // Python truthiness: a 0.0 coordinate counts as missing.
                (Some(lat), Some(lon)) if lat != 0.0 && lon != 0.0 => {
                    let mut union = RoaringBitmap::new();
                    for cell in expand(lat, lon) {
                        union |= self.index.geohash(&cell);
                    }
                    Some(union).filter(|u| !u.is_empty())
                }
                _ => None,
            };
            self.geohash = Some(computed);
        }
        matches!(self.geohash, Some(Some(_)))
    }

    // Bucket state.

    pub fn bucket_full(&self) -> bool {
        self.bucket.len() >= self.wanted && self.bucket.len() < BUCKET_MAX
    }

    pub fn bucket_overflow(&self) -> bool {
        self.bucket.len() >= BUCKET_MAX
    }

    pub fn bucket_dry(&self) -> bool {
        self.bucket.len() < self.wanted
    }

    pub fn bucket_empty(&self) -> bool {
        self.bucket.is_empty()
    }

    pub fn only_commons(&self) -> bool {
        !self.tokens.is_empty() && self.tokens.len() == self.common.len()
    }

    pub fn pass_should_match_threshold(&self) -> bool {
        self.matched_keys.len() >= self.should_match_threshold
    }

    pub fn extend_bucket(&mut self, ids: Vec<DocId>) {
        for id in ids {
            if self.bucket_set.insert(id) {
                self.bucket.push(id);
            }
        }
    }

    pub fn add_to_bucket(&mut self, keys: &[Key], limit: Option<usize>) {
        self.matched_keys
            .extend(keys.iter().filter_map(|k| match k {
                Key::Token(t) => Some(*t),
                _ => None,
            }));
        // `limit or (BUCKET_MAX - len(bucket))`, possibly not positive.
        let limit = match limit {
            Some(l) if l > 0 => l as isize,
            _ => BUCKET_MAX as isize - self.bucket.len() as isize,
        };
        let ids = self.intersect(keys, limit);
        self.extend_bucket(ids);
    }

    pub fn new_bucket(&mut self, keys: &[Key], limit: usize) {
        self.matched_keys = keys
            .iter()
            .filter_map(|k| match k {
                Key::Token(t) => Some(*t),
                _ => None,
            })
            .collect();
        let ids = self.intersect(keys, limit as isize);
        self.bucket.clear();
        self.bucket_set.clear();
        self.extend_bucket(ids);
    }

    /// addok's `intersect` extends the list it receives with the filters.
    /// When that list is `helper.keys` itself, the filters stay in it.
    pub fn absorb_filters_into_keys(&mut self) {
        for i in 0..self.filters.len() {
            self.keys.push(Key::Filter(i));
        }
    }

    /// `Search.intersect`: top documents of the keys (plus filters).
    pub fn intersect(&mut self, keys: &[Key], limit: isize) -> Vec<DocId> {
        let limit = if limit > 0 {
            limit as usize
        } else {
            self.wanted.max(BUCKET_MAX)
        };
        if keys.is_empty() {
            return Vec::new();
        }
        let mut all: Vec<Key> = Vec::with_capacity(keys.len() + self.filters.len());
        for key in keys
            .iter()
            .copied()
            .chain((0..self.filters.len()).map(Key::Filter))
        {
            if !all.contains(&key) {
                all.push(key);
            }
        }
        if all.len() == 1 {
            return match all[0] {
                Key::Token(t) => self.index.top(t, limit).collect(),
                Key::Geohash => self.geohash_set().iter().collect(),
                Key::Filter(i) => self.filters[i].iter().collect(),
            };
        }
        self.zinter(&all, limit)
    }

    fn geohash_set(&mut self) -> RoaringBitmap {
        self.geohash_key();
        self.geohash.clone().flatten().unwrap_or_default()
    }

    /// `ZINTERSTORE` (scores summed, plain sets count 1) then `ZREVRANGE`.
    fn zinter(&mut self, keys: &[Key], limit: usize) -> Vec<DocId> {
        let mut tokens: Vec<TokenId> = Vec::new();
        let mut sets: Vec<RoaringBitmap> = Vec::new();
        for key in keys {
            match *key {
                Key::Token(t) => tokens.push(t),
                Key::Geohash => sets.push(self.geohash_set()),
                Key::Filter(i) => sets.push(self.filters[i].clone()),
            }
        }
        let set_score = sets.len() as f64;
        let mut scored: Vec<(DocId, f64)> = Vec::new();
        if tokens.is_empty() {
            let mut iter = sets.into_iter();
            let mut inter = iter.next().unwrap_or_default();
            for s in iter {
                inter &= s;
            }
            scored.extend(inter.iter().map(|d| (d, set_score)));
        } else {
            let index = self.index;
            tokens.sort_by_key(|&t| index.frequency(t));
            let driver = index.postings(tokens[0]);
            let others: Vec<_> = tokens[1..].iter().map(|&t| index.postings(t)).collect();
            'docs: for (doc, boost) in driver.iter() {
                let mut total = boost as f64;
                for postings in &others {
                    match postings.find(doc) {
                        Some(b) => total += b as f64,
                        None => continue 'docs,
                    }
                }
                if sets.iter().any(|s| !s.contains(doc)) {
                    continue;
                }
                scored.push((doc, total + set_score));
            }
        }
        top(scored, limit)
    }

    /// Score the documents of the bucket not scored yet.
    pub fn convert(&mut self) {
        let housenumber = self.housenumber_key();
        let ctx = Context {
            profile: self.profile,
            query: &self.query,
            autocomplete: self.autocomplete,
            lat: self.lat,
            lon: self.lon,
            check_housenumber: self.check_housenumber,
            only_housenumber: self.only_housenumber,
            housenumber: &housenumber,
        };
        let pending: Vec<DocId> = self
            .bucket
            .iter()
            .copied()
            .filter(|id| !self.results.contains_key(id))
            .collect();
        for id in pending {
            // A rejected document is not kept, and would be tried again.
            if let Some(scored) = self
                .index
                .document(id)
                .and_then(|doc| results::process(&ctx, id, doc))
            {
                self.results.insert(id, scored);
                self.results_order.push(id);
            }
        }
    }

    pub fn cream(&self) -> usize {
        self.results
            .values()
            .filter(|r| r.str_distance() >= MATCH_THRESHOLD)
            .count()
    }

    pub fn has_cream(&mut self) -> bool {
        if self.bucket_empty() || self.bucket_overflow() || self.bucket.len() > BUCKET_MIN {
            return false;
        }
        self.convert();
        self.cream() > 0
    }

    /// Run the collectors then rank.
    pub fn run(mut self) -> Vec<Scored> {
        self.preprocess();
        collectors::run(&mut self);
        self.render()
    }

    fn render(mut self) -> Vec<Scored> {
        self.convert();
        let mut ranked: Vec<Scored> = self
            .results_order
            .iter()
            .filter_map(|id| self.results.remove(id))
            .collect();
        // Stable, like Python's sort.
        ranked.sort_by(|a, b| {
            b.score()
                .partial_cmp(&a.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(self.wanted);
        let min_score = self.profile.min_score();
        ranked.retain(|r| r.score() >= min_score);
        ranked
    }
}

/// Best `limit` documents by summed score, Redis order.
fn top(mut scored: Vec<(DocId, f64)>, limit: usize) -> Vec<DocId> {
    let cmp = |a: &(DocId, f64), b: &(DocId, f64)| rank(a.1, a.0, b.1, b.0);
    if scored.len() > limit {
        scored.select_nth_unstable_by(limit, cmp);
        scored.truncate(limit);
    }
    scored.sort_unstable_by(cmp);
    scored.into_iter().map(|(d, _)| d).collect()
}

/// `geohash.expand`: the cell and its 8 neighbours.
pub(crate) fn expand(lat: f64, lon: f64) -> Vec<String> {
    let Ok(cell) = geohash::encode(geohash::Coord { x: lon, y: lat }, GEOHASH_PRECISION) else {
        return Vec::new();
    };
    expand_cell(&cell)
}

pub(crate) fn expand_cell(cell: &str) -> Vec<String> {
    let mut cells = vec![cell.to_string()];
    if let Ok(n) = geohash::neighbors(cell) {
        cells.extend([n.n, n.ne, n.e, n.se, n.s, n.sw, n.w, n.nw]);
    }
    cells
}
