use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub type DocId = u32;
pub type TokenId = u32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default)]
    pub housenumbers: Vec<HouseNumber>,
    #[serde(default)]
    pub filters: HashMap<String, String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

fn default_importance() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseNumber {
    pub number: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub doc: serde_json::Value,
    pub score: f32,
    pub matched_housenumber: Option<String>,
    pub distance_km: Option<f64>,
}

#[derive(Debug, Default, Clone)]
pub struct SearchOpts {
    pub limit: Option<usize>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub filters: HashMap<String, Vec<String>>,
}

#[derive(Debug, Default, Clone)]
pub struct ReverseOpts {
    pub limit: Option<usize>,
    pub filters: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub normalized: String,
    pub position: usize,
    pub is_housenumber: bool,
}

