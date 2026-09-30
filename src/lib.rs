//! Address geocoding, ported from addok.
//!
//! - [`profile`]: country specific text processing (default and `fr`).
//! - [`readers`]: data sources converted to addok's document format.
//! - [`IndexBuilder`] / [`Index`]: the on-disk index.
//! - [`Geocoder`]: search and reverse geocoding.

pub mod doc;
mod error;
pub mod index;
pub mod profile;
pub mod readers;
pub mod search;
pub mod settings;
pub mod text;
mod types;

use std::collections::HashSet;
use std::sync::Arc;

pub use doc::Document;
pub use error::GeocoderError;
pub use index::{Index, IndexBuilder, Meta};
pub use profile::Profile;
pub use types::{ReverseOpts, SearchOpts, SearchResult};

use search::results::{Scored, process_reverse};
use search::{Helper, Query, expand, expand_cell};
use settings::{DEFAULT_LIMIT, QUERY_MAX_LENGTH};

/// An index opened with the profile it was built with.
pub struct Geocoder {
    index: Index,
    profile: Arc<dyn Profile>,
}

impl Geocoder {
    pub fn open(path: &std::path::Path) -> Result<Self, GeocoderError> {
        let index = Index::open(path)?;
        let profile = profile::by_name(&index.meta.profile).ok_or(GeocoderError::UnknownProfile)?;
        if profile.version() != index.meta.profile_version {
            return Err(GeocoderError::ProfileVersion {
                profile: index.meta.profile.clone(),
                index: index.meta.profile_version,
                code: profile.version(),
            });
        }
        Ok(Self { index, profile })
    }

    pub fn meta(&self) -> &Meta {
        &self.index.meta
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn profile(&self) -> &dyn Profile {
        self.profile.as_ref()
    }

    /// Forward geocoding, like addok's `/search`.
    pub fn search(&self, query: &str, opts: SearchOpts) -> Vec<SearchResult> {
        if text::ascii(query.trim()).chars().count() > QUERY_MAX_LENGTH {
            return Vec::new();
        }
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT);
        let q = Query {
            text: query,
            // addok may be asked for more results than returned (see
            // `Profile::min_limit`): the thresholds of its collectors depend
            // on it.
            limit: limit.max(self.profile.min_limit()),
            autocomplete: opts.autocomplete.unwrap_or(true),
            lat: opts.lat,
            lon: opts.lon,
            filters: self.filters(&opts.filters),
        };
        let mut results = Helper::new(&self.index, self.profile.as_ref(), &q).run();
        results.truncate(limit);
        results.into_iter().map(to_result).collect()
    }

    /// Same as [`Geocoder::search`] with autocomplete forced on.
    pub fn autocomplete(&self, query: &str, opts: SearchOpts) -> Vec<SearchResult> {
        self.search(
            query,
            SearchOpts {
                autocomplete: Some(true),
                ..opts
            },
        )
    }

    /// Reverse geocoding, like addok's `/reverse`.
    pub fn reverse(&self, lat: f64, lon: f64, opts: ReverseOpts) -> Vec<SearchResult> {
        let limit = opts.limit.unwrap_or(1);
        let filters = self.filters(&opts.filters);
        let type_filter = filters.iter().find(|(k, _)| k == "type").map(|(_, v)| v);
        let check_housenumber =
            type_filter.is_none_or(|values| values.iter().any(|v| v == "housenumber"));
        let only_housenumber =
            type_filter.is_some_and(|values| values.iter().all(|v| v == "housenumber"));
        let masks: Vec<roaring::RoaringBitmap> = filters
            .iter()
            .map(|(key, values)| {
                values
                    .iter()
                    .map(|v| self.index.filter(key, v))
                    .fold(Default::default(), |a, b| a | b)
            })
            .collect();

        let mut fetched: Vec<String> = Vec::new();
        let mut keys: HashSet<u32> = HashSet::new();
        let fetch = |cells: &[String], keys: &mut HashSet<u32>, fetched: &mut Vec<String>| {
            for cell in cells {
                let mut docs = self.index.geohash(cell);
                for mask in &masks {
                    docs &= mask;
                }
                keys.extend(docs.iter());
                fetched.push(cell.clone());
            }
        };
        let grow = |cells: &[String], fetched: &[String]| -> Vec<String> {
            cells
                .iter()
                .flat_map(|c| expand_cell(c))
                .filter(|n| !fetched.contains(n))
                .collect()
        };
        let first = expand(lat, lon);
        fetch(&first, &mut keys, &mut fetched);
        if keys.is_empty() {
            let next = grow(&first, &fetched);
            fetch(&next, &mut keys, &mut fetched);
        }

        let mut results: Vec<Scored> = keys
            .into_iter()
            .filter_map(|id| {
                let doc = self.index.document(id)?;
                process_reverse(
                    self.profile.as_ref(),
                    id,
                    doc,
                    lat,
                    lon,
                    check_housenumber,
                    only_housenumber,
                )
            })
            .collect();
        results.sort_by(|a, b| {
            b.score()
                .partial_cmp(&a.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results.into_iter().map(to_result).collect()
    }

    /// Keep the filters the profile knows, their values trimmed, deduplicated,
    /// limited to `MAX_FILTER_VALUES` and sorted (addok 1.3's
    /// `_normalize_filter_values`). Several values are OR-ed.
    fn filters(
        &self,
        filters: &std::collections::HashMap<String, Vec<String>>,
    ) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = self
            .profile
            .filters()
            .iter()
            .filter_map(|&name| {
                let mut values: Vec<String> = Vec::new();
                for v in filters.get(name)? {
                    let v = v.trim().to_string();
                    if !v.is_empty() && !values.contains(&v) {
                        values.push(v);
                    }
                }
                values.truncate(settings::MAX_FILTER_VALUES);
                values.sort();
                (!values.is_empty()).then(|| (name.to_string(), values))
            })
            .collect();
        out.sort();
        out
    }
}

fn to_result(r: Scored) -> SearchResult {
    SearchResult {
        label: r.label(),
        score: r.score() as f32,
        housenumber: r.housenumber.clone(),
        lat: r.lat(),
        lon: r.lon(),
        distance_km: r.distance_km,
        doc: serde_json::Value::Object(r.doc),
    }
}
