use std::collections::HashMap;

use super::Token;

/// Synonym table in addok's file format: `a, b, c => wanted words`, `#` for
/// comments. Applied on index and query tokens alike (`synonymize`).
#[derive(Debug, Default, Clone)]
pub struct Synonyms {
    map: HashMap<String, String>,
}

impl Synonyms {
    /// `addok.helpers.text.load_synonyms`.
    pub fn parse(source: &str) -> Self {
        let mut map = HashMap::new();
        for line in source.lines() {
            if line.starts_with('#') {
                continue;
            }
            let Some((synonyms, wanted)) = line.split_once("=>") else {
                continue;
            };
            let wanted = wanted.trim();
            for synonym in synonyms.split(',') {
                let synonym = synonym.trim();
                if !synonym.is_empty() {
                    map.insert(synonym.to_string(), wanted.to_string());
                }
            }
        }
        Self { map }
    }

    /// `addok.helpers.text.synonymize`: replace, never add, possibly into
    /// several tokens.
    pub fn apply(&self, tokens: Vec<Token>) -> Vec<Token> {
        let mut out = Vec::with_capacity(tokens.len());
        for token in tokens {
            let replacement = self
                .map
                .get(&token.value)
                .map(String::as_str)
                .unwrap_or(&token.value);
            for (position, word) in replacement.split_whitespace().enumerate() {
                out.push(token.with_value_at(word.to_string(), position));
            }
        }
        out
    }

    pub fn get(&self, token: &str) -> Option<&str> {
        self.map.get(token).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_addok_format() {
        let s = Synonyms::parse("# comment\nbd,bld, bvd => boulevard\ncr => chemin rural\n");
        assert_eq!(s.get("bld"), Some("boulevard"));
        let tokens = s.apply(vec![Token::new("cr".into(), 0)]);
        assert_eq!(
            tokens.iter().map(|t| t.value.as_str()).collect::<Vec<_>>(),
            ["chemin", "rural"]
        );
    }
}
