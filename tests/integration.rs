use geocoder_core::{Document, Geocoder, IndexBuilder, ReverseOpts, SearchOpts, profile};
use serde_json::json;
use tempfile::NamedTempFile;

fn documents() -> Vec<Document> {
    [
        json!({"id": "75102_0001", "type": "street", "name": "Rue de la Paix", "postcode": "75002",
               "citycode": "75102", "city": "Paris", "context": "75, Paris, Île-de-France",
               "importance": 0.8, "lat": 48.8698, "lon": 2.3310,
               "housenumbers": {"15": {"lat": 48.8700, "lon": 2.3312},
                                "15 bis": {"lat": 48.8701, "lon": 2.3313},
                                "17": {"lat": 48.8702, "lon": 2.3314}}}),
        json!({"id": "75111_0002", "type": "street", "name": "Place de la République", "postcode": "75011",
               "citycode": "75111", "city": "Paris", "context": "75, Paris, Île-de-France",
               "importance": 0.9, "lat": 48.8674, "lon": 2.3633}),
        json!({"id": "75108_0003", "type": "street", "name": "Avenue des Champs-Élysées", "postcode": "75008",
               "citycode": "75108", "city": "Paris", "context": "75, Paris, Île-de-France",
               "importance": 0.95, "lat": 48.8698, "lon": 2.3078}),
        json!({"id": "75109_0004", "type": "street", "name": "Boulevard Haussmann", "postcode": "75009",
               "citycode": "75109", "city": "Paris", "context": "75, Paris, Île-de-France",
               "importance": 0.9, "lat": 48.8737, "lon": 2.3320}),
        json!({"id": "33063", "type": "municipality", "name": "Bordeaux", "postcode": ["33000", "33100"],
               "citycode": "33063", "city": "Bordeaux", "context": "33, Gironde, Nouvelle-Aquitaine",
               "importance": 0.8, "lat": 44.8378, "lon": -0.5792}),
        json!({"id": "16015_0005", "type": "street", "name": "Rue de Bordeaux", "postcode": "16000",
               "citycode": "16015", "city": "Angoulême", "context": "16, Charente, Nouvelle-Aquitaine",
               "importance": 0.4, "lat": 45.6461, "lon": 0.1398}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value(v).unwrap())
    .collect()
}

fn geocoder() -> (NamedTempFile, Geocoder) {
    let file = NamedTempFile::new().unwrap();
    let mut builder = IndexBuilder::new(profile::by_name("fr").unwrap());
    for doc in documents() {
        assert!(builder.add_document(doc).unwrap());
    }
    builder.build(file.path()).unwrap();
    let geocoder = Geocoder::open(file.path()).unwrap();
    (file, geocoder)
}

fn top_label(g: &Geocoder, q: &str) -> String {
    g.search(q, SearchOpts::default())
        .first()
        .map(|r| r.label.clone())
        .unwrap_or_default()
}

#[test]
fn stores_the_profile() {
    let (_f, g) = geocoder();
    assert_eq!(g.meta().profile, "fr");
    assert_eq!(g.meta().documents, 6);
}

#[test]
fn finds_a_street_with_its_city() {
    let (_f, g) = geocoder();
    assert_eq!(
        top_label(&g, "rue de la paix paris"),
        "Rue de la Paix 75002 Paris"
    );
}

#[test]
fn synonyms_apply_both_ways() {
    let (_f, g) = geocoder();
    assert_eq!(
        top_label(&g, "bd haussmann"),
        "Boulevard Haussmann 75009 Paris"
    );
    assert_eq!(
        top_label(&g, "av des champs elysees"),
        "Avenue des Champs-Élysées 75008 Paris"
    );
}

#[test]
fn matches_housenumbers_and_ordinals() {
    let (_f, g) = geocoder();
    let r = &g.search("15 rue de la paix paris", SearchOpts::default())[0];
    assert_eq!(r.housenumber.as_deref(), Some("15"));
    assert_eq!(r.label, "15 Rue de la Paix 75002 Paris");
    assert!((r.lat - 48.8700).abs() < 1e-9, "housenumber coordinates");
    assert_eq!(r.doc["type"], "housenumber");

    let r = &g.search("15bis rue de la paix", SearchOpts::default())[0];
    assert_eq!(r.housenumber.as_deref(), Some("15 bis"));
}

#[test]
fn municipality_ranks_first() {
    let (_f, g) = geocoder();
    assert_eq!(top_label(&g, "bordeaux"), "Bordeaux");
    assert_eq!(top_label(&g, "33000 bordeaux"), "Bordeaux");
}

#[test]
fn fuzzy_and_autocomplete() {
    let (_f, g) = geocoder();
    assert_eq!(
        top_label(&g, "place de la repblique paris"),
        "Place de la République 75011 Paris"
    );
    assert_eq!(
        top_label(&g, "place de la rep"),
        "Place de la République 75011 Paris"
    );
}

#[test]
fn filters() {
    let (_f, g) = geocoder();
    let mut opts = SearchOpts::default();
    opts.filters.insert("type".into(), vec!["street".into()]);
    let results = g.search("bordeaux", opts);
    assert_eq!(results[0].label, "Rue de Bordeaux 16000 Angoulême");
    assert!(results.iter().all(|r| r.doc["type"] == "street"));
}

#[test]
fn center_favours_nearby_results() {
    let (_f, g) = geocoder();
    let opts = SearchOpts {
        lat: Some(45.6461),
        lon: Some(0.1398),
        ..Default::default()
    };
    let results = g.search("bordeaux", opts);
    assert!(results[0].distance_km.is_some());
}

#[test]
fn reverse_finds_the_closest_housenumber() {
    let (_f, g) = geocoder();
    let results = g.reverse(48.87011, 2.33131, ReverseOpts::default());
    assert_eq!(results[0].housenumber.as_deref(), Some("15 bis"));
    assert!(results[0].distance_km.unwrap() < 0.01);
}

#[test]
fn rejects_an_index_built_with_another_profile_version() {
    // The version check is what prevents querying an index whose tokens were
    // produced by different processing.
    let (f, g) = geocoder();
    let bytes = std::fs::read(f.path()).unwrap();
    let needle = format!(r#""profile_version":{}"#, g.meta().profile_version).into_bytes();
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
        .unwrap();
    let mut patched = bytes.clone();
    patched[at + needle.len() - 1] = b'9';
    let other = NamedTempFile::new().unwrap();
    std::fs::write(other.path(), patched).unwrap();
    assert!(Geocoder::open(other.path()).is_err());
}
