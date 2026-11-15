use crate::doc::Document;
use crate::error::GeocoderError;

/// The BAN exports are already in addok's format.
pub fn parse_line(line: &str) -> Result<Document, GeocoderError> {
    Ok(serde_json::from_str(line)?)
}
