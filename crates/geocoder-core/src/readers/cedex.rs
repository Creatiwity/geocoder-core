//! The open sources of CEDEX codes, read into a
//! [`crate::postal::CedexTableBuilder`]. All under the
//! Licence Ouverte. La Poste's own list (Hexaposte) is not open; SIRENE no
//! longer publishes the CEDEX of establishments.
//!
//! - [`read_list`]: the CEDEX list rebuilt from SIRENE in 2017
//!   (data.gouv.fr "Liste des CEDEX"), frozen but the widest: ~10 800 codes.
//! - [`read_finess`]: FINESS+ structures (health and social establishments,
//!   Agence du Numérique en Santé), monthly: their CEDEX postal addresses.
//! - [`read_annuaire`]: the Annuaire de l'administration (DILA), daily:
//!   idem for public offices.
//!
//! The JSON sources are streamed, one record at a time: FINESS+ is a single
//! ~800 MB document.

use std::fmt;
use std::io::Read;

use serde::Deserialize;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::error::GeocoderError;
use crate::postal::{CedexTableBuilder, is_cedex};

/// `cedex,libelle,insee` CSV. Returns the rows kept.
pub fn read_list(reader: impl Read, table: &mut CedexTableBuilder) -> Result<usize, GeocoderError> {
    #[derive(Deserialize)]
    struct Row {
        cedex: String,
        libelle: String,
        insee: String,
    }
    let mut kept = 0;
    for row in csv::Reader::from_reader(reader).deserialize::<Row>() {
        let row = row.map_err(|e| GeocoderError::Io(std::io::Error::other(e)))?;
        kept += usize::from(table.add(&row.cedex, &row.libelle, &row.insee));
    }
    Ok(kept)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinessAddress {
    code_postal: Option<String>,
    ligne_acheminement: Option<String>,
    cog_commune: Option<String>,
}

#[derive(Deserialize)]
struct Ege {
    #[serde(default)]
    adresse: Vec<FinessAddress>,
}

#[derive(Deserialize)]
struct Pmej {
    #[serde(default)]
    adresse: Vec<FinessAddress>,
    #[serde(default)]
    ege: Vec<Ege>,
}

/// FINESS+ structures JSON (`finess-structures-*.json`, decompressed): the
/// addresses of its legal entities (`pmej`) and their establishments
/// (`ege`) whose delivery line is a CEDEX. Returns the addresses kept.
pub fn read_finess(
    reader: impl Read,
    table: &mut CedexTableBuilder,
) -> Result<usize, GeocoderError> {
    let mut kept = 0;
    let mut add = |a: &FinessAddress| {
        if let (Some(code), Some(line), Some(insee)) =
            (&a.code_postal, &a.ligne_acheminement, &a.cog_commune)
            && is_cedex(line)
        {
            kept += usize::from(table.add(code, line, insee));
        }
    };
    let mut each = |pmej: Pmej| {
        pmej.adresse.iter().for_each(&mut add);
        pmej.ege.iter().flat_map(|e| &e.adresse).for_each(&mut add);
    };
    let mut json = serde_json::Deserializer::from_reader(reader);
    Field {
        name: "pmej",
        each: &mut each,
    }
    .deserialize(&mut json)?;
    Ok(kept)
}

/// The array under one key of the top object, each element handed to
/// `each` as it is parsed; every other key skipped.
struct Field<'a, T> {
    name: &'static str,
    each: &'a mut dyn FnMut(T),
}

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Field<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
}

impl<'de, T: Deserialize<'de>> Visitor<'de> for Field<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "an object with a `{}` array", self.name)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            if key == self.name {
                map.next_value_seed(Each {
                    each: &mut *self.each,
                })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

/// An array, each element handed to `each` as it is parsed.
struct Each<'a, T> {
    each: &'a mut dyn FnMut(T),
}

impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for Each<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_seq(self)
    }
}

impl<'de, T: Deserialize<'de>> Visitor<'de> for Each<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while let Some(item) = seq.next_element::<T>()? {
            (self.each)(item);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct AnnuaireAddress {
    code_postal: Option<String>,
    nom_commune: Option<String>,
}

#[derive(Deserialize)]
struct AnnuaireRecord {
    /// A JSON array, or that array as a string (the export).
    adresse: Option<serde_json::Value>,
    code_insee_commune: Option<String>,
}

/// Annuaire de l'administration JSON export (an array of records with
/// `adresse` and `code_insee_commune`): the addresses whose city is a CEDEX
/// one. Returns the addresses kept.
pub fn read_annuaire(
    reader: impl Read,
    table: &mut CedexTableBuilder,
) -> Result<usize, GeocoderError> {
    let mut kept = 0;
    let mut each = |record: AnnuaireRecord| {
        let Some(insee) = record.code_insee_commune else {
            return;
        };
        let addresses: Vec<AnnuaireAddress> = match record.adresse {
            Some(serde_json::Value::String(s)) => serde_json::from_str(&s).unwrap_or_default(),
            Some(v @ serde_json::Value::Array(_)) => serde_json::from_value(v).unwrap_or_default(),
            _ => Vec::new(),
        };
        for a in addresses {
            if let (Some(code), Some(city)) = (a.code_postal, a.nom_commune)
                && is_cedex(&city)
            {
                kept += usize::from(table.add(&code, &city, &insee));
            }
        }
    };
    let mut json = serde_json::Deserializer::from_reader(reader);
    Each { each: &mut each }.deserialize(&mut json)?;
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_2017_list() {
        let mut b = CedexTableBuilder::new();
        let csv = "cedex,libelle,insee\n\"92074\",\"PARIS LA DEFENSE CEDEX\",\"92062\"\n\"92081\",\"PARIS LA DEFENSE CEDEX\",\"75112\"\n";
        assert_eq!(read_list(csv.as_bytes(), &mut b).unwrap(), 1);
        assert_eq!(b.build().by_code("92074"), ["92062"]);
    }

    #[test]
    fn finess_streamed() {
        let json = r#"{"schemaVersion": "v1.0.0", "gco": [{"x": [1, {"y": 2}]}],
            "pmej": [
              {"adresse": [{"codePostal": "01003", "ligneAcheminement": "BOURG EN BRESSE CEDEX", "cogCommune": "01053", "other": {"a": 1}}],
               "ege": [{"adresse": [{"codePostal": "01108", "ligneAcheminement": "OYONNAX CEDEX", "cogCommune": "01283"}]},
                       {"adresse": [{"codePostal": "01000", "ligneAcheminement": "BOURG EN BRESSE", "cogCommune": "01053"}]}]},
              {"adresse": []}
            ],
            "gcc": []}"#;
        let mut b = CedexTableBuilder::new();
        assert_eq!(read_finess(json.as_bytes(), &mut b).unwrap(), 2);
        let t = b.build();
        assert_eq!(t.by_code("01108"), ["01283"]);
        // Not a CEDEX: not kept.
        assert!(t.by_code("01000").is_empty());
    }

    #[test]
    fn annuaire_with_string_addresses() {
        let json = r#"[
            {"adresse": "[{\"type_adresse\": \"Adresse postale\", \"code_postal\": \"06414\", \"nom_commune\": \"Cannes Cedex\"}]", "code_insee_commune": "06029"},
            {"adresse": [{"code_postal": "33000", "nom_commune": "Bordeaux"}], "code_insee_commune": "33063"},
            {"adresse": null, "code_insee_commune": null}
        ]"#;
        let mut b = CedexTableBuilder::new();
        assert_eq!(read_annuaire(json.as_bytes(), &mut b).unwrap(), 1);
        let t = b.build();
        assert_eq!(t.by_code("06414"), ["06029"]);
        assert_eq!(t.by_locality("CANNES"), ["06029"]);
    }

    #[test]
    fn malformed_json_is_an_error() {
        let mut b = CedexTableBuilder::new();
        assert!(read_finess(r#"{"pmej": [{"adresse": [}"#.as_bytes(), &mut b).is_err());
    }
}
