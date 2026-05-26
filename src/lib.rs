mod error;
mod text;
mod types;

pub mod index;
pub mod search;

pub use error::GeocoderError;
pub use index::{Index, IndexBuilder};
pub use types::{Document, HouseNumber, ReverseOpts, SearchOpts, SearchResult};

/// High-level geocoding API. Holds a memory-mapped index.
pub struct Geocoder {
    index: Index,
}

impl Geocoder {
    pub fn open(path: &std::path::Path) -> Result<Self, GeocoderError> {
        let index = Index::open(path)?;
        Ok(Self { index })
    }

    pub fn search(&self, query: &str, opts: SearchOpts) -> Vec<SearchResult> {
        search::search(query, &opts, &self.index)
    }

    pub fn reverse(&self, lat: f64, lon: f64, opts: ReverseOpts) -> Vec<SearchResult> {
        search::reverse(lat, lon, &opts, &self.index)
    }
}
