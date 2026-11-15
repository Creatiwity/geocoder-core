use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;

use byteorder::{LittleEndian, WriteBytesExt};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use fst::MapBuilder;
use roaring::RoaringBitmap;
use serde_json::Value;

use super::format::{MAGIC, Meta, SECTION_HEADER_SIZE, SectionHeader, SectionKind, VERSION};
use super::tie_break;
use crate::doc::{Document, first_f64, py_str, truthy};
use crate::error::GeocoderError;
use crate::profile::{Profile, preprocess};
use crate::settings::{GEOHASH_PRECISION, IMPORTANCE_WEIGHT, MAX_EDGE_NGRAMS, MIN_EDGE_NGRAMS};
use crate::text::edge_ngrams;
use crate::types::{DocId, TokenId};

/// Builds an index the way addok's batch import does: `prepare_housenumbers`,
/// then the housenumbers, fields, pairs, filters and geohash indexers, then
/// `addok ngrams`.
pub struct IndexBuilder {
    profile: Arc<dyn Profile>,
    token_ids: HashMap<String, TokenId>,
    token_strings: Vec<String>,
    postings: Vec<Vec<(DocId, f32)>>,
    pairs: Vec<RoaringBitmap>,
    geohashes: HashMap<String, RoaringBitmap>,
    filters: HashMap<String, RoaringBitmap>,
    doc_offsets: Vec<u64>,
    doc_blobs: Vec<u8>,
    /// `addok.helpers.index._CACHE`: field values repeat a lot.
    cache: HashMap<String, Vec<String>>,
    skipped: usize,
}

impl IndexBuilder {
    pub fn new(profile: Arc<dyn Profile>) -> Self {
        Self {
            profile,
            token_ids: HashMap::new(),
            token_strings: Vec::new(),
            postings: Vec::new(),
            pairs: Vec::new(),
            geohashes: HashMap::new(),
            filters: HashMap::new(),
            doc_offsets: vec![0],
            doc_blobs: Vec::new(),
            cache: HashMap::new(),
            skipped: 0,
        }
    }

    pub fn documents(&self) -> usize {
        self.doc_offsets.len() - 1
    }

    /// Documents rejected so far (missing required field or coordinates).
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    fn preprocess(&mut self, text: &str) -> Vec<String> {
        if let Some(hit) = self.cache.get(text) {
            return hit.clone();
        }
        let tokens: Vec<String> = preprocess(self.profile.as_ref(), text)
            .into_iter()
            .map(|t| t.value)
            .collect();
        if self.cache.len() > 2_000_000 {
            self.cache.clear();
        }
        self.cache.insert(text.to_string(), tokens.clone());
        tokens
    }

    /// Index one document. Returns `false` when it is rejected.
    pub fn add_document(&mut self, mut doc: Document) -> Result<bool, GeocoderError> {
        let profile = Arc::clone(&self.profile);
        if profile
            .fields()
            .iter()
            .any(|f| f.required && !doc.get(f.key).is_some_and(truthy))
        {
            self.skipped += 1;
            return Ok(false);
        }
        let (Some(lat), Some(lon)) = (first_f64(&doc, "lat"), first_f64(&doc, "lon")) else {
            self.skipped += 1;
            return Ok(false);
        };
        let doc_id = self.documents() as DocId;

        // prepare_housenumbers: key them the way queries will read them.
        let mut housenumber_cells = Vec::new();
        if let Some(Value::Object(housenumbers)) = doc.remove("housenumbers") {
            let mut prepared = serde_json::Map::new();
            for (number, data) in housenumbers {
                let key = self.preprocess(&number).concat();
                let mut data = match data {
                    Value::Object(o) => o,
                    _ => continue,
                };
                if let (Some(lat), Some(lon)) = (first_f64(&data, "lat"), first_f64(&data, "lon")) {
                    housenumber_cells.push(encode_geohash(lat, lon)?);
                }
                data.insert("raw".into(), Value::String(number));
                prepared.insert(key, Value::Object(data));
            }
            doc.insert("housenumbers".into(), Value::Object(prepared));
        }
        let has_housenumbers = doc.get("housenumbers").is_some_and(truthy);

        let tokens = self.field_tokens(&doc);
        let ids: Vec<TokenId> = tokens.iter().map(|(t, _)| self.token_id(t)).collect();
        for (&id, (_, boost)) in ids.iter().zip(&tokens) {
            self.postings[id as usize].push((doc_id, *boost as f32));
        }
        // PairsIndexer.
        for &id in &ids {
            let pairs = &mut self.pairs[id as usize];
            for &other in &ids {
                if other != id {
                    pairs.insert(other);
                }
            }
        }

        // FiltersIndexer.
        for &name in profile.filters() {
            let Some(values) = doc.get(name).filter(|v| truthy(v)) else {
                continue;
            };
            let values: Vec<String> = match values {
                Value::Array(a) => a.iter().map(py_str).collect(),
                v => vec![py_str(v)],
            };
            for value in values {
                self.filters
                    .entry(format!("{name}|{value}"))
                    .or_default()
                    .insert(doc_id);
            }
        }
        if has_housenumbers && profile.filters().contains(&"type") {
            self.filters
                .entry("type|housenumber".into())
                .or_default()
                .insert(doc_id);
        }

        // HousenumbersIndexer and GeohashIndexer.
        housenumber_cells.push(encode_geohash(lat, lon)?);
        for cell in housenumber_cells {
            self.geohashes.entry(cell).or_default().insert(doc_id);
        }

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        serde_json::to_writer(&mut encoder, &doc)?;
        self.doc_blobs.extend_from_slice(&encoder.finish()?);
        self.doc_offsets.push(self.doc_blobs.len() as u64);
        Ok(true)
    }

    /// `FieldsIndexer`: the document's tokens with their boost, each token
    /// keeping its best boost across fields.
    pub fn field_tokens(&mut self, doc: &Document) -> Vec<(String, f64)> {
        let profile = Arc::clone(&self.profile);
        let importance = first_f64(doc, "importance").unwrap_or(0.0) * IMPORTANCE_WEIGHT;
        let mut tokens: Vec<(String, f64)> = Vec::new();
        for field in profile.fields() {
            let Some(values) = doc.get(field.key).filter(|v| truthy(v)) else {
                continue;
            };
            let boost = field.boost.value(doc) + importance;
            let values: Vec<String> = match values {
                Value::Array(a) => a.iter().map(py_str).collect(),
                v => vec![py_str(v)],
            };
            for value in values {
                let els = self.preprocess(&value);
                if els.is_empty() {
                    continue;
                }
                let token_boost = 1.0 / els.len() as f64 * boost;
                for token in els {
                    match tokens.iter_mut().find(|(t, _)| *t == token) {
                        Some((_, b)) if *b < token_boost => *b = token_boost,
                        Some(_) => {}
                        None => tokens.push((token, token_boost)),
                    }
                }
            }
        }
        tokens
    }

    fn token_id(&mut self, token: &str) -> TokenId {
        if let Some(&id) = self.token_ids.get(token) {
            return id;
        }
        let id = self.token_strings.len() as TokenId;
        self.token_ids.insert(token.to_string(), id);
        self.token_strings.push(token.to_string());
        self.postings.push(Vec::new());
        self.pairs.push(RoaringBitmap::new());
        id
    }

    /// Write the index to `path`.
    pub fn build(self, path: &Path) -> Result<(), GeocoderError> {
        let meta = Meta {
            profile: self.profile.name().to_string(),
            profile_version: self.profile.version(),
            documents: self.documents() as u32,
            tokens: self.token_strings.len() as u32,
        };

        let token_fst = {
            let mut sorted: Vec<(&str, u64)> = self
                .token_strings
                .iter()
                .enumerate()
                .map(|(i, t)| (t.as_str(), i as u64))
                .collect();
            sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));
            build_fst(sorted)?
        };

        // Postings: by doc as indexed, and ordered like ZREVRANGE.
        let mut posting_offsets = Vec::with_capacity((self.postings.len() + 1) * 8);
        let mut by_doc = Vec::new();
        let mut by_score = Vec::new();
        let mut entries = 0u64;
        posting_offsets.write_u64::<LittleEndian>(0)?;
        for list in &self.postings {
            for &(doc, boost) in list {
                by_doc.write_u32::<LittleEndian>(doc)?;
                by_doc.write_f32::<LittleEndian>(boost)?;
            }
            let mut ordered = list.clone();
            ordered.sort_unstable_by(|a, b| rank(a.1 as f64, a.0, b.1 as f64, b.0));
            for (doc, _) in ordered {
                by_score.write_u32::<LittleEndian>(doc)?;
            }
            entries += list.len() as u64;
            posting_offsets.write_u64::<LittleEndian>(entries)?;
        }

        let (pair_offsets, pair_blobs) = bitmap_table(self.pairs.iter())?;

        // Edge ngrams of every token, numbers included: the Géoplateforme
        // imports through stdin (`addok batch < file`), which keeps
        // `EdgeNgramIndexer` on; `addok ngrams` then only adds the non
        // numeric ones again. So "268" completes to the postcode "26800".
        let mut ngrams: HashMap<String, RoaringBitmap> = HashMap::new();
        for (id, token) in self.token_strings.iter().enumerate() {
            for ngram in edge_ngrams(token, MIN_EDGE_NGRAMS, MAX_EDGE_NGRAMS) {
                ngrams.entry(ngram).or_default().insert(id as u32);
            }
        }
        let (ngram_fst, ngram_offsets, ngram_blobs) = keyed_bitmaps(ngrams)?;
        let (geohash_fst, geohash_offsets, geohash_blobs) = keyed_bitmaps(self.geohashes)?;
        let (filter_fst, filter_offsets, filter_blobs) = keyed_bitmaps(self.filters)?;

        let mut doc_offsets = Vec::with_capacity(self.doc_offsets.len() * 8);
        for offset in &self.doc_offsets {
            doc_offsets.write_u64::<LittleEndian>(*offset)?;
        }

        let meta_bytes = serde_json::to_vec(&meta)?;
        let sections: Vec<(SectionKind, &[u8])> = vec![
            (SectionKind::Meta, &meta_bytes),
            (SectionKind::TokenFst, &token_fst),
            (SectionKind::PostingOffsets, &posting_offsets),
            (SectionKind::PostingsByDoc, &by_doc),
            (SectionKind::PostingsByScore, &by_score),
            (SectionKind::PairOffsets, &pair_offsets),
            (SectionKind::PairBlobs, &pair_blobs),
            (SectionKind::NgramFst, &ngram_fst),
            (SectionKind::NgramOffsets, &ngram_offsets),
            (SectionKind::NgramBlobs, &ngram_blobs),
            (SectionKind::GeohashFst, &geohash_fst),
            (SectionKind::GeohashOffsets, &geohash_offsets),
            (SectionKind::GeohashBlobs, &geohash_blobs),
            (SectionKind::FilterFst, &filter_fst),
            (SectionKind::FilterOffsets, &filter_offsets),
            (SectionKind::FilterBlobs, &filter_blobs),
            (SectionKind::DocOffsets, &doc_offsets),
            (SectionKind::DocBlobs, &self.doc_blobs),
        ];
        write_sections(path, &sections)
    }
}

/// Redis ordering of a sorted set read with `ZREVRANGE`: score descending,
/// then member descending. Members are `d|<import order>`, compared as text.
pub(crate) fn rank(score_a: f64, doc_a: DocId, score_b: f64, doc_b: DocId) -> Ordering {
    score_b
        .partial_cmp(&score_a)
        .unwrap_or(Ordering::Equal)
        .then_with(|| tie_break(doc_b).cmp(&tie_break(doc_a)))
}

fn encode_geohash(lat: f64, lon: f64) -> Result<String, GeocoderError> {
    Ok(geohash::encode(
        geohash::Coord { x: lon, y: lat },
        GEOHASH_PRECISION,
    )?)
}

fn build_fst(sorted: Vec<(&str, u64)>) -> Result<Vec<u8>, GeocoderError> {
    let mut builder = MapBuilder::memory();
    for (key, value) in sorted {
        builder.insert(key, value)?;
    }
    Ok(builder.into_inner()?)
}

fn bitmap_table<'a>(
    bitmaps: impl Iterator<Item = &'a RoaringBitmap>,
) -> Result<(Vec<u8>, Vec<u8>), GeocoderError> {
    let mut offsets = Vec::new();
    let mut blobs = Vec::new();
    offsets.write_u64::<LittleEndian>(0)?;
    for bitmap in bitmaps {
        bitmap.serialize_into(&mut blobs)?;
        offsets.write_u64::<LittleEndian>(blobs.len() as u64)?;
    }
    Ok((offsets, blobs))
}

/// FST, offsets and bitmaps sections of a keyed bitmap table.
type KeyedSections = (Vec<u8>, Vec<u8>, Vec<u8>);

fn keyed_bitmaps(map: HashMap<String, RoaringBitmap>) -> Result<KeyedSections, GeocoderError> {
    let mut entries: Vec<(String, RoaringBitmap)> = map.into_iter().collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let fst = build_fst(
        entries
            .iter()
            .enumerate()
            .map(|(i, (k, _))| (k.as_str(), i as u64))
            .collect(),
    )?;
    let (offsets, blobs) = bitmap_table(entries.iter().map(|(_, b)| b))?;
    Ok((fst, offsets, blobs))
}

fn write_sections(path: &Path, sections: &[(SectionKind, &[u8])]) -> Result<(), GeocoderError> {
    let mut w = BufWriter::new(std::fs::File::create(path)?);
    w.write_all(MAGIC)?;
    w.write_u32::<LittleEndian>(VERSION)?;
    w.write_u32::<LittleEndian>(sections.len() as u32)?;

    let headers_end = (16 + sections.len() * SECTION_HEADER_SIZE) as u64;
    let mut offset = align8(headers_end);
    let mut headers = Vec::with_capacity(sections.len());
    for (kind, data) in sections {
        headers.push(SectionHeader {
            kind: *kind,
            offset,
            length: data.len() as u64,
        });
        offset = align8(offset + data.len() as u64);
    }
    for h in &headers {
        w.write_u8(h.kind as u8)?;
        w.write_u64::<LittleEndian>(h.offset)?;
        w.write_u64::<LittleEndian>(h.length)?;
    }
    let mut position = headers_end;
    for (h, (_, data)) in headers.iter().zip(sections) {
        w.write_all(&vec![0u8; (h.offset - position) as usize])?;
        w.write_all(data)?;
        position = h.offset + data.len() as u64;
    }
    w.flush()?;
    Ok(())
}

fn align8(offset: u64) -> u64 {
    (offset + 7) & !7
}
