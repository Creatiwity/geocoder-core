use crate::index::store::Index;
use crate::text::tokenize;
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
        match index.token_fst.get(&token.normalized) {
            None => not_found.push(token),
            Some(token_id) => {
                let freq = index.token_frequency(token_id as u32);
                if freq >= COMMON_THRESHOLD {
                    common.push((token_id as TokenId, token));
                } else {
                    meaningful.push((token_id as TokenId, token));
                }
            }
        }
    }

    let threshold = compute_threshold(meaningful.len());

    ClassifiedQuery { meaningful, common, housenumbers, not_found, threshold }
}

fn compute_threshold(meaningful_count: usize) -> usize {
    match meaningful_count {
        0 => 0,
        1 => 1,
        n => (n as f32 * 2.0 / 3.0).ceil() as usize,
    }
}
