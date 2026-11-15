use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;
use fst::Map;
use memmap2::Mmap;
use roaring::RoaringBitmap;

use super::format::{HEADER_SIZE, MAGIC, Meta, SECTION_HEADER_SIZE, SectionKind, VERSION};
use crate::doc::Document;
use crate::error::GeocoderError;
use crate::types::{DocId, TokenId};

/// A read-only index, memory mapped. Only the FSTs are copied in memory;
/// postings, bitmaps and documents are read in place on demand.
///
/// Reading never trusts the file: every offset is bounds checked and the
/// FSTs' checksums verified, so a damaged or tampered index fails to open
/// or yields empty lookups, never a panic. Whether the content is genuine
/// is a matter of signatures (see `signed`), not of this type.
pub struct Index {
    mmap: Mmap,
    ranges: Vec<(usize, usize)>,
    pub meta: Meta,
    token_fst: Map<Vec<u8>>,
    ngram_fst: Map<Vec<u8>>,
    geohash_fst: Map<Vec<u8>>,
    filter_fst: Map<Vec<u8>>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self, GeocoderError> {
        let file = std::fs::File::open(path)?;
        // SAFETY: the index file is written once and never modified in place.
        let mmap = unsafe { Mmap::map(&file)? };
        let data: &[u8] = &mmap;
        if data.len() < HEADER_SIZE || &data[..8] != MAGIC {
            return Err(GeocoderError::InvalidIndex("bad magic bytes"));
        }
        if u32::from_le_bytes(data[8..12].try_into().unwrap()) != VERSION {
            return Err(GeocoderError::InvalidIndex(
                "unsupported format version, rebuild the index",
            ));
        }
        let count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let mut ranges = vec![(0usize, 0usize); SectionKind::ALL.len()];
        let mut found = vec![false; SectionKind::ALL.len()];
        for i in 0..count {
            let at = HEADER_SIZE + i * SECTION_HEADER_SIZE;
            let header = data
                .get(at..at + SECTION_HEADER_SIZE)
                .ok_or(GeocoderError::InvalidIndex("truncated"))?;
            let Ok(kind) = SectionKind::try_from(header[0]) else {
                continue;
            };
            let offset = u64::from_le_bytes(header[1..9].try_into().unwrap());
            let length = u64::from_le_bytes(header[9..17].try_into().unwrap());
            let end = offset
                .checked_add(length)
                .filter(|&end| end <= data.len() as u64)
                .ok_or(GeocoderError::InvalidIndex("truncated"))?;
            ranges[kind as usize] = (offset as usize, end as usize);
            found[kind as usize] = true;
        }
        if found.iter().any(|f| !f) {
            return Err(GeocoderError::InvalidIndex("missing section"));
        }
        let section = |kind: SectionKind| {
            let (start, end) = ranges[kind as usize];
            &data[start..end]
        };
        let meta: Meta = serde_json::from_slice(section(SectionKind::Meta))?;
        // fst walks its nodes without bounds checks of its own (it panics on
        // a malformed node): its checksum is verified first.
        let fst = |kind: SectionKind| -> Result<Map<Vec<u8>>, GeocoderError> {
            let map = Map::new(section(kind).to_vec())?;
            map.as_fst().verify()?;
            Ok(map)
        };
        let token_fst = fst(SectionKind::TokenFst)?;
        let ngram_fst = fst(SectionKind::NgramFst)?;
        let geohash_fst = fst(SectionKind::GeohashFst)?;
        let filter_fst = fst(SectionKind::FilterFst)?;
        Ok(Self {
            mmap,
            ranges,
            meta,
            token_fst,
            ngram_fst,
            geohash_fst,
            filter_fst,
        })
    }

    fn section(&self, kind: SectionKind) -> &[u8] {
        let (start, end) = self.ranges[kind as usize];
        &self.mmap[start..end]
    }

    /// Entry `i` of an offset table: `(start, end)`, with `start <= end`.
    fn offset(&self, kind: SectionKind, i: usize) -> Option<(usize, usize)> {
        let table = self.section(kind);
        let at = i.checked_mul(8)?;
        let start = u64::from_le_bytes(table.get(at..at.checked_add(8)?)?.try_into().ok()?);
        let end = u64::from_le_bytes(table.get(at + 8..at.checked_add(16)?)?.try_into().ok()?);
        (start <= end).then_some((usize::try_from(start).ok()?, usize::try_from(end).ok()?))
    }

    /// `section[start * width..end * width]`, empty when out of bounds.
    fn slice(&self, kind: SectionKind, range: Option<(usize, usize)>, width: usize) -> &[u8] {
        range
            .and_then(|(start, end)| {
                let (start, end) = (start.checked_mul(width)?, end.checked_mul(width)?);
                self.section(kind).get(start..end)
            })
            .unwrap_or_default()
    }

    pub fn token_id(&self, token: &str) -> Option<TokenId> {
        self.token_fst.get(token).map(|v| v as TokenId)
    }

    pub fn token_fst(&self) -> &Map<Vec<u8>> {
        &self.token_fst
    }

    /// Number of documents holding the token (Redis `ZCARD`).
    pub fn frequency(&self, token: TokenId) -> usize {
        self.offset(SectionKind::PostingOffsets, token as usize)
            .map(|(s, e)| e - s)
            .unwrap_or(0)
    }

    /// Postings sorted by document.
    pub fn postings(&self, token: TokenId) -> Postings<'_> {
        let range = self.offset(SectionKind::PostingOffsets, token as usize);
        Postings {
            bytes: self.slice(SectionKind::PostingsByDoc, range, 8),
        }
    }

    /// The first `limit` documents of a token, like `ZREVRANGE key 0 limit-1`.
    pub fn top(&self, token: TokenId, limit: usize) -> impl Iterator<Item = DocId> + '_ {
        let range = self.offset(SectionKind::PostingOffsets, token as usize);
        self.slice(SectionKind::PostingsByScore, range, 4)
            .as_chunks::<4>()
            .0
            .iter()
            .take(limit)
            .map(|c| u32::from_le_bytes(*c))
    }

    /// Tokens seen in at least one document along with `token`.
    pub fn pairs(&self, token: TokenId) -> RoaringBitmap {
        let range = self.offset(SectionKind::PairOffsets, token as usize);
        bitmap(self.slice(SectionKind::PairBlobs, range, 1))
    }

    /// Tokens starting with `prefix` (and longer than it).
    pub fn edge_ngram(&self, prefix: &str) -> RoaringBitmap {
        self.keyed(
            &self.ngram_fst,
            prefix,
            SectionKind::NgramOffsets,
            SectionKind::NgramBlobs,
        )
    }

    pub fn geohash(&self, cell: &str) -> RoaringBitmap {
        self.keyed(
            &self.geohash_fst,
            cell,
            SectionKind::GeohashOffsets,
            SectionKind::GeohashBlobs,
        )
    }

    /// Documents whose filter `key` has `value`.
    pub fn filter(&self, key: &str, value: &str) -> RoaringBitmap {
        let k = format!("{key}|{value}");
        self.keyed(
            &self.filter_fst,
            &k,
            SectionKind::FilterOffsets,
            SectionKind::FilterBlobs,
        )
    }

    fn keyed(
        &self,
        fst: &Map<Vec<u8>>,
        key: &str,
        offsets: SectionKind,
        blobs: SectionKind,
    ) -> RoaringBitmap {
        let range = fst.get(key).and_then(|i| self.offset(offsets, i as usize));
        bitmap(self.slice(blobs, range, 1))
    }

    pub fn document(&self, doc: DocId) -> Option<Document> {
        let range = self.offset(SectionKind::DocOffsets, doc as usize)?;
        let blob = self.slice(SectionKind::DocBlobs, Some(range), 1);
        // Bounded: a zlib bomb would otherwise fill the memory.
        let mut json = Vec::new();
        ZlibDecoder::new(blob)
            .take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut json)
            .ok()?;
        if json.len() as u64 > MAX_DOCUMENT_BYTES {
            return None;
        }
        serde_json::from_slice(&json).ok()
    }
}

/// Largest decompressed document. The biggest BAN street (thousands of
/// housenumbers) is a few hundred KB.
const MAX_DOCUMENT_BYTES: u64 = 16 << 20;

/// A serialized bitmap, validated (empty when invalid).
fn bitmap(bytes: &[u8]) -> RoaringBitmap {
    RoaringBitmap::deserialize_from(bytes).unwrap_or_default()
}

/// `(doc, boost)` entries of one token, sorted by doc.
#[derive(Clone, Copy)]
pub struct Postings<'a> {
    bytes: &'a [u8],
}

impl<'a> Postings<'a> {
    pub fn len(&self) -> usize {
        self.bytes.len() / 8
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn get(&self, i: usize) -> (DocId, f32) {
        let at = i * 8;
        let doc = u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap());
        let boost = f32::from_le_bytes(self.bytes[at + 4..at + 8].try_into().unwrap());
        (doc, boost)
    }

    pub fn iter(&self) -> impl Iterator<Item = (DocId, f32)> + 'a {
        let bytes = self.bytes;
        bytes.as_chunks::<8>().0.iter().map(|c| {
            (
                u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
    }

    /// Boost of `doc`, if it holds the token.
    pub fn find(&self, doc: DocId) -> Option<f32> {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            let (d, boost) = self.get(mid);
            match d.cmp(&doc) {
                std::cmp::Ordering::Equal => return Some(boost),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }
}
