use std::collections::HashMap;

use geocoder_core::{Document, Geocoder, IndexBuilder, ReverseOpts, SearchOpts};
use tempfile::NamedTempFile;

fn sample_documents() -> Vec<Document> {
    vec![
        Document {
            name: "15 rue de la Paix".to_string(),
            lat: 48.8698,
            lon: 2.3310,
            importance: 0.8,
            housenumbers: vec![],
            filters: HashMap::from([("type".to_string(), "street".to_string())]),
            extra: HashMap::new(),
        },
        Document {
            name: "Place de la République".to_string(),
            lat: 48.8674,
            lon: 2.3633,
            importance: 0.9,
            housenumbers: vec![],
            filters: HashMap::from([
                ("type".to_string(), "place".to_string()),
                ("postcode".to_string(), "75011".to_string()),
            ]),
            extra: HashMap::new(),
        },
        Document {
            name: "Avenue des Champs-Élysées".to_string(),
            lat: 48.8698,
            lon: 2.3078,
            importance: 0.95,
            housenumbers: vec![],
            filters: HashMap::from([("type".to_string(), "street".to_string())]),
            extra: HashMap::new(),
        },
        Document {
            name: "Gare de Lyon".to_string(),
            lat: 48.8447,
            lon: 2.3737,
            importance: 0.85,
            housenumbers: vec![],
            filters: HashMap::from([("type".to_string(), "poi".to_string())]),
            extra: HashMap::new(),
        },
    ]
}

fn build_test_index() -> (NamedTempFile, Geocoder) {
    let file = NamedTempFile::new().unwrap();
    let mut builder = IndexBuilder::new();
    for doc in sample_documents() {
        builder.add_document(doc).unwrap();
    }
    builder.build(file.path()).unwrap();
    let geocoder = Geocoder::open(file.path()).unwrap();
    (file, geocoder)
}

#[test]
fn test_build_and_search() {
    let (_file, geocoder) = build_test_index();

    let results = geocoder.search("republique", SearchOpts::default());
    assert!(!results.is_empty(), "should find Place de la République");
    assert!(
        results[0].doc["name"].as_str().unwrap().contains("République"),
        "top result should be République, got: {}",
        results[0].doc["name"]
    );
}

#[test]
fn test_search_with_geo_hint() {
    let (_file, geocoder) = build_test_index();

    // Searching "avenue" near Champs-Élysées should rank it above other results
    let results = geocoder.search(
        "avenue",
        SearchOpts { lat: Some(48.8698), lon: Some(2.3078), ..Default::default() },
    );
    assert!(!results.is_empty());
    assert!(results[0].doc["name"].as_str().unwrap().contains("Champs"));
}

#[test]
fn test_search_fuzzy() {
    let (_file, geocoder) = build_test_index();

    // "repblique" (missing u) should still find République via fuzzy
    let results = geocoder.search("repblique", SearchOpts::default());
    assert!(!results.is_empty(), "fuzzy should find République from 'repblique'");
}

#[test]
fn test_reverse() {
    let (_file, geocoder) = build_test_index();

    // Reverse near Gare de Lyon (48.8447, 2.3737)
    let results = geocoder.reverse(48.8447, 2.3737, ReverseOpts::default());
    assert!(!results.is_empty(), "reverse should return at least one result");
    assert!(
        results[0].doc["name"].as_str().unwrap().contains("Lyon"),
        "closest result should be Gare de Lyon, got: {}",
        results[0].doc["name"]
    );
    assert!(results[0].distance_km.unwrap() < 1.0, "should be within 1km");
}

#[test]
fn test_filter() {
    let (_file, geocoder) = build_test_index();

    let mut opts = SearchOpts::default();
    opts.filters.insert("type".to_string(), vec!["poi".to_string()]);

    let results = geocoder.search("gare", opts);
    assert!(!results.is_empty());
    assert_eq!(
        results[0].doc["filters"]["type"].as_str().unwrap(),
        "poi"
    );
}
