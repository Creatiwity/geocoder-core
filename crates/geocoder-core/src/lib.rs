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
pub mod postal;
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
            postal = tracing::field::Empty,
        );
        let _entered = span.enter();
        if text::ascii(query.trim()).chars().count() > QUERY_MAX_LENGTH {
            span.record("results", 0);
            return Vec::new();
        }
        let filters = self.filters(&opts.filters);
        // addok may be asked for more results than returned (see
        // `Profile::min_limit`): the thresholds of its collectors depend
        // on it.
        let wanted = limit.max(self.profile.min_limit());
        let run = |text: &str, filters: Vec<(String, Vec<String>)>| {
            let q = Query {
                text,
                limit: wanted,
                autocomplete,
                lat: opts.lat,
                lon: opts.lon,
                filters,
            };
            Helper::new(&self.index, self.profile.as_ref(), &q).run()
        };

        let postal = self.profile.postal(query);
        let (mut results, stats) = match &postal {
            None => run(query, filters),
            Some(p) => {
                let plan = self.postal_plan(query, p, &filters);
                span.record("postal", plan.route);
                let restricted = plan.filters != filters;
                let best = |r: &(Vec<Scored>, _)| r.0.first().map_or(0.0, Scored::score);
                let mut found = run(&plan.text, plan.filters.clone());
                if restricted {
                    // Restricted to the right municipalities, the locality
                    // helps when it names one (LILLE) and hurts when it does
                    // not (PARIS LA DEFENSE): both are tried.
                    let names_it = found
                        .0
                        .first()
                        .is_some_and(|r| names_city(&r.doc, &p.locality));
                    if !names_it {
                        let without = run(&plan.without_locality, plan.filters);
                        if best(&without) > best(&found) {
                            found = without;
                        }
                    }
                }
                if best(&found) < POSTAL_FALLBACK_SCORE && plan.text != plan.fallback {
                    // The table may be wrong or stale, the locality not a
                    // place: the query as before, anywhere, if it does better.
                    let anywhere = run(&plan.fallback, filters);
                    if best(&anywhere) > best(&found) {
                        span.record("postal", "fallback");
                        anywhere
                    } else {
                        found
                    }
                } else {
                    found
                }
            }
        };
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
        results
            .into_iter()
            .map(to_result)
            .map(|r| match &postal {
                Some(p) => with_postal(r, &p.line),
                None => r,
            })
            .collect()
    }

    /// What to search for a query carrying a delivery line:
    /// - its code is a real postcode: the line, without the delivery words;
    /// - the code is known: the municipalities it serves, as a filter;
    /// - the locality is known (a business district, a new town): idem;
    /// - otherwise the locality as written.
    ///
    /// A `citycode` filter given by the caller wins over the table's.
    fn postal_plan(
        &self,
        query: &str,
        p: &profile::Postal,
        filters: &[(String, Vec<String>)],
    ) -> PostalPlan {
        let with = |middle: &str| {
            let text = format!(
                "{} {middle} {}",
                &query[..p.range.start],
                &query[p.range.end..]
            );
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        // The query as is: addok-france keeps the department of the code,
        // what the geocoder did before CEDEX lines were handled.
        let fallback = query.to_string();
        let filtered = |route: &'static str, citycodes: &[String]| {
            let mut out = filters.to_vec();
            if !out.iter().any(|(k, _)| k == "citycode") {
                out.push(("citycode".into(), citycodes.to_vec()));
                out.sort();
            }
            // The locality stays: it names the municipality among those of
            // the code (59012 LILLE CEDEX serves Lille and Dunkerque).
            PostalPlan {
                route,
                text: with(&p.locality),
                without_locality: with(""),
                filters: out,
                fallback: fallback.clone(),
            }
        };
        let plain = |route: &'static str, text: String| PostalPlan {
            route,
            without_locality: text.clone(),
            text,
            filters: filters.to_vec(),
            fallback: fallback.clone(),
        };
        let table = self.index.cedex();
        let by_code = p
            .line
            .code
            .as_deref()
            .zip(table)
            .map_or(&[][..], |(code, t)| t.by_code(code));
        let by_locality = table.map_or(&[][..], |t| t.by_locality(&p.locality));
        match p.line.code.as_deref() {
            Some(code) if !self.index.filter("postcode", code).is_empty() => {
                plain("postcode", with(&format!("{code} {}", p.locality)))
            }
            _ if !by_code.is_empty() => filtered("code", by_code),
            _ if !by_locality.is_empty() => filtered("locality", by_locality),
            _ => plain("unknown", with(&p.locality)),
        }
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

/// Below this, a search restricted to the municipalities of a CEDEX is
/// tried again without the restriction.
const POSTAL_FALLBACK_SCORE: f64 = 0.5;

/// Whether one of the document's cities is `locality` ("PARIS LA DEFENSE",
/// "ST QUENTIN"), ignoring case, accents and punctuation.
fn names_city(doc: &Document, locality: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        text::ascii(s)
            .to_uppercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| match w {
                "ST" => "SAINT".to_string(),
                "STE" => "SAINTE".to_string(),
                w => w.to_string(),
            })
            .collect()
    };
    let wanted = words(locality);
    !wanted.is_empty()
        && doc::str_values(doc, "city")
            .iter()
            .any(|city| words(city) == wanted)
}

struct PostalPlan {
    /// For the `geocoder.search` span: how the place was found.
    route: &'static str,
    text: String,
    /// `text` without the locality, tried too when restricted.
    without_locality: String,
    filters: Vec<(String, Vec<String>)>,
    /// Searched when `text` finds nothing.
    fallback: String,
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
        postal: None,
        postal_label: None,
    }
}

/// The delivery line given back, and the mail label: the street part of the
/// result (none for a municipality), then the line.
fn with_postal(mut r: SearchResult, line: &profile::PostalLine) -> SearchResult {
    let name = r
        .doc
        .get("name")
        .and_then(|n| match n {
            serde_json::Value::Array(a) => a.first(),
            n => Some(n),
        })
        .and_then(|n| n.as_str())
        .map(str::to_string);
    let kind = r
        .doc
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    let street = match (kind, name) {
        ("municipality", _) | (_, None) => None,
        (_, Some(name)) => Some(match &r.housenumber {
            Some(hn) => format!("{hn} {name}"),
            None => name,
        }),
    };
    r.postal_label = Some(match street {
        Some(street) => format!("{street} {}", line.line()),
        None => line.line(),
    });
    r.postal = Some(line.clone());
    r
}
