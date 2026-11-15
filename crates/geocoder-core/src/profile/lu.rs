//! Luxembourg. Street names are mostly French, the others Luxembourgish or
//! German compounds ("Kaesfurterstrooss") that need no splitting: the
//! French processing applies, with the Benelux housenumber and postcode
//! handling. Labels follow the BD-Adresses: "20 Kaesfurterstrooss 9755
//! Hupperdange".

use once_cell::sync::Lazy;

use super::fr::{SYNONYMS_SOURCE, france, phonemicize};
use super::{FieldSpec, Profile, benelux};
use crate::doc::Document;
use crate::text::{Synonyms, Token};

static SYNONYMS: Lazy<Synonyms> = Lazy::new(|| Synonyms::parse(SYNONYMS_SOURCE));

const FILTERS: &[&str] = &["type", "citycode", "postcode"];

pub struct LuProfile;

impl Profile for LuProfile {
    fn name(&self) -> &'static str {
        "lu"
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

    fn make_labels(&self, doc: &Document, housenumber: Option<&str>) -> Vec<String> {
        france::make_labels(doc, housenumber)
    }
}
