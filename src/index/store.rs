use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use flate2::read::ZlibDecoder;
use fst::Map;
use memmap2::Mmap;
use roaring::RoaringBitmap;

use crate::error::GeocoderError;
use crate::types::{DocId, TokenId};

use super::format::{SectionHeader, SectionKind, MAGIC, VERSION};

pub struct Index {
    _mmap: Mmap,
    pub token_fst: Map<Vec<u8>>,
    /// TokenId → Vec<(DocId, score)> sorted by score desc
    pub token_scores: Vec<Vec<(DocId, f32)>>,
    /// "filter:value" → RoaringBitmap<DocId>
    pub filter_sets: HashMap<String, RoaringBitmap>,
    /// geohash7 → RoaringBitmap<DocId>
    pub geohash_sets: HashMap<String, RoaringBitmap>,
    /// TokenId → RoaringBitmap<TokenId> co-occurrents
    pub pair_sets: HashMap<TokenId, RoaringBitmap>,
    /// ngram prefix → RoaringBitmap<TokenId>
    pub ngram_sets: HashMap<String, RoaringBitmap>,
    /// DocId → byte offset within doc_blobs
    doc_offsets: HashMap<DocId, u64>,
    /// Concatenated zlib-compressed JSON documents
    doc_blobs: Vec<u8>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self, GeocoderError> {
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Self::parse(mmap)
    }

    fn parse(mmap: Mmap) -> Result<Self, GeocoderError> {
        let data: &[u8] = &mmap;

        if &data[..8] != MAGIC {
            return Err(GeocoderError::InvalidIndex("bad magic bytes"));
        }
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(GeocoderError::InvalidIndex("unsupported version"));
        }
        let section_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;

        let mut headers: Vec<SectionHeader> = Vec::with_capacity(section_count);
        let mut cursor = 16;
        for _ in 0..section_count {
            let kind = SectionKind::try_from(data[cursor])
                .map_err(|_| GeocoderError::InvalidIndex("unknown section kind"))?;
            let offset = u64::from_le_bytes(data[cursor + 1..cursor + 9].try_into().unwrap());
            let length = u64::from_le_bytes(data[cursor + 9..cursor + 17].try_into().unwrap());
            headers.push(SectionHeader { kind, offset, length });
            cursor += 17;
        }

        let section = |kind: SectionKind| -> Result<&[u8], GeocoderError> {
            headers
                .iter()
                .find(|h| h.kind == kind)
                .map(|h| &data[h.offset as usize..h.offset as usize + h.length as usize])
                .ok_or(GeocoderError::InvalidIndex("missing section"))
        };

        let token_fst = Map::new(section(SectionKind::TokenFst)?.to_vec())?;
        let token_scores = parse_token_scores(section(SectionKind::TokenScores)?)?;
        let filter_sets = parse_keyed_bitmaps(section(SectionKind::FilterSets)?)?;
        let geohash_sets = parse_keyed_bitmaps(section(SectionKind::GeohashSets)?)?;
        let pair_sets = parse_pair_sets(section(SectionKind::PairSets)?)?;
        let ngram_sets = parse_keyed_bitmaps(section(SectionKind::NgramSets)?)?;
        let doc_offsets = parse_doc_offsets(section(SectionKind::DocOffsets)?)?;
        // DocBlobs copied from mmap into Vec so get_document doesn't borrow mmap
        let doc_blobs = section(SectionKind::DocBlobs)?.to_vec();

        Ok(Self {
            _mmap: mmap,
            token_fst,
            token_scores,
            filter_sets,
            geohash_sets,
            pair_sets,
            ngram_sets,
            doc_offsets,
            doc_blobs,
        })
    }

    /// Decompress and parse the document stored at `doc_id`.
    pub fn get_document(&self, doc_id: DocId) -> Option<serde_json::Value> {
        let &offset = self.doc_offsets.get(&doc_id)?;
        // ZlibDecoder is self-terminating: stops at end of compressed stream
        // even though doc_blobs may contain subsequent documents after this one.
        let blob = &self.doc_blobs[offset as usize..];
        let mut decoder = ZlibDecoder::new(blob);
        let mut json = Vec::new();
        decoder.read_to_end(&mut json).ok()?;
        serde_json::from_slice(&json).ok()
    }

    /// Number of documents indexed under `token_id` (used for frequency ranking).
    pub fn token_frequency(&self, token_id: TokenId) -> usize {
        self.token_scores.get(token_id as usize).map(|v| v.len()).unwrap_or(0)
    }

    /// Intersect token score lists (SUM aggregate), return top-`limit` by score.
    pub fn zinter_top_n(&self, token_ids: &[TokenId], limit: usize) -> Vec<(DocId, f32)> {
        if token_ids.is_empty() {
            return vec![];
        }

        // Process smallest set first for faster intersection
        let mut sorted_ids = token_ids.to_vec();
        sorted_ids.sort_unstable_by_key(|&tid| self.token_frequency(tid));

        let first = match self.token_scores.get(sorted_ids[0] as usize) {
            Some(v) => v,
            None => return vec![],
        };

        let mut candidates: HashMap<DocId, f32> =
            first.iter().map(|&(doc, score)| (doc, score)).collect();

        for &tid in &sorted_ids[1..] {
            let Some(scores) = self.token_scores.get(tid as usize) else {
                return vec![];
            };
            let lookup: HashMap<DocId, f32> =
                scores.iter().map(|&(doc, score)| (doc, score)).collect();
            candidates.retain(|doc, score| match lookup.get(doc) {
                Some(&s) => { *score += s; true }
                None => false,
            });
        }

        let mut result: Vec<(DocId, f32)> = candidates.into_iter().collect();
        if result.len() > limit {
            result.select_nth_unstable_by(limit, |a, b| b.1.partial_cmp(&a.1).unwrap());
            result.truncate(limit);
        }
        result.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        result
    }
}

fn parse_token_scores(data: &[u8]) -> Result<Vec<Vec<(DocId, f32)>>, GeocoderError> {
    let mut r = data;
    let count = r.read_u32::<LittleEndian>()? as usize;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let entry_count = r.read_u32::<LittleEndian>()? as usize;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let doc_id = r.read_u32::<LittleEndian>()?;
            let score = r.read_f32::<LittleEndian>()?;
            entries.push((doc_id, score));
        }
        result.push(entries);
    }
    Ok(result)
}

fn parse_keyed_bitmaps(data: &[u8]) -> Result<HashMap<String, RoaringBitmap>, GeocoderError> {
    let mut r = data;
    let count = r.read_u32::<LittleEndian>()? as usize;
    let mut map = HashMap::with_capacity(count);
    for _ in 0..count {
        let key_len = r.read_u16::<LittleEndian>()? as usize;
        let key = std::str::from_utf8(&r[..key_len])
            .map_err(|_| GeocoderError::InvalidIndex("invalid utf8 key"))?
            .to_string();
        r = &r[key_len..];
        let bm_len = r.read_u32::<LittleEndian>()? as usize;
        let bitmap = RoaringBitmap::deserialize_from(&r[..bm_len])?;
        r = &r[bm_len..];
        map.insert(key, bitmap);
    }
    Ok(map)
}

fn parse_pair_sets(data: &[u8]) -> Result<HashMap<TokenId, RoaringBitmap>, GeocoderError> {
    let mut r = data;
    let count = r.read_u32::<LittleEndian>()? as usize;
    let mut map = HashMap::with_capacity(count);
    for _ in 0..count {
        let token_id = r.read_u32::<LittleEndian>()?;
        let bm_len = r.read_u32::<LittleEndian>()? as usize;
        let bitmap = RoaringBitmap::deserialize_from(&r[..bm_len])?;
        r = &r[bm_len..];
        map.insert(token_id, bitmap);
    }
    Ok(map)
}

fn parse_doc_offsets(data: &[u8]) -> Result<HashMap<DocId, u64>, GeocoderError> {
    let mut r = data;
    let count = r.read_u32::<LittleEndian>()? as usize;
    let mut map = HashMap::with_capacity(count);
    for _ in 0..count {
        let doc_id = r.read_u32::<LittleEndian>()?;
        let offset = r.read_u64::<LittleEndian>()?;
        map.insert(doc_id, offset);
    }
    Ok(map)
}
