use crate::index::store::Index;
use crate::text::{synonyms_for, tokenize};
use crate::types::{Token, TokenId};

const COMMON_THRESHOLD: usize = 10_000;

pub struct ClassifiedQuery {
    pub meaningful: Vec<(TokenId, Token)>,
    pub common: Vec<(TokenId, Token)>,
    pub housenumbers: Vec<Token>,
    pub not_found: Vec<Token>,
    /// Minimum number of meaningful tokens that must match (2/3 rule)
    pub threshold: usize,
}

pub fn preprocess(raw: &str, index: &Index) -> ClassifiedQuery {
    let tokens = tokenize(raw);
    let mut meaningful = Vec::new();
    let mut common = Vec::new();
    let mut housenumbers = Vec::new();
    let mut not_found = Vec::new();

    for token in tokens {
        if token.is_housenumber {
            housenumbers.push(token);
            continue;
        }

        // Try the token itself, then its synonyms
        let token_id = resolve_token(&token.normalized, index);

        match token_id {
            None => not_found.push(token),
            Some(tid) => {
                let freq = index.token_frequency(tid);
                if freq >= COMMON_THRESHOLD {
                    common.push((tid, token));
                } else {
                    meaningful.push((tid, token));
                }
            }
        }
    }

    let threshold = compute_threshold(meaningful.len());
    ClassifiedQuery { meaningful, common, housenumbers, not_found, threshold }
}

/// Look up a token in the FST. If not found, try its synonyms and return the first hit.
fn resolve_token(normalized: &str, index: &Index) -> Option<TokenId> {
    if let Some(id) = index.token_fst.get(normalized) {
        return Some(id as TokenId);
    }
    for &syn in synonyms_for(normalized) {
        if let Some(id) = index.token_fst.get(syn) {
            return Some(id as TokenId);
        }
    }
    None
}

fn compute_threshold(meaningful_count: usize) -> usize {
    match meaningful_count {
        0 => 0,
        1 => 1,
        n => (n as f32 * 2.0 / 3.0).ceil() as usize,
    }
}
