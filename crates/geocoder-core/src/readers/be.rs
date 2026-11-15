//! Belgium: BeST Address of the FPS BOSA (CC BY 4.0), one CSV per region
//! (`openaddress-bevlg.zip`, `-bewal`, `-bebru`), one row per address and
//! box number:
//!
//! `EPSG:3812_x,EPSG:3812_y,EPSG:4326_lat,EPSG:4326_lon,address_id,
//! box_number,house_number,municipality_id,municipality_name_de,
//! municipality_name_fr,municipality_name_nl,postcode,postname_fr,
//! postname_nl,street_id,streetname_de,streetname_fr,streetname_nl,
//! region_code,status`
//!
//! Names are bilingual or trilingual: they are all indexed, the region's
//! language first (Dutch in Flanders, French in Wallonia and Brussels). The
//! label city is the postal name ("Berchem"), else the municipality.

use std::io::Read;

use serde::Deserialize;

use super::grouped::{AddressRow, distinct};
use crate::error::GeocoderError;

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "EPSG:4326_lat")]
    lat: Option<f64>,
    #[serde(rename = "EPSG:4326_lon")]
    lon: Option<f64>,
    address_id: String,
    house_number: String,
    municipality_id: String,
    municipality_name_de: String,
    municipality_name_fr: String,
    municipality_name_nl: String,
    postcode: String,
    postname_fr: String,
    postname_nl: String,
    street_id: String,
    streetname_de: String,
    streetname_fr: String,
    streetname_nl: String,
    region_code: String,
    status: String,
}

#[derive(Clone, Copy)]
enum Lang {
    Fr,
    Nl,
    De,
}

/// Rows of one regional file; returns the rows and the invalid ones.
pub fn rows(reader: impl Read) -> Result<(Vec<AddressRow>, usize), GeocoderError> {
    let mut out = Vec::new();
    let invalid = for_each_row(reader, |row| out.push(row))?;
    Ok((out, invalid))
}

/// Call `f` on each row of one regional file; returns the number of invalid
/// rows. Retired and proposed addresses are left out (`status` other than
/// `current`).
pub fn for_each_row(
    reader: impl Read,
    mut f: impl FnMut(AddressRow),
) -> Result<usize, GeocoderError> {
    let mut csv = csv::ReaderBuilder::new().from_reader(super::skip_bom(reader));
    let mut invalid = 0;
    for record in csv.deserialize::<Row>() {
        let Ok(row) = record else {
            invalid += 1;
            continue;
        };
        if row.status != "current" {
            continue;
        }
        let (Some(lat), Some(lon)) = (row.lat, row.lon) else {
            invalid += 1;
            continue;
        };
        let (order, region, code) = match row.region_code.as_str() {
            "BE-VLG" => ([Lang::Nl, Lang::Fr, Lang::De], "Vlaanderen", "vlg"),
            "BE-WAL" => ([Lang::Fr, Lang::De, Lang::Nl], "Wallonie", "wal"),
            _ => ([Lang::Fr, Lang::Nl, Lang::De], "Bruxelles-Capitale", "bru"),
        };
        let pick = |fr: &str, nl: &str, de: &str| {
            distinct(order.iter().map(|lang| {
                match lang {
                    Lang::Fr => fr,
                    Lang::Nl => nl,
                    Lang::De => de,
                }
                .to_string()
            }))
        };
        let street_names = pick(&row.streetname_fr, &row.streetname_nl, &row.streetname_de);
        let municipality_names = pick(
            &row.municipality_name_fr,
            &row.municipality_name_nl,
            &row.municipality_name_de,
        );
        let postal_names = distinct(
            pick(&row.postname_fr, &row.postname_nl, "")
                .iter()
                .flat_map(|name| postal_name(name)),
        );
        let city_names = if postal_names.is_empty() {
            municipality_names.clone()
        } else {
            postal_names
        };
        let context = match municipality_names.first() {
            Some(m) => format!("{m}, {region}"),
            None => region.to_string(),
        };
        f(AddressRow {
            // Identifiers are only unique within a region.
            id: format!("{code}_{}", row.address_id),
            street_id: format!("{code}_{}", row.street_id),
            street_names,
            housenumber: row.house_number.trim().to_string(),
            postcode: row.postcode.trim().to_string(),
            city_names,
            municipality_id: row.municipality_id,
            municipality_names,
            context,
            lat,
            lon,
        });
    }
    Ok(invalid)
}

/// Postal names come as typed by bpost: "ANTWERPEN", "Liège_",
/// "OOSTENDE/Stene/Zandvoorde" (one postcode, several names).
fn postal_name(raw: &str) -> Vec<String> {
    raw.split('/')
        .map(|name| {
            let name = name.trim().trim_end_matches('_').trim();
            let has_lower = name.chars().any(char::is_lowercase);
            if has_lower {
                name.to_string()
            } else {
                title_case(name)
            }
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// "SINT-NIKLAAS" → "Sint-Niklaas", "L'ESCAILLERE" → "L'Escaillere".
fn title_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut start = true;
    for c in name.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start = matches!(c, ' ' | '-' | '\'' | '(');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readers::grouped::documents;

    const HEADER: &str = "EPSG:3812_x,EPSG:3812_y,EPSG:4326_lat,EPSG:4326_lon,address_id,box_number,house_number,municipality_id,municipality_name_de,municipality_name_fr,municipality_name_nl,postcode,postname_fr,postname_nl,street_id,streetname_de,streetname_fr,streetname_nl,region_code,status";

    #[test]
    fn cleans_postal_names() {
        assert_eq!(postal_name("ANTWERPEN"), ["Antwerpen"]);
        assert_eq!(postal_name("Liège_"), ["Liège"]);
        assert_eq!(
            postal_name("OOSTENDE/Stene/Zandvoorde"),
            ["Oostende", "Stene", "Zandvoorde"]
        );
        assert_eq!(postal_name("SINT-NIKLAAS"), ["Sint-Niklaas"]);
    }

    #[test]
    fn reads_bilingual_rows() {
        let csv = format!(
            "{HEADER}
646320.884,669504.711,50.83595,4.31653,615867,RDC,475,21001,,Anderlecht,Anderlecht,1070,,,4568,,Chaussée de Mons,Bergense Steenweg,BE-BRU,current
646320.884,669504.711,50.83595,4.31653,615869,1,475,21001,,Anderlecht,Anderlecht,1070,,,4568,,Chaussée de Mons,Bergense Steenweg,BE-BRU,current
0,0,51.19,4.43,1,,12,11002,,Anvers,Antwerpen,2600,,Berchem,9,,,Grote Steenweg,BE-VLG,current
0,0,51.19,4.43,2,,14,11002,,Anvers,Antwerpen,2600,,Berchem,9,,,Grote Steenweg,BE-VLG,retired
"
        );
        let (rows, invalid) = rows(csv.as_bytes()).unwrap();
        assert_eq!((rows.len(), invalid), (3, 0));
        assert_eq!(
            rows[0].street_names,
            ["Chaussée de Mons", "Bergense Steenweg"]
        );
        assert_eq!(rows[0].city_names, ["Anderlecht"]);
        assert_eq!(rows[2].municipality_names, ["Antwerpen", "Anvers"]);
        assert_eq!(rows[2].city_names, ["Berchem"]);
        assert_eq!(rows[2].context, "Antwerpen, Vlaanderen");

        let docs = documents(rows, "be");
        let streets: Vec<_> = docs.iter().filter(|d| d["type"] == "street").collect();
        // The box numbers of 475 make one housenumber.
        assert_eq!(streets[0]["housenumbers"].as_object().unwrap().len(), 1);
        assert!(
            docs.iter()
                .any(|d| d["type"] == "locality" && d["name"] == "Berchem")
        );
    }
}
