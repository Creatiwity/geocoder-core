use super::ngram::ngram_similarity;

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
/// Mirrors addok's compare_str(): 85% ngrams + 5% reverse ngrams + 10% edit distance.
pub fn compare_str(query: &str, candidate: &str) -> f32 {
    if query == candidate {
        return 1.0;
    }
    let max_len = query.len().max(candidate.len()) as f32;
    if max_len == 0.0 {
        return 1.0;
    }

    let ngram_score = ngram_similarity(query, candidate);
    let reverse_score = ngram_similarity(
        &query.chars().rev().collect::<String>(),
        &candidate.chars().rev().collect::<String>(),
    );
    let edit_score = 1.0 - (edit_distance(query, candidate) as f32 / max_len);

    0.85 * ngram_score + 0.05 * reverse_score + 0.10 * edit_score
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
}

/// Bigram Dice coefficient on two normalized strings.
/// Mirrors addok's compare_ngrams(label, query, N=2): 2|A∩B|/(|A|+|B|).
/// Used for full-label vs full-query comparison.
pub fn compare_label(query: &str, label: &str) -> f32 {
    use super::normalize::normalize;
    let q = normalize(query);
    let l = normalize(label);
    bigram_dice(&q, &l)
}

fn bigram_dice(a: &str, b: &str) -> f32 {
    use std::collections::HashSet;
    if a.len() < 2 || b.len() < 2 {
        return 0.0;
    }
    // Pad with a single space each side, then collect char bigrams
    let a_chars: Vec<char> = format!(" {} ", a).chars().collect();
    let b_chars: Vec<char> = format!(" {} ", b).chars().collect();
    let a_grams: HashSet<(char, char)> =
        a_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let b_grams: HashSet<(char, char)> =
        b_chars.windows(2).map(|w| (w[0], w[1])).collect();
    let intersection = a_grams.intersection(&b_grams).count();
    let total = a_grams.len() + b_grams.len();
    if total == 0 { 0.0 } else { 2.0 * intersection as f32 / total as f32 }
}
