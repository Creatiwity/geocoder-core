//! Country / language profiles.
//!
//! The engine (index, collectors, scoring) is generic. Everything that
//! depends on how addresses are written in a given country goes through a
//! [`Profile`]: query cleanup, token processing (housenumber detection,
//! synonyms, phonetics), result labels, indexed fields and filters.
//!
//! Every method has a default mirroring core addok's configuration; a
//! country overrides only what it needs, like addok plugins do. The same
//! profile must be used to build and to query an index: its name and version
//! are stored in the index and checked when it is opened.

pub mod default;
pub mod fr;

use std::sync::Arc;

use crate::doc::Document;
use crate::text::{Synonyms, Token, tokenize};

/// How much a field weighs in the token → document boost.
#[derive(Debug, Clone, Copy)]
pub enum Boost {
    Const(f64),
    /// `matched` when the document `type` equals `doc_type`, else `otherwise`.
    ByType {
        doc_type: &'static str,
        matched: f64,
        otherwise: f64,
    },
}

impl Boost {
    pub fn value(&self, doc: &Document) -> f64 {
        match *self {
            Boost::Const(v) => v,
            Boost::ByType {
                doc_type,
                matched,
                otherwise,
            } => {
                if doc.get("type").and_then(|v| v.as_str()) == Some(doc_type) {
                    matched
                } else {
                    otherwise
                }
            }
        }
    }
}

/// An indexed text field (addok's `FIELDS`).
#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub key: &'static str,
    pub boost: Boost,
    /// A document missing a required field is not indexed.
    pub required: bool,
}

impl FieldSpec {
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            boost: Boost::Const(1.0),
            required: false,
        }
    }
}

pub trait Profile: Send + Sync {
    /// Stored in the index; must be unique among registered profiles.
    fn name(&self) -> &'static str;

    /// Bump whenever token processing changes: an index built with another
    /// version would no longer match the queries.
    fn version(&self) -> u32;

    /// Indexed text fields, with their boosts.
    fn fields(&self) -> &[FieldSpec] {
        default::FIELDS
    }

    /// Document keys usable as search filters.
    fn filters(&self) -> &[&'static str] {
        default::FILTERS
    }

    /// Keyboard used to limit fuzzy substitutions to neighbouring keys
    /// (addok's `FUZZY_KEY_MAP`); `None` tries every letter.
    fn fuzzy_key_map(&self) -> Option<crate::search::fuzzy::KeyMap> {
        Some(crate::search::fuzzy::AZERTY)
    }

    /// Number of results addok computes at least, whatever the requested
    /// limit; the best `limit` of them are returned. It changes which
    /// collectors run (a bucket is "full" or "dry" relative to it).
    fn min_limit(&self) -> usize {
        1
    }

    /// Results scoring below this are dropped (addok's `MIN_SCORE`).
    fn min_score(&self) -> f64 {
        crate::settings::MIN_SCORE
    }

    /// Synonyms applied to index and query tokens.
    fn synonyms(&self) -> Option<&Synonyms> {
        None
    }

    /// addok's `QUERY_PROCESSORS`: string cleanup applied to the ASCII
    /// folded query only, before tokenizing.
    fn process_query(&self, query: &str) -> String {
        query.to_string()
    }

    /// addok's `PROCESSORS` after `tokenize` and `normalize`, run on index
    /// and query text alike.
    fn process_tokens(&self, tokens: Vec<Token>) -> Vec<Token> {
        let tokens = default::flag_housenumber(tokens);
        match self.synonyms() {
            Some(synonyms) => synonyms.apply(tokens),
            None => tokens,
        }
    }

    /// Labels a result is compared against, best one first.
    fn make_labels(&self, doc: &Document, housenumber: Option<&str>) -> Vec<String> {
        default::make_labels(doc, housenumber)
    }
}

/// Index-time text processing (`addok.helpers.index.preprocess`).
pub fn preprocess(profile: &dyn Profile, text: &str) -> Vec<Token> {
    profile.process_tokens(tokenize(text))
}

/// Query-time text processing (`addok.helpers.search.preprocess_query`), on
/// an already ASCII-folded query.
pub fn preprocess_query(profile: &dyn Profile, query: &str) -> Vec<Token> {
    profile.process_tokens(tokenize(&profile.process_query(query)))
}

/// Profiles an index can be built with.
pub fn by_name(name: &str) -> Option<Arc<dyn Profile>> {
    match name {
        "default" => Some(Arc::new(default::DefaultProfile)),
        "fr" => Some(Arc::new(fr::FrProfile::new())),
        _ => None,
    }
}

pub fn names() -> &'static [&'static str] {
    &["default", "fr"]
}
