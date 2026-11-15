//! A tampered or damaged index must fail with an error or give poor
//! results, never panic: the API serves indexes downloaded from object
//! storage, and a panic there takes a request (or a reload) down.

use std::panic::{AssertUnwindSafe, catch_unwind};

use geocoder_core::{Document, Geocoder, IndexBuilder, ReverseOpts, SearchOpts, profile};
use serde_json::json;

fn index_bytes() -> Vec<u8> {
    let docs = [
        json!({"id": "75102_0001", "type": "street", "name": "Rue de la Paix", "postcode": "75002",
               "citycode": "75102", "city": "Paris", "importance": 0.8, "lat": 48.8698, "lon": 2.3310,
               "housenumbers": {"15": {"lat": 48.8700, "lon": 2.3312}, "17": {"lat": 48.8702, "lon": 2.3314}}}),
        json!({"id": "75111_0002", "type": "street", "name": "Place de la République", "postcode": "75011",
               "citycode": "75111", "city": "Paris", "importance": 0.9, "lat": 48.8674, "lon": 2.3633}),
        json!({"id": "33063", "type": "municipality", "name": "Bordeaux", "postcode": "33000",
               "citycode": "33063", "city": "Bordeaux", "importance": 0.8, "lat": 44.8378, "lon": -0.5792}),
        json!({"id": "16015_0005", "type": "street", "name": "Rue de Bordeaux", "postcode": "16000",
               "citycode": "16015", "city": "Angoulême", "importance": 0.4, "lat": 45.6461, "lon": 0.1398}),
    ];
    let mut builder = IndexBuilder::new(profile::by_name("fr").unwrap());
    for doc in docs {
        let doc: Document = serde_json::from_value(doc).unwrap();
        builder.add_document(doc).unwrap();
    }
    let file = tempfile::NamedTempFile::new().unwrap();
    builder.build(file.path()).unwrap();
    std::fs::read(file.path()).unwrap()
}

/// xorshift64*: reproducible, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn exercise(bytes: &[u8]) {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), bytes).unwrap();
    let Ok(g) = Geocoder::open(file.path()) else {
        return;
    };
    for q in [
        "rue de la paix paris",
        "15 rue de la paix",
        "bordeaux",
        "republique",
        "ang",
    ] {
        g.search(q, SearchOpts::default());
    }
    g.reverse(48.87, 2.33, ReverseOpts::default());
    g.reverse(44.83, -0.57, ReverseOpts::default());
}

#[test]
fn corrupted_indexes_never_panic() {
    let original = index_bytes();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut panics = Vec::new();
    let previous_hook = std::panic::take_hook();
    // Where each panic comes from, for the failure message.
    let locations = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let seen = locations.clone();
    std::panic::set_hook(Box::new(move |info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        *seen
            .lock()
            .unwrap()
            .entry(at.unwrap_or_default())
            .or_insert(0) += 1;
    }));
    // CORRUPTED_ROUNDS=100000 for a longer run.
    let rounds = std::env::var("CORRUPTED_ROUNDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3000);
    for round in 0..rounds {
        let mut bytes = original.clone();
        match round % 4 {
            // A few random bytes anywhere.
            0 | 1 => {
                for _ in 0..1 + rng.below(8) {
                    let at = rng.below(bytes.len());
                    bytes[at] = rng.next() as u8;
                }
            }
            // A whole random 8-byte word: offsets and lengths go wild.
            2 => {
                let at = rng.below(bytes.len() - 8);
                bytes[at..at + 8].copy_from_slice(&rng.next().to_le_bytes());
            }
            // Truncated.
            _ => bytes.truncate(rng.below(bytes.len())),
        }
        if catch_unwind(AssertUnwindSafe(|| exercise(&bytes))).is_err() {
            panics.push(round);
        }
    }
    std::panic::set_hook(previous_hook);
    assert!(
        panics.is_empty(),
        "{} panics, rounds {:?}, at {:?}",
        panics.len(),
        &panics[..panics.len().min(20)],
        locations.lock().unwrap()
    );
}
