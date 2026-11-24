//! CEDEX lines: "92074 PARIS LA DEFENSE CEDEX", "75592 PARIS CEDEX 12".
//!
//! A CEDEX code is a La Poste delivery code, not a place: the BAN does not
//! know it, and addok-france only keeps its department. The line is taken
//! out of the query; the geocoder then searches the place it stands for,
//! with the index's CEDEX table ([`crate::postal`]).

use once_cell::sync::Lazy;
use regex::Regex;

use crate::profile::{Postal, PostalLine};

/// "92074 Paris La Défense Cedex 12": the code, then the label.
static WITH_CODE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(\d{5})[\s,]+([^\d,]*?)[\s,]*\bc[eé]dex\b(?:\s*(\d{1,2})\b)?").unwrap()
});
/// Without a code, the label after a comma: "…, Libourne Cedex".
static AFTER_COMMA: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i),\s*([^\d,]*?)\s*\bc[eé]dex\b(?:\s*(\d{1,2})\b)?").unwrap());
static BARE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bc[eé]dex\b(?:\s*(\d{1,2})\b)?").unwrap());

/// The CEDEX line of a raw query, if any.
pub fn postal(query: &str) -> Option<Postal> {
    let (range, code, label, number) = if let Some(c) = WITH_CODE.captures(query) {
        let m = c.get(0)?;
        (
            m.range(),
            Some(c[1].to_string()),
            c[2].to_string(),
            c.get(3).map(|n| n.as_str().to_string()),
        )
    } else if let Some(c) = AFTER_COMMA.captures(query) {
        let m = c.get(0)?;
        // Keep the comma: it separated the street from the city.
        (
            m.start() + 1..m.end(),
            None,
            c[1].to_string(),
            c.get(2).map(|n| n.as_str().to_string()),
        )
    } else {
        let c = BARE.captures(query)?;
        let m = c.get(0)?;
        (
            m.range(),
            None,
            String::new(),
            c.get(1).map(|n| n.as_str().to_string()),
        )
    };
    let locality = normalize(&label);
    let mut line_label = if locality.is_empty() {
        "CEDEX".to_string()
    } else {
        format!("{locality} CEDEX")
    };
    if let Some(number) = &number {
        line_label = format!("{line_label} {number}");
    }
    Some(Postal {
        line: PostalLine {
            code,
            label: line_label,
        },
        range,
        locality,
    })
}

/// Upper case ASCII, single spaces: the way a CEDEX line is written
/// (NF Z10-011).
fn normalize(text: &str) -> String {
    crate::text::ascii(text)
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'' && c != '-')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_with_its_code() {
        let q = "1, avenue du Général-de-Gaulle, 92074 Paris La Défense Cedex";
        let p = postal(q).unwrap();
        assert_eq!(p.line.code.as_deref(), Some("92074"));
        assert_eq!(p.line.label, "PARIS LA DEFENSE CEDEX");
        assert_eq!(p.line.line(), "92074 PARIS LA DEFENSE CEDEX");
        assert_eq!(p.locality, "PARIS LA DEFENSE");
        assert_eq!(&q[..p.range.start], "1, avenue du Général-de-Gaulle, ");
    }

    #[test]
    fn the_number_is_kept() {
        let p = postal("12 rue de Lyon 75592 paris cedex 12").unwrap();
        assert_eq!(p.line.line(), "75592 PARIS CEDEX 12");
        assert_eq!(p.locality, "PARIS");
    }

    #[test]
    fn an_unknown_code_is_given_back_too() {
        let p = postal("3 rue X 99999 Nulle Part Cedex 4").unwrap();
        assert_eq!(p.line.line(), "99999 NULLE PART CEDEX 4");
    }

    #[test]
    fn without_code() {
        let q = "1 rue Vidal, Libourne Cedex";
        let p = postal(q).unwrap();
        assert_eq!(p.line.code, None);
        assert_eq!(p.line.line(), "LIBOURNE CEDEX");
        assert_eq!(&q[..p.range.start], "1 rue Vidal,");

        let p = postal("1 rue Vidal Libourne cedex 3").unwrap();
        assert_eq!(p.line.line(), "CEDEX 3");
    }

    #[test]
    fn no_cedex() {
        assert!(postal("1 rue de la Paix 75002 Paris").is_none());
        // A word containing it is not one.
        assert!(postal("1 rue des Cedexiens").is_none());
    }
}
