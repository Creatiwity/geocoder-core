//! addok's `config/default.py` values, unchanged by the BAN configuration.

/// Min/max number of documents retrieved and scored.
pub const BUCKET_MIN: usize = 10;
pub const BUCKET_MAX: usize = 100;
/// Above this many documents, a token is common.
pub const COMMON_THRESHOLD: usize = 10_000;
/// Above this many documents, avoid intersecting.
pub const INTERSECT_LIMIT: usize = 100_000;
/// Minimum text score considered a match.
pub const MATCH_THRESHOLD: f64 = 0.9;
/// Results below this final score are dropped.
pub const MIN_SCORE: f64 = 0.1;
pub const QUERY_MAX_LENGTH: usize = 200;
pub const GEOHASH_PRECISION: usize = 7;
pub const MIN_EDGE_NGRAMS: usize = 3;
pub const MAX_EDGE_NGRAMS: usize = 20;
/// Weight of a document's own importance.
pub const IMPORTANCE_WEIGHT: f64 = 0.1;
/// `Search.MAX_MEANINGFUL`.
pub const MAX_MEANINGFUL: usize = 10;
/// Default `limit` of the HTTP API.
pub const DEFAULT_LIMIT: usize = 5;
/// Maximum number of values of a multi-value filter.
pub const MAX_FILTER_VALUES: usize = 10;
