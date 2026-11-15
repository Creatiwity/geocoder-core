//! Luxembourg: BD-Adresses of the Administration du cadastre et de la
//! topographie (CC0), `addresses.csv` on data.public.lu, one row per
//! address:
//!
//! `rue;numero;localite;code_postal;id_caclr_rue;id_caclr_bat;lat_wgs84;
//! lon_wgs84;coord_est_luref;coord_nord_luref;id_geoportail;commune;lau2`
//!
//! The label city is the locality ("Hupperdange"), the municipality
//! ("Clervaux") is the context, like the Luxembourgish postal address.

use std::io::Read;

use serde::Deserialize;

use super::grouped::{AddressRow, Grouper};
use crate::doc::Document;
use crate::error::GeocoderError;

#[derive(Debug, Deserialize)]
struct Row {
    rue: String,
    numero: String,
    localite: String,
    code_postal: String,
    id_caclr_rue: String,
    id_caclr_bat: String,
    lat_wgs84: f64,
    lon_wgs84: f64,
    commune: String,
    lau2: String,
}

/// Read the whole register; returns the documents and the invalid rows.
pub fn read(reader: impl Read) -> Result<(Vec<Document>, usize), GeocoderError> {
    let mut grouper = Grouper::new("lu");
    let invalid = for_each_row(reader, |row| grouper.add(row))?;
    Ok((grouper.finish(), invalid))
}

/// The rows of the register, and the invalid ones.
pub fn rows(reader: impl Read) -> Result<(Vec<AddressRow>, usize), GeocoderError> {
    let mut out = Vec::new();
    let invalid = for_each_row(reader, |row| out.push(row))?;
    Ok((out, invalid))
}

/// Call `f` on each row; returns the number of invalid rows.
pub fn for_each_row(
    reader: impl Read,
    mut f: impl FnMut(AddressRow),
) -> Result<usize, GeocoderError> {
    let mut csv = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_reader(super::skip_bom(reader));
    let mut invalid = 0;
    for record in csv.deserialize::<Row>() {
        let Ok(row) = record else {
            invalid += 1;
            continue;
        };
        f(AddressRow {
            id: row.id_caclr_bat,
            street_id: row.id_caclr_rue,
            street_names: vec![row.rue.trim().to_string()],
            housenumber: row.numero.trim().to_string(),
            postcode: row.code_postal.trim().to_string(),
            city_names: vec![row.localite.trim().to_string()],
            municipality_id: row.lau2.trim().to_string(),
            municipality_names: vec![row.commune.trim().to_string()],
            context: row.commune.trim().to_string(),
            lat: row.lat_wgs84,
            lon: row.lon_wgs84,
        });
    }
    Ok(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_register() {
        let csv = "\u{feff}rue;numero;localite;code_postal;id_caclr_rue;id_caclr_bat;lat_wgs84;lon_wgs84;coord_est_luref;coord_nord_luref;id_geoportail;commune;lau2
Kaesfurterstrooss;20;Hupperdange;9755;7984;210309;50.0948;6.0629;72473.06;128979.71;x;Clervaux;0501
Kaesfurterstrooss;24;Hupperdange;9755;7984;210311;50.0947;6.0637;72529.19;128965.98;x;Clervaux;0501
broken;row
";
        let (docs, invalid) = read(csv.as_bytes()).unwrap();
        assert_eq!(invalid, 1);
        let street = docs.iter().find(|d| d["type"] == "street").unwrap();
        assert_eq!(street["id"], "lu_7984");
        assert_eq!(street["name"], "Kaesfurterstrooss");
        assert_eq!(street["city"], "Hupperdange");
        assert_eq!(street["context"], "Clervaux");
        assert_eq!(street["housenumbers"]["20"]["id"], "lu_210309");
    }
}
