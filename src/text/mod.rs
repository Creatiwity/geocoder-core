pub mod distance;
pub mod ngram;
pub mod normalize;
pub mod synonyms;
pub mod tokenize;

pub use distance::{compare_label, compare_str};
pub use ngram::edge_ngrams;
pub use synonyms::synonyms_for;
pub use tokenize::tokenize;
