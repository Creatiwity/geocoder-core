//! `/search` and `/reverse`, with the parameters of the BAN API plus
//! `country`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use geocoder_core::{Geocoder, ReverseOpts, SearchOpts};
use serde::Deserialize;
use serde_json::{Map, Value};
use utoipa::IntoParams;

use super::Context;
use super::error::{ApiError, ErrorBody};
use super::geojson::{FeatureCollection, feature};
use crate::config::Country;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchParams {
    /// Address, or its beginning with `autocomplete` (3 to 200 characters).
    pub q: String,
    /// Number of results, 1 to 20 (default 5).
    pub limit: Option<usize>,
    /// `1` (default) treats the last word as a prefix, `0` does not.
    pub autocomplete: Option<String>,
    /// Favour addresses around this point (with `lon`).
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Comma separated: `housenumber`, `street`, `locality`, `municipality`.
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub kind: Option<String>,
    /// Comma separated postcodes.
    pub postcode: Option<String>,
    /// Comma separated municipality codes (INSEE in France, LAU2 in
    /// Luxembourg, NIS in Belgium).
    pub citycode: Option<String>,
    /// `fr`, `lu` or `be`: the index to search (default: the server's).
    pub country: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ReverseParams {
    pub lat: f64,
    pub lon: f64,
    /// Number of results, 1 to 20 (default 1).
    pub limit: Option<usize>,
    /// Comma separated: `housenumber`, `street`, `locality`, `municipality`.
    #[serde(rename = "type")]
    #[param(rename = "type")]
    pub kind: Option<String>,
    pub postcode: Option<String>,
    pub citycode: Option<String>,
    /// `fr`, `lu` or `be`: the index to search (default: the server's).
    pub country: Option<String>,
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::BadRequest(message.into())
}

/// The country asked for and its loaded index.
fn index<'a>(
    context: &'a Context,
    country: Option<&str>,
) -> Result<(&'a Country, Arc<Geocoder>), ApiError> {
    let code = country
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| context.config.default_country.clone());
    let config = context.config.country(&code).ok_or_else(|| {
        let served: Vec<&str> = context
            .config
            .countries
            .iter()
            .map(|c| c.code.as_str())
            .collect();
        bad(format!("country must be one of: {}", served.join(", ")))
    })?;
    let geocoder = context
        .indexes
        .get(&code)
        .and_then(|h| h.get())
        .ok_or_else(|| ApiError::Unavailable(format!("the {code} index is not built yet")))?;
    Ok((config, geocoder))
}

fn limit(value: Option<usize>, default: usize) -> Result<usize, ApiError> {
    let limit = value.unwrap_or(default);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(bad(format!("limit must be between 1 and {MAX_LIMIT}")));
    }
    Ok(limit)
}

type Filters = HashMap<String, Vec<String>>;

/// Comma separated filters, and their echo in the response.
fn filters(pairs: [(&str, &Option<String>); 3]) -> (Filters, Option<Map<String, Value>>) {
    let mut out = HashMap::new();
    let mut echo = Map::new();
    for (key, value) in pairs {
        let Some(value) = value.as_deref() else {
            continue;
        };
        let values: Vec<String> = value
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect();
        if values.is_empty() {
            continue;
        }
        echo.insert(key.to_string(), Value::String(values.join(",")));
        out.insert(key.to_string(), values);
    }
    let echo = (!echo.is_empty()).then_some(echo);
    (out, echo)
}

fn valid_coordinates(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

/// Geocode an address
///
/// Forward geocoding, like the BAN API's `/search`.
#[utoipa::path(
    get,
    path = "/search",
    params(SearchParams),
    responses(
        (status = 200, description = "Matching addresses, best first", body = FeatureCollection),
        (status = 400, description = "Invalid parameters", body = ErrorBody),
        (status = 503, description = "Index not built yet", body = ErrorBody)
    )
)]
pub async fn search(
    State(context): State<Arc<Context>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<FeatureCollection>, ApiError> {
    let q = params.q.trim().to_string();
    let length = q.chars().count();
    if !(3..=200).contains(&length) || !q.chars().next().is_some_and(char::is_alphanumeric) {
        return Err(bad(
            "q must contain between 3 and 200 chars and start with a number or a letter",
        ));
    }
    let limit = limit(params.limit, DEFAULT_LIMIT)?;
    let autocomplete = match params.autocomplete.as_deref().map(str::trim) {
        None | Some("1") | Some("true") => true,
        Some("0") | Some("false") => false,
        Some(_) => return Err(bad("autocomplete must be 0 or 1")),
    };
    let center = match (params.lat, params.lon) {
        (Some(lat), Some(lon)) if valid_coordinates(lat, lon) => Some((lat, lon)),
        (None, None) => None,
        _ => return Err(bad("lat and lon must be valid and provided together")),
    };
    let (filters, echo) = filters([
        ("type", &params.kind),
        ("postcode", &params.postcode),
        ("citycode", &params.citycode),
    ]);
    let (country, geocoder) = index(&context, params.country.as_deref())?;
    let opts = SearchOpts {
        limit: Some(limit),
        lat: center.map(|c| c.0),
        lon: center.map(|c| c.1),
        filters,
        autocomplete: Some(autocomplete),
    };
    // Off the async runtime: the index is memory mapped, a query may fault
    // pages in.
    let query = q.clone();
    let results = tokio::task::spawn_blocking(move || geocoder.search(&query, opts))
        .await
        .map_err(|_| ApiError::Unavailable("search interrupted".into()))?;
    Ok(Json(FeatureCollection {
        kind: "FeatureCollection",
        version: "draft",
        features: results.iter().map(|r| feature(r, country)).collect(),
        attribution: country.attribution,
        licence: country.licence,
        query: Some(q),
        filters: echo,
        center: center.map(|(lat, lon)| [lon, lat]),
        limit,
    }))
}

/// Reverse geocode a point
///
/// The addresses closest to a point, like the BAN API's `/reverse`.
#[utoipa::path(
    get,
    path = "/reverse",
    params(ReverseParams),
    responses(
        (status = 200, description = "Closest addresses first", body = FeatureCollection),
        (status = 400, description = "Invalid parameters", body = ErrorBody),
        (status = 503, description = "Index not built yet", body = ErrorBody)
    )
)]
pub async fn reverse(
    State(context): State<Arc<Context>>,
    Query(params): Query<ReverseParams>,
) -> Result<Json<FeatureCollection>, ApiError> {
    if !valid_coordinates(params.lat, params.lon) {
        return Err(bad("lat and lon must be valid coordinates"));
    }
    let limit = limit(params.limit, 1)?;
    let (filters, echo) = filters([
        ("type", &params.kind),
        ("postcode", &params.postcode),
        ("citycode", &params.citycode),
    ]);
    let (country, geocoder) = index(&context, params.country.as_deref())?;
    let (lat, lon) = (params.lat, params.lon);
    let opts = ReverseOpts {
        limit: Some(limit),
        filters,
    };
    let results = tokio::task::spawn_blocking(move || geocoder.reverse(lat, lon, opts))
        .await
        .map_err(|_| ApiError::Unavailable("search interrupted".into()))?;
    Ok(Json(FeatureCollection {
        kind: "FeatureCollection",
        version: "draft",
        features: results.iter().map(|r| feature(r, country)).collect(),
        attribution: country.attribution,
        licence: country.licence,
        query: None,
        filters: echo,
        center: Some([lon, lat]),
        limit,
    }))
}
