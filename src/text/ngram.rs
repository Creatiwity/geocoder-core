const NGRAM_SIZE: usize = 3;
const EDGE_NGRAM_MIN: usize = 3;

/// Generates padded character n-grams (mirrors addok's ngrams()).
/// Padding: one space each side, then sliding window of NGRAM_SIZE.
pub fn ngrams(s: &str) -> Vec<String> {
    let padded = format!(" {} ", s);
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

/// N-gram recall: fraction of b's 3-grams that appear in a.
/// Mirrors addok's compare_str() recall component: inter / |right_n|.
pub fn ngram_recall(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if b.is_empty() {
        return 0.0;
    }
    let a_grams: std::collections::HashSet<String> = ngrams(a).into_iter().collect();
    let b_grams: std::collections::HashSet<String> = ngrams(b).into_iter().collect();
    let b_len = b_grams.len();
    if b_len == 0 {
        return 0.0;
    }
    let intersection = a_grams.intersection(&b_grams).count();
    intersection as f32 / b_len as f32
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
    fn test_ngrams_padding() {
        // Single space padding each side
        let grams = ngrams("rue");
        assert!(grams.contains(&" ru".to_string()));
        assert!(grams.contains(&"rue".to_string()));
        assert!(grams.contains(&"ue ".to_string()));
        assert_eq!(grams.len(), 3);
    }

    #[test]
    fn test_ngram_recall_identical() {
        assert!((ngram_recall("paris", "paris") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_ngram_recall_different() {
        assert!(ngram_recall("paris", "lyon") < 0.5);
    }

    #[test]
    fn test_ngram_recall_asymmetric() {
        // recall(a, b) = fraction of b's grams in a
        // "paris" contains all 3-grams of "par" (prefix case)
        let r = ngram_recall("paris", "par");
        assert!(r > 0.5, "recall was {}", r);
        // reverse: "par" does not contain all 3-grams of "paris"
        let r2 = ngram_recall("par", "paris");
        assert!(r2 < r, "expected asymmetry, got {} vs {}", r2, r);
    }
}
