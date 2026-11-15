//! Core addok behaviour, used as is by [`DefaultProfile`] and as building
//! blocks by country profiles.

use super::{Boost, FieldSpec, Profile};
use crate::doc::{Document, first_str};
use crate::text::{Token, TokenKind, is_digit};

/// addok's default `FIELDS`.
pub const FIELDS: &[FieldSpec] = &[
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
];

/// addok's default `FILTERS`.
pub const FILTERS: &[&str] = &["type", "postcode"];

/// Profile with no language-specific processing.
pub struct DefaultProfile;

impl Profile for DefaultProfile {
    fn name(&self) -> &'static str {
        "default"
    }

    fn version(&self) -> u32 {
        1
    }
}

/// `addok.helpers.text.flag_housenumber`: a leading number.
pub fn flag_housenumber(mut tokens: Vec<Token>) -> Vec<Token> {
    for token in &mut tokens {
        if token.is_first && is_digit(token) {
            token.kind = Some(TokenKind::Housenumber);
        }
    }
    tokens
}

/// `addok.helpers.results.make_labels`.
pub fn make_labels(doc: &Document, housenumber: Option<&str>) -> Vec<String> {
    let mut labels = crate::doc::str_values(doc, "name");
    let Some(first) = labels.first().cloned() else {
        return labels;
    };
    let mut label = first;
    if let Some(city) = first_str(doc, "city").filter(|c| !c.is_empty())
        && city != label
    {
        if let Some(postcode) = first_str(doc, "postcode").filter(|p| !p.is_empty()) {
            label = format!("{label} {postcode}");
        }
        label = format!("{label} {city}");
        labels.insert(0, label.clone());
    }
    if let Some(housenumber) = housenumber.filter(|h| !h.is_empty()) {
        labels.insert(0, format!("{housenumber} {label}"));
    }
    labels
}
