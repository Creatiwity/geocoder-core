//! Language-agnostic text primitives, ported from `addok.helpers.text`.
//!
//! Country or language specific processing lives in `crate::profile`.

mod synonyms;
mod token;

use unidecode::unidecode;

pub use synonyms::Synonyms;
pub use token::{Token, TokenKind};

/// `addok.helpers.text.ascii`: lowercase, transliterate, replace every non-word
/// character by a space, collapse runs of spaces and trim.
pub fn ascii(value: &str) -> String {
    alphanumerize(&transliterate(&value.to_lowercase()))
        .trim()
        .to_string()
}

fn alphanumerize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_space = false;
    for c in text.chars() {
        // After unidecode the text is ASCII, where `\w` is `[A-Za-z0-9_]`.
        let c = if c.is_alphanumeric() || c == '_' {
            c
        } else {
            ' '
        };
        if c == ' ' {
            if previous_space {
                continue;
            }
            previous_space = true;
        } else {
            previous_space = false;
        }
        out.push(c);
    }
    out
}

/// Python's `\w` on `str`: `isalnum()` or `_`. Unlike the `regex` crate's
/// `\w`, it includes numbers such as "½" and excludes combining marks.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `addok.helpers.text.tokenize` followed by `normalize`.
pub fn tokenize(text: &str) -> Vec<Token> {
    text.split(|c: char| !is_word_char(c))
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(position, word)| Token::new(normalize(word), position))
        .collect()
}

/// `addok.helpers.text.normalize`: `unidecode(s.lower())`.
pub fn normalize(token: &str) -> String {
    transliterate(&token.to_lowercase())
}

/// `unidecode`, aligned on Python's Unidecode 1.3.6 (pinned by addok 1.1)
/// where the Rust tables differ: vulgar fractions get a leading space
/// (" 1/2"), which splits the token later on, like addok does with
/// mis-encoded "œ" ("S½urdres" → "s", "1/2urdr").
fn transliterate(s: &str) -> String {
    if !s.contains(['¼', '½', '¾']) {
        return unidecode(s);
    }
    let spaced = s
        .replace('¼', " 1/4")
        .replace('½', " 1/2")
        .replace('¾', " 3/4");
    unidecode(&spaced)
}

/// Python's `str.isdigit()` for the ASCII strings we deal with.
pub fn is_digit(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// `addok.helpers.text.compare_ngrams(left, right, N=2, pad_len=0)`, i.e.
/// `NGram.compare`: shared bigrams (with multiplicity) over the bigrams of
/// both strings minus the shared ones.
pub fn compare_ngrams(left: &str, right: &str) -> f64 {
    let left: Vec<char> = ascii(left).chars().collect();
    let right: Vec<char> = ascii(right).chars().collect();
    if left.len() == 1 && right.len() == 1 {
        return if left == right { 1.0 } else { 0.0 };
    }
    if left.len() < 2 || right.len() < 2 {
        return 0.0;
    }
    let mut counts: std::collections::HashMap<(char, char), u32> = std::collections::HashMap::new();
    for w in left.windows(2) {
        *counts.entry((w[0], w[1])).or_default() += 1;
    }
    let mut same = 0u32;
    for w in right.windows(2) {
        if let Some(remaining) = counts.get_mut(&(w[0], w[1]))
            && *remaining > 0
        {
            *remaining -= 1;
            same += 1;
        }
    }
    if same == 0 {
        return 0.0;
    }
    let all = (left.len() - 1) + (right.len() - 1) - same as usize;
    same as f64 / all as f64
}

/// `addok.helpers.text.compute_edge_ngrams`: prefixes from `min` chars up to,
/// but excluding, the token itself, on the token cut to `max + 1` chars.
pub fn edge_ngrams(token: &str, min: usize, max: usize) -> Vec<String> {
    let chars: Vec<char> = token.chars().take(max + 1).collect();
    (min..chars.len())
        .map(|i| chars[..i].iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_folds_like_addok() {
        assert_eq!(ascii("Rue de l'Église"), "rue de l eglise");
        assert_eq!(ascii(" Saint-Étienne  (42)"), "saint etienne 42");
    }

    #[test]
    fn ngram_compare_matches_python() {
        // Values computed with ngram 4.0.3, the version addok 1.0.3 pins.
        let cases = [
            ("rue de la paix 75002 paris", "rue de la paix paris", 0.76),
            (
                "avenue des champs elysees 75008 paris",
                "20 avenue des champs elysees paris",
                0.769_230_769,
            ),
            ("rue de bordeaux 16000 angouleme", "bordeaux", 0.233_333_333),
        ];
        for (l, r, expected) in cases {
            assert!((compare_ngrams(l, r) - expected).abs() < 1e-6, "{l} / {r}");
        }
    }

    #[test]
    fn edge_ngrams_exclude_the_token() {
        assert_eq!(edge_ngrams("paris", 3, 20), vec!["par", "pari"]);
        assert!(edge_ngrams("rue", 3, 20).is_empty());
    }
}
