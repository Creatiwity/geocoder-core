use std::collections::HashMap;

pub type DocId = u32;
pub type TokenId = u32;

#[derive(Debug, Clone, Default)]
pub struct SearchOpts {
    /// Number of results, 5 by default like the addok HTTP API.
    pub limit: Option<usize>,
    /// Center, to favour nearby results.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Filters on indexed filter keys (`type`, `postcode`…). Several values
    /// for one key are OR-ed.
    pub filters: HashMap<String, Vec<String>>,
    /// Treat the last word as a prefix. On by default, like the HTTP API.
    pub autocomplete: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ReverseOpts {
    pub limit: Option<usize>,
    pub filters: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The document, with the matched housenumber's data (coordinates, id…)
    /// merged in and `type` set to `housenumber`, like addok does.
    pub doc: serde_json::Value,
    /// Display label, e.g. "42 Rue de Rivoli 75004 Paris".
    pub label: String,
    pub score: f32,
    pub housenumber: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub distance_km: Option<f64>,
}
