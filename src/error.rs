use thiserror::Error;

#[derive(Debug, Error)]
pub enum GeocoderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("FST error: {0}")]
    Fst(#[from] fst::Error),

    #[error("Geohash error: {0}")]
    Geohash(#[from] geohash::GeohashError),

    #[error("Invalid index: {0}")]
    InvalidIndex(&'static str),

    #[error("The index was built with a profile this build does not know")]
    UnknownProfile,

    #[error(
        "The index was built with profile {profile} v{index}, this build has v{code}: rebuild the index"
    )]
    ProfileVersion {
        profile: String,
        index: u32,
        code: u32,
    },
}
