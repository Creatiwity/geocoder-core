//! Belgium. Names are indexed in every official language of their region
//! (`name`, `city` lists, the region's language first). The French
//! processing applies to all of them (the same on index and query, so
//! Dutch words are only normalized, never mismatched), with Dutch
//! abbreviations and the Benelux housenumber and postcode handling.
//!
//! Labels follow the Belgian convention, "Rue de la Loi 16, 1000
//! Bruxelles", in the language closest to the query: "Wetstraat 16, 1000
//! Brussel" when the query is Dutch.

use once_cell::sync::Lazy;

use super::fr::{SYNONYMS_SOURCE, france, phonemicize};
use super::{FieldSpec, Profile, benelux};
use crate::doc::{Document, first_str, str_values};
use crate::text::{Synonyms, Token, ascii, compare_ngrams};

static SYNONYMS: Lazy<Synonyms> = Lazy::new(|| {
    Synonyms::parse(&format!(
        "{SYNONYMS_SOURCE}\n{}",
        include_str!("synonyms.txt")
    ))
});

const FILTERS: &[&str] = &["type", "citycode", "postcode"];

pub struct BeProfile;

impl Profile for BeProfile {
    fn name(&self) -> &'static str {
        "be"
    }

    fn version(&self) -> u32 {
        1
    }

    fn min_score(&self) -> f64 {
        0.3
    }

    fn min_limit(&self) -> usize {
        10
    }

    fn fields(&self) -> &[FieldSpec] {
        super::fr::FIELDS
    }

    fn filters(&self) -> &[&'static str] {
        FILTERS
    }

    fn synonyms(&self) -> Option<&Synonyms> {
        Some(&SYNONYMS)
    }

    fn process_query(&self, query: &str) -> String {
        france::remove_leading_zeros(&benelux::clean_query(query))
    }

    fn process_tokens(&self, tokens: Vec<Token>) -> Vec<Token> {
        let tokens = benelux::glue_ordinal(tokens);
        let tokens: Vec<Token> = tokens.into_iter().map(france::fold_ordinal).collect();
        let tokens = benelux::flag_housenumber(tokens);
        let tokens = SYNONYMS.apply(tokens);
        tokens
            .into_iter()
            .map(|t| t.with_value(phonemicize(&t.value)))
            .collect()
    }

    /// The French combinations (number first) plus the Belgian ones
    /// (number after the street), so both ways of writing score alike.
    fn make_labels(&self, doc: &Document, housenumber: Option<&str>) -> Vec<String> {
        let mut labels = belgian_labels(doc, housenumber);
        labels.extend(france::make_labels(doc, housenumber));
        labels
    }

    fn display_label(
        &self,
        doc: &Document,
        housenumber: Option<&str>,
        query: &str,
    ) -> Option<String> {
        let candidates = belgian_labels(doc, housenumber);
        let query = ascii(query);
        let mut best: Option<(f64, String)> = None;
        for label in candidates {
            let score = compare_ngrams(&ascii(&label), &query);
            if best.as_ref().is_none_or(|(b, _)| score > *b) {
                best = Some((score, label));
            }
        }
        best.map(|(_, label)| label)
    }
}

/// "Rue de la Loi 16, 1000 Bruxelles" for each language; "1000 Bruxelles"
/// for a municipality or a locality.
fn belgian_labels(doc: &Document, housenumber: Option<&str>) -> Vec<String> {
    let postcode = first_str(doc, "postcode").unwrap_or_default();
    let names = str_values(doc, "name");
    let cities = str_values(doc, "city");
    let is_street = !matches!(
        doc.get("type").and_then(|v| v.as_str()),
        Some("municipality" | "locality")
    );
    let place = |name: &str| format!("{postcode} {name}").trim().to_string();
    let mut labels = Vec::new();
    if !is_street {
        for name in &names {
            labels.push(place(name));
        }
        return labels;
    }
    // Each name in its language goes with the city in the same position
    // (lists are ordered by language), else the first city.
    for (i, name) in names.iter().enumerate() {
        let city = cities
            .get(i)
            .or(cities.first())
            .cloned()
            .unwrap_or_default();
        let street = match housenumber.filter(|h| !h.is_empty()) {
            Some(hn) => format!("{name} {hn}"),
            None => name.clone(),
        };
        labels.push(format!("{street}, {}", place(&city)));
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_in_the_query_language() {
        let doc: Document = serde_json::from_value(json!({
            "type": "street", "name": ["Rue de la Loi", "Wetstraat"],
            "postcode": "1000", "city": ["Bruxelles", "Brussel"]
        }))
        .unwrap();
        let p = BeProfile;
        assert_eq!(
            p.display_label(&doc, Some("16"), "wetstraat 16 brussel")
                .as_deref(),
            Some("Wetstraat 16, 1000 Brussel")
        );
        assert_eq!(
            p.display_label(&doc, Some("16"), "16 rue de la loi")
                .as_deref(),
            Some("Rue de la Loi 16, 1000 Bruxelles")
        );
    }
}
