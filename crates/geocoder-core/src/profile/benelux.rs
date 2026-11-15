//! What Belgium and Luxembourg share, on top of the French processing (most
//! street names of Luxembourg and half of Belgium's are French):
//!
//! - 4-digit postcodes, written "L-9755" or "B-1000";
//! - the housenumber may follow the street ("Rue de la Loi 16"), so it is
//!   the first number of the query wherever it is, except a 4-digit one
//!   (a postcode). addok-france only flags a leading number or one before a
//!   street type, and drops a number followed by another one;
//! - box numbers ("bte 3", "bus 2"), which the registers do not address.

use once_cell::sync::Lazy;
use regex::Regex;

use super::fr::france;
use crate::text::{Token, TokenKind};

static POSTCODE_PREFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[lb] ?-? ?(\d{4})\b").unwrap());
/// "bte 3", "bus 12", "boite a": the box, then up to 4 characters.
static BOX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(bte|bt|boite|bus|box)\b\.? *[0-9a-z]{1,4}\b").unwrap());
static NUMBER: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{1,4}[a-z]{0,2}$").unwrap());
static ORDINAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(bis|ter|[a-z])$").unwrap());

/// Query cleanup, on the ASCII folded query (where "L-1741" already reads
/// "l 1741").
pub fn clean_query(query: &str) -> String {
    let query = POSTCODE_PREFIX.replace_all(query, "${1}");
    let query = BOX.replace_all(&query, " ");
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_postcode(token: &str) -> bool {
    token.len() == 4 && token.bytes().all(|b| b.is_ascii_digit())
}

fn is_number(token: &str) -> bool {
    token.bytes().all(|b| b.is_ascii_digit()) && !token.is_empty() && token.len() < 4
}

/// "16 a" → "16a", "16 bis" → "16bis", when the letter ends the query or
/// comes before a postcode or a street type, like addok-france: elsewhere
/// it starts a name ("31 D. Coppensstraat", "260 's Gravenwezel", "11 Ter
/// Eikenlaan").
pub fn glue_ordinal(tokens: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut iter = tokens.into_iter().peekable();
    while let Some(token) = iter.next() {
        let next = iter.peek();
        if let Some(previous) = out.last_mut()
            && is_number(previous)
            && ORDINAL.is_match(&token)
            && next.is_none_or(|n| is_postcode(n) || france::is_street_type(n))
        {
            previous.value.push_str(&token.value);
            continue;
        }
        out.push(token);
    }
    out
}

/// The first number of the query that is not a postcode.
pub fn flag_housenumber(mut tokens: Vec<Token>) -> Vec<Token> {
    if let Some(token) = tokens
        .iter_mut()
        .find(|t| NUMBER.is_match(t) && !is_postcode(t))
    {
        token.kind = Some(TokenKind::Housenumber);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::tokenize;

    fn values(tokens: &[Token]) -> Vec<&str> {
        tokens.iter().map(|t| t.value.as_str()).collect()
    }

    #[test]
    fn cleans_postcodes_and_boxes() {
        assert_eq!(
            clean_query("rue de la loi 16 bte 3 b-1000 bruxelles"),
            "rue de la loi 16 1000 bruxelles"
        );
        assert_eq!(
            clean_query("rue de hollerich 33 l 1741 luxembourg"),
            "rue de hollerich 33 1741 luxembourg"
        );
        assert_eq!(
            clean_query("kerkstraat 5 bus 12 2000 antwerpen"),
            "kerkstraat 5 2000 antwerpen"
        );
    }

    #[test]
    fn housenumber_after_the_street() {
        let tokens = flag_housenumber(glue_ordinal(tokenize("rue de la loi 16 a 1000 bruxelles")));
        assert_eq!(
            values(&tokens),
            ["rue", "de", "la", "loi", "16a", "1000", "bruxelles"]
        );
        assert!(tokens[4].is_housenumber());
        assert!(!tokens[5].is_housenumber());

        let tokens = flag_housenumber(glue_ordinal(tokenize("1000 bruxelles")));
        assert!(tokens.iter().all(|t| !t.is_housenumber()));

        for query in [
            "31 d coppensstraat",
            "260 s gravenwezel",
            "11 ter eikenlaan",
        ] {
            let tokens = glue_ordinal(tokenize(query));
            assert_eq!(tokens.len(), 3, "{query}");
        }
        let tokens = glue_ordinal(tokenize("12 b rue de la gare"));
        assert_eq!(tokens[0].value, "12b");
    }
}
