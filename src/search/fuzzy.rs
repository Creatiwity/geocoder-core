use fst::{IntoStreamer, Map, Streamer};
use fst::automaton::Levenshtein;

use crate::types::TokenId;

/// Returns TokenIds of all tokens within edit distance `max_dist` of `token`.
/// Uses the FST's built-in Levenshtein automaton — no manual neighbor generation needed.
pub fn fuzzy_lookup(token_fst: &Map<Vec<u8>>, token: &str, max_dist: u32) -> Vec<TokenId> {
    let Ok(lev) = Levenshtein::new(token, max_dist) else {
        return vec![];
    };
    let mut stream = token_fst.search(lev).into_stream();
    let mut results = Vec::new();
    while let Some((_, token_id)) = stream.next() {
        results.push(token_id as TokenId);
    }
    results
}
