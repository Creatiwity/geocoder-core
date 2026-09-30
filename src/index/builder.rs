use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::Path;

use byteorder::{LittleEndian, WriteBytesExt};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use fst::MapBuilder;
use roaring::RoaringBitmap;

use crate::error::GeocoderError;
use crate::text::{edge_ngrams, phonemicize, synonyms_for, tokenize};
use crate::types::{DocId, Document, TokenId};

use super::format::{MAGIC, SectionHeader, SectionKind, VERSION};

const FIELD_BOOSTS: &[(&str, f32)] = &[("name", 4.0), ("street", 1.0), ("city", 1.0)];

#[derive(Default)]
pub struct IndexBuilder {
    next_doc_id: DocId,
    next_token_id: TokenId,
    /// token normalized string → TokenId
    token_ids: HashMap<String, TokenId>,
    /// TokenId → sorted Vec<(DocId, score)>
    token_scores: HashMap<TokenId, Vec<(DocId, f32)>>,
    /// "filter_name:value" → RoaringBitmap<DocId>
    filter_sets: HashMap<String, RoaringBitmap>,
    /// geohash7 → RoaringBitmap<DocId>
    geohash_sets: HashMap<String, RoaringBitmap>,
    /// TokenId → RoaringBitmap<TokenId> (co-occurrents)
    pair_sets: HashMap<TokenId, RoaringBitmap>,
    /// ngram prefix → RoaringBitmap<TokenId>
    ngram_sets: HashMap<String, RoaringBitmap>,
    /// (DocId, zlib-compressed JSON bytes)
    doc_blobs: Vec<(DocId, Vec<u8>)>,
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self {
            next_doc_id: 0,
            next_token_id: 0,
            token_ids: HashMap::new(),
            token_scores: HashMap::new(),
            filter_sets: HashMap::new(),
            geohash_sets: HashMap::new(),
            pair_sets: HashMap::new(),
            ngram_sets: HashMap::new(),
            doc_blobs: Vec::new(),
        }
    }

    pub fn add_document(&mut self, doc: Document) -> Result<(), GeocoderError> {
        let doc_id = self.next_doc_id;
        self.next_doc_id += 1;

        // Store compressed document
        let json = serde_json::to_vec(&doc)?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&json)?;
        self.doc_blobs.push((doc_id, encoder.finish()?));

        // Index geohash
        let coord = geohash::Coord {
            x: doc.lon,
            y: doc.lat,
        };
        let gh = geohash::encode(coord, 7)?;
        self.geohash_sets.entry(gh).or_default().insert(doc_id);

        // Index housenumber geohashes
        for hn in &doc.housenumbers {
            let coord = geohash::Coord {
                x: hn.lon,
                y: hn.lat,
            };
            let gh = geohash::encode(coord, 7)?;
            self.geohash_sets.entry(gh).or_default().insert(doc_id);
        }

        // Index filters
        for (key, value) in &doc.filters {
            let filter_key = format!("{}:{}", key, value);
            self.filter_sets
                .entry(filter_key)
                .or_default()
                .insert(doc_id);
        }

        // Index tokens from fields
        let boost = doc.importance;
        let mut doc_token_ids: Vec<TokenId> = Vec::new();

        for (field_name, field_boost) in FIELD_BOOSTS {
            let field_value = match *field_name {
                "name" => Some(doc.name.as_str()),
                _ => doc.extra.get(*field_name).and_then(|v| v.as_str()),
            };
            if let Some(value) = field_value {
                let score = boost * field_boost;
                for token in tokenize(value) {
                    if token.is_housenumber {
                        continue;
                    }
                    self.index_token_with_synonyms(
                        &token.normalized,
                        &token.lemma,
                        doc_id,
                        score,
                        &mut doc_token_ids,
                    );
                }
            }
        }

        // Token pairs (co-occurrence)
        for (i, &tid_a) in doc_token_ids.iter().enumerate() {
            for &tid_b in &doc_token_ids[i + 1..] {
                self.pair_sets.entry(tid_a).or_default().insert(tid_b);
                self.pair_sets.entry(tid_b).or_default().insert(tid_a);
            }
        }

        Ok(())
    }

    /// Write the index to a binary file at `path`.
    pub fn build(mut self, path: &Path) -> Result<(), GeocoderError> {
        // Sort token scores by score descending (for top-N queries)
        for scores in self.token_scores.values_mut() {
            scores.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        }

        let file = std::fs::File::create(path)?;
        let mut w = BufWriter::new(file);

        // Collect section data in memory before writing header
        let token_fst_bytes = self.build_token_fst()?;
        let token_scores_bytes = self.build_token_scores()?;
        let filter_sets_bytes = Self::build_keyed_bitmaps(&self.filter_sets)?;
        let geohash_sets_bytes = Self::build_keyed_bitmaps(&self.geohash_sets)?;
        let pair_sets_bytes = self.build_pair_sets()?;
        let ngram_sets_bytes = Self::build_keyed_bitmaps(&self.ngram_sets)?;
        let (doc_offsets_bytes, doc_blobs_bytes) = self.build_doc_sections()?;

        let sections: &[(SectionKind, &[u8])] = &[
            (SectionKind::TokenFst, &token_fst_bytes),
            (SectionKind::TokenScores, &token_scores_bytes),
            (SectionKind::FilterSets, &filter_sets_bytes),
            (SectionKind::GeohashSets, &geohash_sets_bytes),
            (SectionKind::PairSets, &pair_sets_bytes),
            (SectionKind::NgramSets, &ngram_sets_bytes),
            (SectionKind::DocOffsets, &doc_offsets_bytes),
            (SectionKind::DocBlobs, &doc_blobs_bytes),
        ];

        // Header
        w.write_all(MAGIC)?;
        w.write_u32::<LittleEndian>(VERSION)?;
        w.write_u32::<LittleEndian>(sections.len() as u32)?;

        // Compute section offsets
        let headers_end = 16 + sections.len() as u64 * super::format::SECTION_HEADER_SIZE as u64;
        let mut offset = align8(headers_end);

        let mut headers: Vec<SectionHeader> = Vec::with_capacity(sections.len());
        for (kind, data) in sections {
            headers.push(SectionHeader {
                kind: *kind,
                offset,
                length: data.len() as u64,
            });
            offset = align8(offset + data.len() as u64);
        }

        // Write section headers
        for h in &headers {
            w.write_u8(h.kind as u8)?;
            w.write_u64::<LittleEndian>(h.offset)?;
            w.write_u64::<LittleEndian>(h.length)?;
        }

        // Pad to first section
        let current = 16 + sections.len() as u64 * super::format::SECTION_HEADER_SIZE as u64;
        write_padding(&mut w, current, headers[0].offset)?;

        // Write sections with alignment padding
        for (i, (_, data)) in sections.iter().enumerate() {
            w.write_all(data)?;
            let end = headers[i].offset + data.len() as u64;
            if i + 1 < headers.len() {
                write_padding(&mut w, end, headers[i + 1].offset)?;
            }
        }

        w.flush()?;
        Ok(())
    }

    /// Index a token for `doc_id`. Also indexes the phonemicized canonical synonym form so that
    /// documents using abbreviations (e.g. "Bd Haussmann") are also findable via the canonical
    /// phonemicized form ("boulevar"). Mirrors addok's index-time synonym expansion.
    fn index_token_with_synonyms(
        &mut self,
        normalized: &str,
        lemma: &str,
        doc_id: DocId,
        score: f32,
        doc_token_ids: &mut Vec<TokenId>,
    ) {
        // Always index the phonemicized form
        self.index_one_token(normalized, doc_id, score, doc_token_ids);

        // If the lemma (pre-phonemicize form) is an abbreviation, also index its canonical
        // phonemicized equivalent so both "bd" and "boulevard" documents share the same token.
        if let Some(canonical) = synonyms_for(lemma) {
            let ph = phonemicize(canonical);
            if ph != normalized {
                self.index_one_token(&ph, doc_id, score, doc_token_ids);
            }
        }
    }

    fn index_one_token(
        &mut self,
        form: &str,
        doc_id: DocId,
        score: f32,
        doc_token_ids: &mut Vec<TokenId>,
    ) {
        let token_id = self.get_or_insert_token(form);
        self.token_scores.entry(token_id).or_default().push((doc_id, score));
        doc_token_ids.push(token_id);
        for ngram in edge_ngrams(form) {
            self.ngram_sets.entry(ngram).or_default().insert(token_id);
        }
    }

    fn get_or_insert_token(&mut self, normalized: &str) -> TokenId {
        if let Some(&id) = self.token_ids.get(normalized) {
            return id;
        }
        let id = self.next_token_id;
        self.next_token_id += 1;
        self.token_ids.insert(normalized.to_string(), id);
        id
    }

    fn build_token_fst(&self) -> Result<Vec<u8>, GeocoderError> {
        let mut sorted: Vec<(&str, u64)> = self
            .token_ids
            .iter()
            .map(|(k, &v)| (k.as_str(), v as u64))
            .collect();
        sorted.sort_unstable_by_key(|(k, _)| *k);

        let mut buf = Vec::new();
        let mut builder = MapBuilder::new(&mut buf)?;
        for (key, val) in sorted {
            builder.insert(key, val)?;
        }
        builder.finish()?;
        Ok(buf)
    }

    fn build_token_scores(&self) -> Result<Vec<u8>, GeocoderError> {
        let mut buf = Vec::new();
        let count = self.next_token_id;
        buf.write_u32::<LittleEndian>(count)?;
        for token_id in 0..count {
            let entries = self
                .token_scores
                .get(&token_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            buf.write_u32::<LittleEndian>(entries.len() as u32)?;
            for (doc_id, score) in entries {
                buf.write_u32::<LittleEndian>(*doc_id)?;
                buf.write_f32::<LittleEndian>(*score)?;
            }
        }
        Ok(buf)
    }

    fn build_keyed_bitmaps(map: &HashMap<String, RoaringBitmap>) -> Result<Vec<u8>, GeocoderError> {
        let mut buf = Vec::new();
        buf.write_u32::<LittleEndian>(map.len() as u32)?;
        for (key, bitmap) in map {
            let key_bytes = key.as_bytes();
            buf.write_u16::<LittleEndian>(key_bytes.len() as u16)?;
            buf.write_all(key_bytes)?;
            let mut bm_buf = Vec::new();
            bitmap.serialize_into(&mut bm_buf)?;
            buf.write_u32::<LittleEndian>(bm_buf.len() as u32)?;
            buf.write_all(&bm_buf)?;
        }
        Ok(buf)
    }

    fn build_pair_sets(&self) -> Result<Vec<u8>, GeocoderError> {
        let mut buf = Vec::new();
        buf.write_u32::<LittleEndian>(self.pair_sets.len() as u32)?;
        for (token_id, bitmap) in &self.pair_sets {
            buf.write_u32::<LittleEndian>(*token_id)?;
            let mut bm_buf = Vec::new();
            bitmap.serialize_into(&mut bm_buf)?;
            buf.write_u32::<LittleEndian>(bm_buf.len() as u32)?;
            buf.write_all(&bm_buf)?;
        }
        Ok(buf)
    }

    fn build_doc_sections(&self) -> Result<(Vec<u8>, Vec<u8>), GeocoderError> {
        let mut offsets = Vec::new();
        let mut blobs = Vec::new();

        offsets.write_u32::<LittleEndian>(self.doc_blobs.len() as u32)?;
        for (doc_id, compressed) in &self.doc_blobs {
            offsets.write_u32::<LittleEndian>(*doc_id)?;
            offsets.write_u64::<LittleEndian>(blobs.len() as u64)?;
            blobs.extend_from_slice(compressed);
        }
        Ok((offsets, blobs))
    }
}

fn align8(offset: u64) -> u64 {
    (offset + 7) & !7
}

fn write_padding<W: Write>(w: &mut W, current: u64, target: u64) -> std::io::Result<()> {
    let padding = (target - current) as usize;
    w.write_all(&vec![0u8; padding])
}
