use std::collections::HashMap;

use geocoder_core::{Document, HouseNumber, Geocoder, IndexBuilder, ReverseOpts, SearchOpts};
use tempfile::NamedTempFile;

fn sample_documents() -> Vec<Document> {
    vec![
        Document {
            name: "rue de la Paix".to_string(),
            lat: 48.8698,
            lon: 2.3310,
            importance: 0.8,
            housenumbers: vec![
                HouseNumber { number: "15".to_string(), lat: 48.8700, lon: 2.3312 },
                HouseNumber { number: "15bis".to_string(), lat: 48.8701, lon: 2.3313 },
                HouseNumber { number: "17".to_string(), lat: 48.8702, lon: 2.3314 },
            ],
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
fn test_synonym_long_to_short() {
    let (_file, geocoder) = build_test_index();
    // "avenue" → indexed as both "avenue" and "av"
    // User queries "av champs" → should match "Avenue des Champs-Élysées"
    let results = geocoder.search("av champs", SearchOpts::default());
    assert!(!results.is_empty(), "synonym 'av' should match 'avenue'");
    assert!(
        results[0].doc["name"].as_str().unwrap().contains("Champs"),
        "got: {}",
        results[0].doc["name"]
    );
}

#[test]
fn test_synonym_short_to_long() {
    let (_file, geocoder) = build_test_index();
    // User queries "avenue champs" → "avenue" resolves to same token as "av"
    let results = geocoder.search("avenue champs", SearchOpts::default());
    assert!(!results.is_empty(), "synonym 'avenue' should match 'av'");
    assert!(results[0].doc["name"].as_str().unwrap().contains("Champs"));
}

#[test]
fn test_housenumber_match_coordinates() {
    let (_file, geocoder) = build_test_index();
    // "15 rue paix" should find the street AND use housenumber 15's coordinates
    let results = geocoder.search("15 rue paix", SearchOpts::default());
    assert!(!results.is_empty(), "should find rue de la Paix");
    let top = &results[0];
    assert_eq!(
        top.matched_housenumber.as_deref(),
        Some("15"),
        "should have matched housenumber 15"
    );
    // Coordinates should be housenumber's, not street center
    assert!((top.doc["lat"].as_f64().unwrap() - 48.8698).abs() < 0.001);
}

#[test]
fn test_housenumber_prefix_match() {
    let (_file, geocoder) = build_test_index();
    let results = geocoder.search("15b rue paix", SearchOpts::default());
    assert!(!results.is_empty());
    assert_eq!(results[0].matched_housenumber.as_deref(), Some("15bis"));
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
