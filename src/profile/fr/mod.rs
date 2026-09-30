//! France, configured like the address index of the Géoplateforme geocoder
//! (IGN), which serves the BAN API: addok-fr 1.1.0 + addok-france 1.2.0 on
//! addok 1.3, with `indexes/address/config/addok.conf` of
//! `geoplateforme/geocodage/geocodeur`. The versions are inferred from the
//! API behaviour (multi-value filters, labels for former municipalities,
//! addok-fr 1.1 phonetics), the configuration is the published one.

mod france;
mod phonemicize;

use once_cell::sync::Lazy;

use super::{Boost, FieldSpec, Profile};
use crate::doc::Document;
use crate::text::{Synonyms, Token};

pub use phonemicize::phonemicize;

static SYNONYMS: Lazy<Synonyms> = Lazy::new(|| Synonyms::parse(include_str!("synonyms.txt")));

/// Default fields plus the Géoplateforme `EXTRA_FIELDS` (`citycode`).
const FIELDS: &[FieldSpec] = &[
    FieldSpec {
        key: "name",
        boost: Boost::Const(4.0),
        required: true,
    },
    FieldSpec::new("street"),
    FieldSpec {
        key: "postcode",
        boost: Boost::ByType {
            doc_type: "municipality",
            matched: 1.2,
            otherwise: 1.0,
        },
        required: false,
    },
    FieldSpec::new("city"),
    FieldSpec::new("context"),
    FieldSpec::new("citycode"),
];

const FILTERS: &[&str] = &["source", "type", "citycode", "postcode"];

pub struct FrProfile {
    _private: (),
}

impl FrProfile {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for FrProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl Profile for FrProfile {
    fn name(&self) -> &'static str {
        "fr"
    }

    fn version(&self) -> u32 {
        3
    }

    fn min_score(&self) -> f64 {
        0.3
    }

    /// The Géoplateforme asks addok for at least 10 results
    /// (`api/indexes/address.js`: `Math.max(params.limit || 10, 10)`).
    fn min_limit(&self) -> usize {
        10
    }

    fn fields(&self) -> &[FieldSpec] {
        FIELDS
    }

    fn filters(&self) -> &[&'static str] {
        FILTERS
    }

    fn synonyms(&self) -> Option<&Synonyms> {
        Some(&SYNONYMS)
    }

    /// `extract_address`, `clean_query`, `remove_leading_zeros`.
    fn process_query(&self, query: &str) -> String {
        let query = france::extract_address(query);
        let query = france::clean_query(&query);
        france::remove_leading_zeros(&query)
    }

    /// `glue_ordinal`, `fold_ordinal`, `flag_housenumber`, `synonymize`,
    /// `phonemicize`.
    fn process_tokens(&self, tokens: Vec<Token>) -> Vec<Token> {
        let tokens = france::glue_ordinal(tokens);
        let tokens: Vec<Token> = tokens.into_iter().map(france::fold_ordinal).collect();
        let tokens = france::flag_housenumber(tokens);
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
