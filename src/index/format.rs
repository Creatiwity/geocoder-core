/// Binary layout of index.bin
///
/// [0..8]   Magic: b"GEOCODER"
/// [8..12]  Version: u32 le
/// [12..16] Section count: u32 le
/// [16..]   Section headers: [SectionHeader; count]
/// [...]    Section data (aligned to 8 bytes)
///
/// Each section is identified by SectionKind and contains self-describing data.

pub const MAGIC: &[u8; 8] = b"GEOCODER";
pub const VERSION: u32 = 1;
pub const HEADER_SIZE: usize = 16;
pub const SECTION_HEADER_SIZE: usize = 17; // 1 (kind) + 8 (offset) + 8 (length)

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    /// fst::Map bytes: token_str → TokenId (u32)
    TokenFst = 0,
    /// u32 token_count, then for each TokenId:
    ///   u32 entry_count, then (DocId: u32, score: f32) * entry_count
    TokenScores = 1,
    /// Serialized RoaringBitmaps keyed by filter string "type:street", "postcode:75001"
    /// u32 entry_count, then for each: u16 key_len, key bytes, u32 bitmap_len, bitmap bytes
    FilterSets = 2,
    /// Serialized RoaringBitmaps keyed by geohash string (7 chars)
    /// Same layout as FilterSets
    GeohashSets = 3,
    /// Token co-occurrence: TokenId → RoaringBitmap<TokenId>
    /// u32 entry_count, then for each: u32 token_id, u32 bitmap_len, bitmap bytes
    PairSets = 4,
    /// Edge n-gram index: prefix → RoaringBitmap<TokenId>
    /// Same layout as FilterSets
    NgramSets = 5,
    /// Document offset table: u32 doc_count, then (DocId: u32, offset: u64) * doc_count
    DocOffsets = 6,
    /// Concatenated zlib-compressed JSON documents, addressed by DocOffsets
    DocBlobs = 7,
}

impl TryFrom<u8> for SectionKind {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        match v {
            0 => Ok(Self::TokenFst),
            1 => Ok(Self::TokenScores),
            2 => Ok(Self::FilterSets),
            3 => Ok(Self::GeohashSets),
            4 => Ok(Self::PairSets),
            5 => Ok(Self::NgramSets),
            6 => Ok(Self::DocOffsets),
            7 => Ok(Self::DocBlobs),
            other => Err(other),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SectionHeader {
    pub kind: SectionKind,
    pub offset: u64,
    pub length: u64,
}
