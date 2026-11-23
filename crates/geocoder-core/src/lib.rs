//! Address geocoding, ported from addok.
//!
//! - [`profile`]: country specific text processing (`default`, `fr`, `lu`, `be`).
//! - [`readers`]: data sources converted to addok's document format.
//! - [`IndexBuilder`] / [`Index`]: the on-disk index.
//! - [`Geocoder`]: search and reverse geocoding.
//! - `signed` (feature `signed`): signed manifests of prebuilt indexes.

pub mod doc;
mod error;
pub mod index;
pub mod profile;
pub mod readers;
pub mod search;
pub mod settings;
#[cfg(feature = "signed")]
pub mod signed;
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
        let span = tracing::info_span!(
            "geocoder.open",
            sentry.op = "geocoder.open",
            path = %path.display(),
            profile = tracing::field::Empty,
            documents = tracing::field::Empty,
        );
        let _entered = span.enter();
        let index = Index::open(path)?;
        span.record("profile", index.meta.profile.as_str());
        span.record("documents", index.meta.documents);
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
    ///
    /// Runs in a `geocoder.search` span (INFO) describing the query's shape,
    /// never its text: token counts, the collector that settled it, the
    /// documents read and scored, the results.
    pub fn search(&self, query: &str, opts: SearchOpts) -> Vec<SearchResult> {
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT);
        let autocomplete = opts.autocomplete.unwrap_or(true);
        let span = tracing::info_span!(
            "geocoder.search",
            sentry.op = "geocoder.search",
            profile = self.profile.name(),
            autocomplete,
            limit,
            near = opts.lat.is_some() && opts.lon.is_some(),
            filters = opts.filters.len(),
            tokens = tracing::field::Empty,
            meaningful = tracing::field::Empty,
            common = tracing::field::Empty,
            collector = tracing::field::Empty,
            bucket = tracing::field::Empty,
            scored = tracing::field::Empty,
            scoring_ms = tracing::field::Empty,
            results = tracing::field::Empty,
        );
        let _entered = span.enter();
        if text::ascii(query.trim()).chars().count() > QUERY_MAX_LENGTH {
            span.record("results", 0);
            return Vec::new();
        }
        let q = Query {
            text: query,
            // addok may be asked for more results than returned (see
            // `Profile::min_limit`): the thresholds of its collectors depend
            // on it.
            limit: limit.max(self.profile.min_limit()),
            autocomplete,
            lat: opts.lat,
            lon: opts.lon,
            filters: self.filters(&opts.filters),
        };
        let (mut results, stats) = Helper::new(&self.index, self.profile.as_ref(), &q).run();
        results.truncate(limit);
        span.record("tokens", stats.tokens);
        span.record("meaningful", stats.meaningful);
        span.record("common", stats.common);
        if let Some(collector) = stats.collector {
            span.record("collector", collector);
        }
        span.record("bucket", stats.bucket);
        span.record("scored", stats.scored);
        span.record("scoring_ms", stats.scoring.as_secs_f64() * 1000.0);
        span.record("results", results.len());
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
        let span = tracing::info_span!(
            "geocoder.reverse",
            sentry.op = "geocoder.reverse",
            profile = self.profile.name(),
            limit,
            filters = opts.filters.len(),
            candidates = tracing::field::Empty,
            results = tracing::field::Empty,
        );
        let _entered = span.enter();
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

        span.record("candidates", keys.len());
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
        span.record("results", results.len());
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
