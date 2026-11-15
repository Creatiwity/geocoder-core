//! Binary layout of an index file.
//!
//! ```text
//! [0..8]   magic  b"GEOCODER"
//! [8..12]  format version, u32 le
//! [12..16] section count, u32 le
//! [16..]   section headers: kind u8, offset u64 le, length u64 le
//! [...]    sections, each aligned to 8 bytes
//! ```
//!
//! Sections are read in place from the memory map, except the FSTs.
//! Offsets tables hold `count + 1` little-endian u64, entry `i` spanning
//! `offsets[i]..offsets[i + 1]`.

use serde::{Deserialize, Serialize};

pub const MAGIC: &[u8; 8] = b"GEOCODER";
pub const VERSION: u32 = 2;
pub const HEADER_SIZE: usize = 16;
pub const SECTION_HEADER_SIZE: usize = 17;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionKind {
    /// JSON [`Meta`].
    Meta = 0,
    /// `fst::Map`: token → TokenId.
    TokenFst = 1,
    /// Offsets, in entries, into the two postings sections.
    PostingOffsets = 2,
    /// Per token, `(doc u32, boost f32)` sorted by doc.
    PostingsByDoc = 3,
    /// Per token, `doc u32` sorted like Redis `ZREVRANGE`.
    PostingsByScore = 4,
    /// Offsets, in bytes, into `PairBlobs`, per TokenId.
    PairOffsets = 5,
    /// Serialized RoaringBitmap<TokenId> of co-occurring tokens.
    PairBlobs = 6,
    /// `fst::Map`: edge ngram → entry, then offsets and bitmaps of TokenId.
    NgramFst = 7,
    NgramOffsets = 8,
    NgramBlobs = 9,
    /// `fst::Map`: geohash → entry, then offsets and bitmaps of DocId.
    GeohashFst = 10,
    GeohashOffsets = 11,
    GeohashBlobs = 12,
    /// `fst::Map`: `key|value` → entry, then offsets and bitmaps of DocId.
    FilterFst = 13,
    FilterOffsets = 14,
    FilterBlobs = 15,
    /// Offsets, in bytes, into `DocBlobs`, per DocId.
    DocOffsets = 16,
    /// zlib-compressed JSON documents.
    DocBlobs = 17,
}

impl SectionKind {
    pub const ALL: [SectionKind; 18] = [
        Self::Meta,
        Self::TokenFst,
        Self::PostingOffsets,
        Self::PostingsByDoc,
        Self::PostingsByScore,
        Self::PairOffsets,
        Self::PairBlobs,
        Self::NgramFst,
        Self::NgramOffsets,
        Self::NgramBlobs,
        Self::GeohashFst,
        Self::GeohashOffsets,
        Self::GeohashBlobs,
        Self::FilterFst,
        Self::FilterOffsets,
        Self::FilterBlobs,
        Self::DocOffsets,
        Self::DocBlobs,
    ];
}

impl TryFrom<u8> for SectionKind {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        Self::ALL.iter().copied().find(|k| *k as u8 == v).ok_or(v)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SectionHeader {
    pub kind: SectionKind,
    pub offset: u64,
    pub length: u64,
}

/// What an index was built with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub profile: String,
    pub profile_version: u32,
    pub documents: u32,
    pub tokens: u32,
}
