use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeocoderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("FST build error: {0}")]
    FstBuild(#[from] fst::Error),

    #[error("Geohash error: {0}")]
    Geohash(#[from] geohash::GeohashError),

    #[error("Invalid index: {0}")]
    InvalidIndex(&'static str),
}

