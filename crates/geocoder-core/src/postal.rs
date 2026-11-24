//! CEDEX codes and the municipalities they serve, stored in an index (its
//! optional `Postal` section) for the `fr` profile to geocode CEDEX
//! addresses.
//!
//! A CEDEX code ("92074 PARIS LA DEFENSE CEDEX") is a La Poste delivery
//! code, not a place: the BAN does not know it. The table is built from the
//! sources read by [`crate::readers::cedex`].

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::error::GeocoderError;

/// Largest decompressed table (~200 KB for France).
const MAX_TABLE_BYTES: u64 = 16 << 20;

/// CEDEX code → INSEE codes, and locality → INSEE codes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CedexTable {
    codes: BTreeMap<String, Vec<String>>,
    localities: BTreeMap<String, Vec<String>>,
}

impl CedexTable {
    /// Municipalities served by a CEDEX code.
    pub fn by_code(&self, code: &str) -> &[String] {
        self.codes.get(code).map_or(&[], Vec::as_slice)
    }

    /// Municipalities served under a locality ("PARIS LA DEFENSE"), any
    /// spelling ([`locality_key`]).
    pub fn by_locality(&self, locality: &str) -> &[String] {
        self.localities
            .get(&locality_key(locality))
            .map_or(&[], Vec::as_slice)
    }

    pub fn codes(&self) -> usize {
        self.codes.len()
    }

    pub fn localities(&self) -> usize {
        self.localities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }

    /// zlib compressed lines: `92074 92062` (code, municipalities), then
    /// `=PARIS LA DEFENSE 92026 92062` (locality, municipalities).
    pub fn to_bytes(&self) -> Result<Vec<u8>, GeocoderError> {
        let mut out = ZlibEncoder::new(Vec::new(), Compression::best());
        for (code, insee) in &self.codes {
            writeln!(out, "{code} {}", insee.join(" "))?;
        }
        for (locality, insee) in &self.localities {
            writeln!(out, "={locality}\t{}", insee.join(" "))?;
        }
        Ok(out.finish()?)
    }

    /// Reads [`Self::to_bytes`]; malformed lines are skipped.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GeocoderError> {
        let mut text = String::new();
        ZlibDecoder::new(bytes)
            .take(MAX_TABLE_BYTES + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > MAX_TABLE_BYTES {
            return Err(GeocoderError::InvalidIndex("postal table too large"));
        }
        let mut table = Self::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix('=') {
                let Some((locality, insee)) = rest.split_once('\t') else {
                    continue;
                };
                table.localities.insert(locality.to_string(), words(insee));
            } else if let Some((code, insee)) = line.split_once(' ') {
                table.codes.insert(code.to_string(), words(insee));
            }
        }
        Ok(table)
    }
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

/// Collects (code, delivery line, municipality) triples from the sources.
#[derive(Debug, Default)]
pub struct CedexTableBuilder {
    codes: BTreeMap<String, BTreeSet<String>>,
    localities: BTreeMap<String, BTreeSet<String>>,
}

impl CedexTableBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// One CEDEX address: its code, its delivery label ("PARIS LA DEFENSE
    /// CEDEX 08") and the INSEE code of its municipality. Kept only when the
    /// municipality is in the code's department: the sources have a few
    /// stray ones (92081 → Paris 12e). Returns whether it was kept.
    pub fn add(&mut self, code: &str, label: &str, insee: &str) -> bool {
        let (code, insee) = (code.trim(), insee.trim().to_uppercase());
        let valid_code = code.len() == 5 && code.bytes().all(|b| b.is_ascii_digit());
        let valid_insee = insee.len() == 5
            && insee
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'A' || b == b'B');
        if !valid_code || !valid_insee || department(code) != department(&insee) {
            return false;
        }
        self.codes
            .entry(code.to_string())
            .or_default()
            .insert(insee);
        let locality = locality_key(label);
        if !locality.is_empty() {
            self.localities
                .entry(locality)
                .or_default()
                .insert(code.to_string());
        }
        true
    }

    pub fn codes(&self) -> usize {
        self.codes.len()
    }

    /// A locality serves the municipalities of all its codes.
    pub fn build(self) -> CedexTable {
        let localities = self
            .localities
            .iter()
            .map(|(locality, codes)| {
                let insee: BTreeSet<&String> = codes
                    .iter()
                    .filter_map(|c| self.codes.get(c))
                    .flatten()
                    .collect();
                (
                    locality.clone(),
                    insee.into_iter().cloned().collect::<Vec<_>>(),
                )
            })
            .collect();
        CedexTable {
            codes: self
                .codes
                .into_iter()
                .map(|(code, insee)| (code, insee.into_iter().collect()))
                .collect(),
            localities,
        }
    }
}

/// Of a postcode or an INSEE code: two digits, three overseas, Corsica as
/// "20".
fn department(code: &str) -> &str {
    if code.starts_with("97") || code.starts_with("98") {
        &code[..3]
    } else if code.starts_with("2A") || code.starts_with("2B") {
        "20"
    } else {
        &code[..2]
    }
}

/// The key a locality is looked up by: what comes before "CEDEX", upper
/// case ASCII words, "ST" / "STE" spelled out. "Saint-Quentin-en-Yvelines
/// Cedex 3" and "ST QUENTIN EN YVELINES" give "SAINT QUENTIN EN YVELINES".
pub fn locality_key(label: &str) -> String {
    let upper = crate::text::ascii(label).to_uppercase();
    let mut words = Vec::new();
    for word in upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        match word {
            "CEDEX" => break,
            "ST" => words.push("SAINT"),
            "STE" => words.push("SAINTE"),
            w => words.push(w),
        }
    }
    words.join(" ")
}

/// Whether a delivery label is a CEDEX one.
pub fn is_cedex(label: &str) -> bool {
    crate::text::ascii(label)
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "CEDEX")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> CedexTable {
        let mut b = CedexTableBuilder::new();
        assert!(b.add("92074", "PARIS LA DEFENSE CEDEX", "92062"));
        assert!(b.add("92081", "Paris-La-Défense Cedex 08", "92026"));
        // Another department: dropped.
        assert!(!b.add("92081", "PARIS LA DEFENSE CEDEX", "75112"));
        assert!(b.add("20186", "AJACCIO CEDEX 2", "2A004"));
        assert!(!b.add("9207", "X CEDEX", "92062"));
        b.build()
    }

    #[test]
    fn codes_and_localities() {
        let t = table();
        assert_eq!(t.by_code("92074"), ["92062"]);
        assert_eq!(t.by_code("92081"), ["92026"]);
        assert_eq!(t.by_locality("Paris La Défense"), ["92026", "92062"]);
        assert_eq!(t.by_code("20186"), ["2A004"]);
        assert!(t.by_code("99999").is_empty());
    }

    #[test]
    fn round_trip() {
        let t = table();
        assert_eq!(CedexTable::from_bytes(&t.to_bytes().unwrap()).unwrap(), t);
    }

    #[test]
    fn locality_keys() {
        assert_eq!(
            locality_key("Saint-Quentin-en-Yvelines Cedex 3"),
            locality_key("ST QUENTIN EN YVELINES")
        );
        assert_eq!(locality_key("Paris-La-Défense Cedex"), "PARIS LA DEFENSE");
        assert!(is_cedex("Cannes Cedex") && !is_cedex("Cedexiens"));
    }
}
