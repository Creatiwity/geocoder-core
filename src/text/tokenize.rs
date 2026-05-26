use once_cell::sync::Lazy;
use regex::Regex;

use super::normalize::normalize;
use crate::types::Token;

static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\w+").unwrap());
static HOUSENUMBER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{1,4}[a-zA-Z]?$").unwrap());

pub fn tokenize(text: &str) -> Vec<Token> {
    WORD_RE
        .find_iter(text)
        .enumerate()
        .map(|(position, m)| {
            let text = m.as_str().to_string();
            let normalized = normalize(&text);
            let is_housenumber = HOUSENUMBER_RE.is_match(&text);
            Token { text, normalized, position, is_housenumber }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let tokens = tokenize("15 rue de la Paix");
        assert_eq!(tokens.len(), 5);
        assert!(tokens[0].is_housenumber);
        assert!(!tokens[1].is_housenumber);
    }

    #[test]
    fn test_housenumber_with_letter() {
        let tokens = tokenize("15b avenue Foch");
        assert!(tokens[0].is_housenumber);
    }
}
