//! Data sources, converted line by line into addok's document format.

pub mod ban;
pub mod bano;

use crate::doc::Document;
use crate::error::GeocoderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// addok's own format, e.g. the BAN `adresses-addok-*.ndjson.gz` exports.
    Addok,
    /// BANO (`bano-XX.json.gz`), OpenStreetMap based.
    Bano,
}

impl Source {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "addok" | "ban" => Some(Self::Addok),
            "bano" => Some(Self::Bano),
            _ => None,
        }
    }

    /// Parse one line; `None` for blank lines.
    pub fn parse(self, line: &str) -> Result<Option<Document>, GeocoderError> {
        if line.trim().is_empty() {
            return Ok(None);
        }
        match self {
            Source::Addok => ban::parse_line(line).map(Some),
            Source::Bano => bano::parse_line(line).map(Some),
        }
    }
}
