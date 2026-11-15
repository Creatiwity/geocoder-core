//! Address registers published one row per address (Luxembourg's
//! BD-Adresses, Belgium's BeST) turned into addok documents.
//!
//! addok indexes streets carrying their housenumbers, plus municipalities
//! and localities. The rows are grouped by street; a housenumber whose
//! postcode or city differs from its street's carries its own (addok merges
//! the housenumber's fields into the result), so one street crossing two
//! postcodes stays one document and each label is right. Then:
//!
//! - `street`: positioned on its housenumber closest to their centroid, so
//!   the point lies on the street;
//! - `municipality`: one per municipality, on its housenumber closest to
//!   the centroid of all of them;
//! - `locality`: the localities (Luxembourg) or postal names (Belgium)
//!   differing from their municipality, same positioning.
//!
//! The registers carry no importance: it is derived from the number of
//! addresses, on a log scale, like a population proxy.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::doc::Document;

/// One address of a register.
#[derive(Debug, Clone, Default)]
pub struct AddressRow {
    /// Register identifier of the address.
    pub id: String,
    pub street_id: String,
    /// Street names, the main language first.
    pub street_names: Vec<String>,
    /// As written, e.g. "62A".
    pub housenumber: String,
    pub postcode: String,
    /// What the address label shows as the city (locality or postal name),
    /// the main language first.
    pub city_names: Vec<String>,
    pub municipality_id: String,
    pub municipality_names: Vec<String>,
    /// "Clervaux", "Bruxelles-Capitale"…
    pub context: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Default)]
struct Area {
    names: Vec<String>,
    municipality_id: String,
    municipality_names: Vec<String>,
    postcodes: Vec<String>,
    context: String,
    points: Vec<(f64, f64)>,
}

impl Area {
    fn add(&mut self, row: &AddressRow) {
        if !self.postcodes.contains(&row.postcode) {
            self.postcodes.push(row.postcode.clone());
        }
        self.points.push((row.lat, row.lon));
    }
}

#[derive(Default)]
struct Street {
    id: String,
    names: Vec<String>,
    /// The first one is the street's, the others its housenumbers'.
    postcodes: Vec<String>,
    cities: Vec<String>,
    municipality_id: String,
    context: String,
    /// First occurrence kept (box numbers and several buildings share a
    /// number).
    housenumbers: Vec<Housenumber>,
}

struct Housenumber {
    number: String,
    id: String,
    lat: f64,
    lon: f64,
    /// Only when they differ from the street's.
    postcode: Option<String>,
    cities: Option<Vec<String>>,
}

/// Group the rows into documents; `id_prefix` keeps identifiers unique
/// across countries ("lu", "be").
pub fn documents(rows: impl IntoIterator<Item = AddressRow>, id_prefix: &str) -> Vec<Document> {
    let mut grouper = Grouper::new(id_prefix);
    for row in rows {
        grouper.add(row);
    }
    grouper.finish()
}

/// [`documents`], one row at a time: a register is never held in memory,
/// only its streets.
pub struct Grouper {
    id_prefix: String,
    streets: HashMap<String, Street>,
    street_order: Vec<String>,
    municipalities: HashMap<String, Area>,
    localities: HashMap<(String, String), Area>,
}

impl Grouper {
    pub fn new(id_prefix: &str) -> Self {
        Self {
            id_prefix: id_prefix.to_string(),
            streets: HashMap::new(),
            street_order: Vec::new(),
            municipalities: HashMap::new(),
            localities: HashMap::new(),
        }
    }

    pub fn add(&mut self, row: AddressRow) {
        let Self {
            streets,
            street_order,
            municipalities,
            localities,
            ..
        } = self;
        if row.street_names.is_empty() || row.housenumber.is_empty() {
            return;
        }
        let street = streets.entry(row.street_id.clone()).or_insert_with(|| {
            street_order.push(row.street_id.clone());
            Street {
                id: row.street_id.clone(),
                names: row.street_names.clone(),
                postcodes: Vec::new(),
                cities: row.city_names.clone(),
                municipality_id: row.municipality_id.clone(),
                context: row.context.clone(),
                housenumbers: Vec::new(),
            }
        });
        if !street.postcodes.contains(&row.postcode) {
            street.postcodes.push(row.postcode.clone());
        }
        if !street
            .housenumbers
            .iter()
            .any(|h| h.number.eq_ignore_ascii_case(&row.housenumber))
        {
            let postcode = (row.postcode != street.postcodes[0]).then(|| row.postcode.clone());
            let cities = (row.city_names != street.cities).then(|| row.city_names.clone());
            street.housenumbers.push(Housenumber {
                number: row.housenumber.clone(),
                id: row.id.clone(),
                lat: row.lat,
                lon: row.lon,
                postcode,
                cities,
            });
        }

        municipalities
            .entry(row.municipality_id.clone())
            .or_insert_with(|| Area {
                names: row.municipality_names.clone(),
                municipality_id: row.municipality_id.clone(),
                municipality_names: row.municipality_names.clone(),
                context: row.context.clone(),
                ..Default::default()
            })
            .add(&row);

        let is_municipality = row
            .city_names
            .first()
            .is_none_or(|c| row.municipality_names.contains(c));
        if !is_municipality {
            localities
                .entry((row.municipality_id.clone(), row.city_names[0].clone()))
                .or_insert_with(|| Area {
                    names: row.city_names.clone(),
                    municipality_id: row.municipality_id.clone(),
                    municipality_names: row.municipality_names.clone(),
                    context: row.context.clone(),
                    ..Default::default()
                })
                .add(&row);
        }
    }

    pub fn finish(self) -> Vec<Document> {
        let Self {
            id_prefix,
            streets,
            street_order,
            municipalities,
            localities,
        } = self;
        let id_prefix = id_prefix.as_str();
        let max_area = municipalities
            .values()
            .map(|a| a.points.len())
            .max()
            .unwrap_or(1);
        let max_street = streets
            .values()
            .map(|s| s.housenumbers.len())
            .max()
            .unwrap_or(1);
        let municipality_size: HashMap<&str, usize> = municipalities
            .iter()
            .map(|(id, a)| (id.as_str(), a.points.len()))
            .collect();

        let mut docs = Vec::with_capacity(streets.len() + municipalities.len() + localities.len());

        let mut municipality_ids: Vec<&String> = municipalities.keys().collect();
        municipality_ids.sort();
        for id in municipality_ids {
            let area = &municipalities[id];
            let (lat, lon) = central(&area.points);
            let mut doc = Map::new();
            doc.insert("id".into(), json!(format!("{id_prefix}_{id}")));
            doc.insert("type".into(), json!("municipality"));
            doc.insert("name".into(), names(&area.names));
            doc.insert("postcode".into(), names(&area.postcodes));
            doc.insert("citycode".into(), json!(area.municipality_id));
            doc.insert("city".into(), names(&area.municipality_names));
            doc.insert("context".into(), json!(area.context));
            doc.insert(
                "importance".into(),
                json!(round(log_ratio(area.points.len(), max_area))),
            );
            doc.insert("lat".into(), json!(lat));
            doc.insert("lon".into(), json!(lon));
            docs.push(doc);
        }

        let mut locality_keys: Vec<&(String, String)> = localities.keys().collect();
        locality_keys.sort();
        for key in locality_keys {
            let area = &localities[key];
            let (lat, lon) = central(&area.points);
            let mut doc = Map::new();
            doc.insert(
                "id".into(),
                json!(format!(
                    "{id_prefix}_{}_{}",
                    area.municipality_id,
                    slug(&key.1)
                )),
            );
            doc.insert("type".into(), json!("locality"));
            doc.insert("name".into(), names(&area.names));
            doc.insert("postcode".into(), names(&area.postcodes));
            doc.insert("citycode".into(), json!(area.municipality_id));
            doc.insert("city".into(), names(&area.municipality_names));
            doc.insert("context".into(), json!(area.context));
            doc.insert(
                "importance".into(),
                json!(round(0.8 * log_ratio(area.points.len(), max_area))),
            );
            doc.insert("lat".into(), json!(lat));
            doc.insert("lon".into(), json!(lon));
            docs.push(doc);
        }

        for key in street_order {
            let street = &streets[&key];
            let points: Vec<(f64, f64)> =
                street.housenumbers.iter().map(|h| (h.lat, h.lon)).collect();
            let (lat, lon) = central(&points);
            let municipality = municipality_size
                .get(street.municipality_id.as_str())
                .copied()
                .unwrap_or(1);
            // Mostly the size of the municipality, a little the street's.
            let importance = 0.7 * log_ratio(municipality, max_area)
                + 0.3 * log_ratio(street.housenumbers.len(), max_street);
            let housenumbers: Map<String, Value> = street
                .housenumbers
                .iter()
                .map(|h| {
                    let mut data = Map::new();
                    data.insert("id".into(), json!(format!("{id_prefix}_{}", h.id)));
                    data.insert("lat".into(), json!(h.lat));
                    data.insert("lon".into(), json!(h.lon));
                    if let Some(postcode) = &h.postcode {
                        data.insert("postcode".into(), json!(postcode));
                    }
                    if let Some(cities) = &h.cities {
                        data.insert("city".into(), names(cities));
                    }
                    (h.number.clone(), Value::Object(data))
                })
                .collect();
            let mut doc = Map::new();
            doc.insert("id".into(), json!(format!("{id_prefix}_{}", street.id)));
            doc.insert("type".into(), json!("street"));
            doc.insert("name".into(), names(&street.names));
            doc.insert("postcode".into(), names(&street.postcodes));
            doc.insert("citycode".into(), json!(street.municipality_id));
            doc.insert("city".into(), names(&street.cities));
            doc.insert("context".into(), json!(street.context));
            doc.insert("importance".into(), json!(round(importance)));
            doc.insert("lat".into(), json!(lat));
            doc.insert("lon".into(), json!(lon));
            doc.insert("housenumbers".into(), Value::Object(housenumbers));
            docs.push(doc);
        }
        docs
    }
}

/// One name as a string, several as a list (addok: default then aliases).
fn names(values: &[String]) -> Value {
    match values {
        [one] => json!(one),
        _ => json!(values),
    }
}

/// The point closest to the centroid: always a real address.
fn central(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len().max(1) as f64;
    let (clat, clon) = points
        .iter()
        .fold((0.0, 0.0), |(a, b), (lat, lon)| (a + lat / n, b + lon / n));
    points
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (a.0 - clat).powi(2) + (a.1 - clon).powi(2);
            let db = (b.0 - clat).powi(2) + (b.1 - clon).powi(2);
            da.total_cmp(&db)
        })
        .unwrap_or((clat, clon))
}

/// `ln(1 + n) / ln(1 + max)`, in [0, 1].
fn log_ratio(n: usize, max: usize) -> f64 {
    if max == 0 {
        return 0.0;
    }
    ((1 + n) as f64).ln() / ((1 + max) as f64).ln()
}

fn round(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn slug(value: &str) -> String {
    crate::text::ascii(value).replace(' ', "-")
}

/// Distinct non-empty values, in order.
pub fn distinct(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in values {
        let v = v.trim().to_string();
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(street: &str, number: &str, postcode: &str, city: &str, lat: f64) -> AddressRow {
        AddressRow {
            id: format!("{street}{number}"),
            street_id: street.into(),
            street_names: vec![format!("Rue {street}")],
            housenumber: number.into(),
            postcode: postcode.into(),
            city_names: vec![city.into()],
            municipality_id: "0501".into(),
            municipality_names: vec!["Clervaux".into()],
            context: "Clervaux".into(),
            lat,
            lon: 6.0,
        }
    }

    #[test]
    fn groups_streets_municipalities_and_localities() {
        let docs = documents(
            [
                row("A", "1", "9755", "Hupperdange", 50.0),
                row("A", "3", "9755", "Hupperdange", 50.002),
                row("A", "3", "9755", "Hupperdange", 50.003),
                row("A", "5", "9756", "Hupperdange", 50.004),
                row("B", "2", "9701", "Clervaux", 50.1),
            ],
            "lu",
        );
        let kinds: Vec<(&str, &str)> = docs
            .iter()
            .map(|d| (d["type"].as_str().unwrap(), d["id"].as_str().unwrap()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("municipality", "lu_0501"),
                ("locality", "lu_0501_hupperdange"),
                ("street", "lu_A"),
                ("street", "lu_B"),
            ]
        );
        // Duplicated number kept once, the first one.
        let a = &docs[2];
        assert_eq!(a["housenumbers"].as_object().unwrap().len(), 3);
        assert_eq!(a["housenumbers"]["3"]["lat"], 50.002);
        assert_eq!(a["city"], "Hupperdange");
        // The street spans two postcodes: the housenumber carries its own.
        assert_eq!(a["postcode"], json!(["9755", "9756"]));
        assert!(a["housenumbers"]["3"].get("postcode").is_none());
        assert_eq!(a["housenumbers"]["5"]["postcode"], "9756");
        assert_eq!(docs[0]["postcode"], json!(["9755", "9756", "9701"]));
        assert_eq!(docs[0]["importance"], 1.0);
    }
}
