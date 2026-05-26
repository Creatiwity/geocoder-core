use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;

use flate2::read::GzDecoder;
use geocoder_core::{Document, GeocoderError, HouseNumber, IndexBuilder};
use serde::{Deserialize, Deserializer};

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);

    let mut format: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--format" {
            format = Some(args.next().expect("--format requires a value"));
        } else {
            positional.push(arg);
        }
    }

    if positional.len() < 2 {
        eprintln!("usage: geocoder-build [--format bano|native] <url|path> <output.bin>");
        std::process::exit(1);
    }
    let input = &positional[0];
    let output_path: PathBuf = positional[1].clone().into();

    let is_bano = match format.as_deref() {
        Some("bano") => true,
        Some("native") => false,
        _ => input.contains(".sjson") || input.contains("bano"),
    };

    let is_gz = input.ends_with(".gz");
    let raw: Box<dyn Read> = if input.starts_with("http://") || input.starts_with("https://") {
        eprintln!("streaming from {}...", input);
        let response = ureq::get(input)
            .call()
            .map_err(|e| GeocoderError::Io(std::io::Error::other(e.to_string())))?;
        Box::new(response.into_reader())
    } else {
        Box::new(std::fs::File::open(input)?)
    };

    let reader: Box<dyn Read> = if is_gz { Box::new(GzDecoder::new(raw)) } else { raw };
    let reader = BufReader::new(reader);

    let mut builder = IndexBuilder::new();
    let mut count = 0usize;

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let doc = if is_bano {
            match parse_bano_line(&line) {
                Ok(d) => d,
                Err(e) => {
                    let preview = line.get(..120).unwrap_or(&line);
                    eprintln!("line {}: {} | snippet: {}…", line_no + 1, e, preview);
                    continue;
                }
            }
        } else {
            serde_json::from_str::<Document>(&line).map_err(|e| {
                eprintln!("line {}: {}", line_no + 1, e);
                GeocoderError::Json(e)
            })?
        };

        builder.add_document(doc)?;
        count += 1;
        if count % 100_000 == 0 {
            eprintln!("indexed {} documents...", count);
        }
    }

    eprintln!("building index ({} documents)...", count);
    builder.build(&output_path)?;
    eprintln!("done → {}", output_path.display());
    Ok(())
}

// ── BANO format ──────────────────────────────────────────────────────────────

/// Accepts either a bare string or an array of strings (BANO sends both for
/// postcode/city when a street spans multiple municipalities).
#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    fn into_vec(self) -> Vec<String> {
        match self {
            StringOrVec::One(s) => vec![s],
            StringOrVec::Many(v) => v,
        }
    }
}


#[derive(Deserialize)]
struct BanoDoc {
    name: String,
    lat: f64,
    lon: f64,
    #[serde(default)]
    importance: f32,
    #[serde(rename = "type", default)]
    doc_type: String,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_vec")]
    postcode: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_vec")]
    city: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_vec")]
    citycode: Vec<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_vec")]
    street: Vec<String>,
    #[serde(default)]
    housenumbers: HashMap<String, BanoHn>,
}

fn deserialize_opt_string_or_vec<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    // Field is optional: missing → empty vec, present → one or many
    let opt = Option::<StringOrVec>::deserialize(de)?;
    Ok(opt.map(|s| s.into_vec()).unwrap_or_default())
}

#[derive(Deserialize)]
struct BanoHn {
    lat: f64,
    lon: f64,
}

fn parse_bano_line(line: &str) -> Result<Document, serde_json::Error> {
    let bano: BanoDoc = serde_json::from_str(line)?;

    let housenumbers = bano
        .housenumbers
        .into_iter()
        .map(|(number, hn)| HouseNumber { number, lat: hn.lat, lon: hn.lon })
        .collect();

    let mut filters = HashMap::new();
    if !bano.doc_type.is_empty() {
        filters.insert("type".to_string(), bano.doc_type);
    }
    // When multiple postcodes, keep the first as the filter value.
    // All values end up indexed via housenumber geohashes anyway.
    if let Some(p) = bano.postcode.into_iter().next() {
        filters.insert("postcode".to_string(), p);
    }

    let mut extra: HashMap<String, serde_json::Value> = HashMap::new();
    // city / street: join multiple values with " / " so the index sees both names
    if !bano.city.is_empty() {
        extra.insert("city".to_string(), serde_json::Value::String(bano.city.join(" / ")));
    }
    if !bano.street.is_empty() {
        extra.insert("street".to_string(), serde_json::Value::String(bano.street.join(" / ")));
    }
    if let Some(id) = bano.id {
        extra.insert("id".to_string(), serde_json::Value::String(id));
    }
    if !bano.citycode.is_empty() {
        extra.insert("citycode".to_string(), serde_json::Value::String(bano.citycode.join(",")));
    }

    Ok(Document {
        name: bano.name,
        lat: bano.lat,
        lon: bano.lon,
        importance: bano.importance,
        housenumbers,
        filters,
        extra,
    })
}
