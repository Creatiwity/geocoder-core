use super::ngram::ngram_recall;

/// Levenshtein edit distance.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }

    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j - 1].min(dp[i - 1][j]).min(dp[i][j - 1])
            };
        }
    }
    dp[m][n]
}

/// Combined string similarity score in [0.0, 1.0].
/// Mirrors addok's compare_str():
///   score = inter/|right_n| * 0.85  (recall: query 3-grams found in candidate)
///         + inter/|left_n|  * 0.05  (precision: candidate 3-grams found in query)
///         + (1 - distance)  * 0.10  (edit distance, normalized)
/// where distance = (ed - |len_a - len_b|) / max(len_a, len_b)
pub fn compare_str(query: &str, candidate: &str) -> f32 {
    if query == candidate {
        return 1.0;
    }
    let q_len = query.chars().count();
    let c_len = candidate.chars().count();
    let max_len = q_len.max(c_len) as f32;
    if max_len == 0.0 {
        return 1.0;
    }

    // recall: fraction of query's 3-grams found in candidate
    let recall = ngram_recall(candidate, query);
    // precision: fraction of candidate's 3-grams found in query
    let precision = ngram_recall(query, candidate);

    // normalized edit distance per addok: (ed - |len_a - len_b|).max(0) / max_len
    let len_diff = (q_len as i64 - c_len as i64).unsigned_abs() as f32;
    let ed = edit_distance(query, candidate) as f32;
    let distance = (ed - len_diff).max(0.0) / max_len;
    let edit_score = 1.0 - distance;

    0.85 * recall + 0.05 * precision + 0.10 * edit_score
}

/// Bigram Dice coefficient on two normalized strings.
/// Mirrors addok's compare_ngrams(label, query, N=2, pad_len=0): 2|A∩B|/(|A|+|B|).
/// pad_len=0: no padding added before collecting bigrams.
pub fn compare_label(query: &str, label: &str) -> f32 {
    use super::normalize::normalize;
    let q = normalize(query);
    let l = normalize(label);
    bigram_dice(&q, &l)
}

fn bigram_dice(a: &str, b: &str) -> f32 {
    use std::collections::HashSet;
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    // Special case: 1-char strings (NGram.compare returns 0 for them)
    if a_chars.len() == 1 && b_chars.len() == 1 {
        return if a == b { 1.0 } else { 0.0 };
    }
    if a_chars.len() < 2 || b_chars.len() < 2 {
        return 0.0;
    }

    // No padding (pad_len=0), collect character bigrams
    let a_grams: HashSet<(char, char)> =
        a_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let b_grams: HashSet<(char, char)> =
        b_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let intersection = a_grams.intersection(&b_grams).count();
    let total = a_grams.len() + b_grams.len();
    if total == 0 { 0.0 } else { 2.0 * intersection as f32 / total as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edit_distance_identical() {
        assert_eq!(edit_distance("paris", "paris"), 0);
    }

    #[test]
    fn test_edit_distance_one() {
        assert_eq!(edit_distance("paris", "pari"), 1);
    }

    #[test]
    fn test_compare_str_identical() {
        assert!((compare_str("paris", "paris") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_compare_str_close() {
        assert!(compare_str("marseile", "marseille") > 0.7);
    }

    #[test]
    fn test_compare_label_bigram() {
        // Full label vs full query similarity
        let score = compare_label(
            "20 avenue des champs elysees paris",
            "Avenue des Champs Elysees 75008 Paris",
        );
        assert!(score > 0.6, "score was {}", score);
    }

    #[test]
    fn test_bigram_dice_single_char_equal() {
        assert!((bigram_dice("a", "a") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_bigram_dice_single_char_diff() {
        assert!((bigram_dice("a", "b")).abs() < f32::EPSILON);
    }
}
