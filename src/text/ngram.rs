const NGRAM_SIZE: usize = 3;
const EDGE_NGRAM_MIN: usize = 3;

/// Generates padded character n-grams (mirrors addok's ngrams()).
pub fn ngrams(s: &str) -> Vec<String> {
    let padded = format!("  {}  ", s);
    let chars: Vec<char> = padded.chars().collect();
    chars
        .windows(NGRAM_SIZE)
        .map(|w| w.iter().collect())
        .collect()
}

/// Generates all prefixes of length EDGE_NGRAM_MIN..=len for autocomplete indexing.
pub fn edge_ngrams(token: &str) -> Vec<String> {
    if token.len() < EDGE_NGRAM_MIN {
        return vec![];
    }
    (EDGE_NGRAM_MIN..=token.len())
        .filter_map(|i| token.get(..i))
        .map(|s| s.to_string())
        .collect()
}

/// N-gram similarity score in [0.0, 1.0].
pub fn ngram_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_grams: std::collections::HashSet<String> = ngrams(a).into_iter().collect();
    let b_grams: std::collections::HashSet<String> = ngrams(b).into_iter().collect();
    let intersection = a_grams.intersection(&b_grams).count();
    let union = a_grams.union(&b_grams).count();
    if union == 0 { 0.0 } else { intersection as f32 / union as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_ngrams() {
        let eg = edge_ngrams("paris");
        assert_eq!(eg, vec!["par", "pari", "paris"]);
    }

    #[test]
    fn test_ngram_similarity_identical() {
        assert!((ngram_similarity("paris", "paris") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_ngram_similarity_different() {
        assert!(ngram_similarity("paris", "lyon") < 0.5);
    }
}
