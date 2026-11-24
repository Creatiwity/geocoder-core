//! CEDEX addresses: the place is searched, the delivery line given back.

use geocoder_core::postal::CedexTableBuilder;
use geocoder_core::{Document, Geocoder, IndexBuilder, SearchOpts, profile};
use serde_json::json;
use tempfile::NamedTempFile;

fn documents() -> Vec<Document> {
    [
        // The same street in two municipalities of the department.
        json!({"id": "92062_4121", "type": "street", "name": "Avenue du Général de Gaulle", "postcode": "92800",
               "citycode": "92062", "city": "Puteaux", "context": "92, Hauts-de-Seine, Île-de-France",
               "importance": 0.5, "lat": 48.8897, "lon": 2.2391,
               "housenumbers": {"1": {"lat": 48.8897, "lon": 2.2392}}}),
        json!({"id": "92023_0100", "type": "street", "name": "Avenue du Général de Gaulle", "postcode": "92140",
               "citycode": "92023", "city": "Clamart", "context": "92, Hauts-de-Seine, Île-de-France",
               "importance": 0.6, "lat": 48.8000, "lon": 2.2600,
               "housenumbers": {"1": {"lat": 48.8001, "lon": 2.2601}}}),
        json!({"id": "33243_0050", "type": "street", "name": "Rue François Vidal", "postcode": "33500",
               "citycode": "33243", "city": "Libourne", "context": "33, Gironde, Nouvelle-Aquitaine",
               "importance": 0.4, "lat": 44.9460, "lon": -0.2426,
               "housenumbers": {"1": {"lat": 44.9461, "lon": -0.2427}}}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect()
}

fn geocoder(with_table: bool) -> (NamedTempFile, Geocoder) {
    let file = NamedTempFile::new().unwrap();
    let mut builder = IndexBuilder::new(profile::by_name("fr").unwrap());
    for doc in documents() {
        assert!(builder.add_document(doc).unwrap());
    }
    if with_table {
        let mut table = CedexTableBuilder::new();
        table.add("92074", "PARIS LA DEFENSE CEDEX", "92062");
        table.add("33506", "LIBOURNE CEDEX", "33243");
        builder.set_cedex(table.build());
    }
    builder.build(file.path()).unwrap();
    let geocoder = Geocoder::open(file.path()).unwrap();
    (file, geocoder)
}

fn opts() -> SearchOpts {
    SearchOpts {
        autocomplete: Some(false),
        ..Default::default()
    }
}

#[test]
fn the_table_is_stored_in_the_index() {
    let (_f, g) = geocoder(true);
    let table = g.index().cedex().unwrap();
    assert_eq!(table.by_code("92074"), ["92062"]);
    // An index without it opens too.
    let (_f, g) = geocoder(false);
    assert!(g.index().cedex().is_none());
}

#[test]
fn a_cedex_finds_the_municipality_it_serves() {
    let (_f, g) = geocoder(true);
    let results = g.search(
        "1, avenue du Général-de-Gaulle, 92074 Paris La Défense Cedex",
        opts(),
    );
    let top = &results[0];
    assert_eq!(top.label, "1 Avenue du Général de Gaulle 92800 Puteaux");
    assert!(top.score > 0.9, "{}", top.score);
    let line = top.postal.as_ref().unwrap();
    assert_eq!(line.line(), "92074 PARIS LA DEFENSE CEDEX");
    assert_eq!(
        top.postal_label.as_deref(),
        Some("1 Avenue du Général de Gaulle 92074 PARIS LA DEFENSE CEDEX")
    );
}

#[test]
fn a_cedex_named_after_its_municipality_scores_like_its_postcode() {
    let (_f, g) = geocoder(true);
    let cedex = g.search("1 rue François Vidal 33506 Libourne Cedex", opts());
    let plain = g.search("1 rue François Vidal 33500 Libourne", opts());
    assert_eq!(cedex[0].label, plain[0].label);
    assert!(cedex[0].score >= plain[0].score - 0.05);
}

#[test]
fn the_line_is_given_back_even_unknown() {
    let (_f, g) = geocoder(false);
    let results = g.search("1 rue François Vidal 33999 Libourne Cedex 12", opts());
    let top = &results[0];
    assert_eq!(top.label, "1 Rue François Vidal 33500 Libourne");
    assert_eq!(
        top.postal_label.as_deref(),
        Some("1 Rue François Vidal 33999 LIBOURNE CEDEX 12")
    );
}

#[test]
fn no_line_without_cedex() {
    let (_f, g) = geocoder(true);
    let results = g.search("1 rue François Vidal 33500 Libourne", opts());
    assert!(results[0].postal.is_none() && results[0].postal_label.is_none());
}
