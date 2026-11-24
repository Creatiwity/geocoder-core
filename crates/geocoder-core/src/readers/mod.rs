//! Data sources converted into addok's document format.
//!
//! addok's own format and BANO are read line by line. Address registers
//! (Luxembourg, Belgium) list one address per row: they are read whole and
//! grouped into street, locality and municipality documents
//! (see [`grouped`]).

pub mod ban;
pub mod bano;
pub mod be;
pub mod cedex;
pub mod grouped;
pub mod lu;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use flate2::read::MultiGzDecoder;

use crate::doc::Document;
use crate::error::GeocoderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// addok's own format, e.g. the BAN `adresses-addok-*.ndjson.gz` exports.
    Addok,
    /// BANO (`bano-XX.json.gz`), OpenStreetMap based.
    Bano,
    /// Luxembourg's BD-Adresses (`addresses.csv`).
    Lu,
    /// Belgium's BeST Address (`openaddress-be*.zip`, one per region).
    Be,
}

impl Source {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "addok" | "ban" => Some(Self::Addok),
            "bano" => Some(Self::Bano),
            "lu" | "bd-adresses" => Some(Self::Lu),
            "be" | "best" => Some(Self::Be),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Addok => "addok",
            Self::Bano => "bano",
            Self::Lu => "lu",
            Self::Be => "be",
        }
    }

    /// One document per line, or a register to group.
    pub fn is_line_based(self) -> bool {
        matches!(self, Self::Addok | Self::Bano)
    }

    /// Parse one line of a line based source; `None` for blank lines.
    pub fn parse(self, line: &str) -> Result<Option<Document>, GeocoderError> {
        if line.trim().is_empty() {
            return Ok(None);
        }
        match self {
            Source::Addok => ban::parse_line(line).map(Some),
            Source::Bano => bano::parse_line(line).map(Some),
            Source::Lu | Source::Be => Err(GeocoderError::InvalidIndex(
                "a register is not read line by line",
            )),
        }
    }
}

/// Run `f` on each data file of `path`, decompressed: the file itself, or
/// every entry of a `.zip`; `.gz` files are gunzipped.
pub fn for_each_input(
    path: &Path,
    mut f: impl FnMut(&mut dyn Read) -> Result<(), GeocoderError>,
) -> Result<(), GeocoderError> {
    let name = path.to_string_lossy().to_lowercase();
    let file = File::open(path)?;
    if name.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(file).map_err(zip_error)?;
        for i in 0..archive.len() {
            let entry = archive.by_index(i).map_err(zip_error)?;
            if entry.is_dir() {
                continue;
            }
            f(&mut BufReader::with_capacity(1 << 20, entry))?;
        }
        Ok(())
    } else if name.ends_with(".gz") {
        f(&mut BufReader::with_capacity(
            1 << 20,
            MultiGzDecoder::new(file),
        ))
    } else {
        f(&mut BufReader::with_capacity(1 << 20, file))
    }
}

fn zip_error(e: zip::result::ZipError) -> GeocoderError {
    GeocoderError::Io(std::io::Error::other(e))
}

/// Drop a leading UTF-8 byte order mark.
pub fn skip_bom<R: Read>(reader: R) -> impl Read {
    let mut reader = BufReader::new(reader);
    let mut bom = [0u8; 3];
    let mut read = 0;
    while read < 3 {
        match reader.read(&mut bom[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => break,
        }
    }
    let head = if bom[..read] == [0xEF, 0xBB, 0xBF] {
        Vec::new()
    } else {
        bom[..read].to_vec()
    };
    std::io::Cursor::new(head).chain(reader)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_the_bom_only() {
        let mut out = String::new();
        skip_bom("\u{feff}a;b".as_bytes())
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "a;b");
        out.clear();
        skip_bom("ab".as_bytes()).read_to_string(&mut out).unwrap();
        assert_eq!(out, "ab");
    }
}
