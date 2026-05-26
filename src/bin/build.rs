use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;

use flate2::read::GzDecoder;
use geocoder_core::{Document, GeocoderError, HouseNumber, IndexBuilder};
use serde::Deserialize;

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);

    // Parse optional --format flag
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
        eprintln!("usage: geocoder-build [--format bano|native] <input.(ndjson|sjson)[.gz]> <output.bin>");
        std::process::exit(1);
    }
    let input_path: PathBuf = positional[0].clone().into();
    let output_path: PathBuf = positional[1].clone().into();

    // Detect format: explicit flag wins, otherwise sniff from extension
    let is_bano = match format.as_deref() {
        Some("bano") => true,
        Some("native") => false,
        _ => {
            let name = input_path.file_name().unwrap_or_default().to_string_lossy();
            name.contains(".sjson") || name.contains("bano")
        }
    };

    // Open file, optionally decompressing gzip
    let file = std::fs::File::open(&input_path)?;
    let is_gz = input_path.extension().map(|e| e == "gz").unwrap_or(false);

    let reader: Box<dyn Read> = if is_gz {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };

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
                    eprintln!("line {}: {}", line_no + 1, e);
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

/// Raw BANO / addok sjson document.
#[derive(Deserialize)]
struct BanoDoc {
    name: String,
    lat: f64,
    lon: f64,
    #[serde(default)]
    importance: f32,
    #[serde(rename = "type", default)]
    doc_type: String,
    #[serde(default)]
    postcode: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    citycode: Option<String>,
    #[serde(default)]
    id: Option<String>,
    /// Street name when the document is a housenumber entry
    #[serde(default)]
    street: Option<String>,
    /// Dict housenumbers: number → {lat, lon}
    #[serde(default)]
    housenumbers: HashMap<String, BanoHn>,
}

#[derive(Deserialize)]
struct BanoHn {
    lat: f64,
    lon: f64,
}

fn parse_bano_line(line: &str) -> Result<Document, serde_json::Error> {
    let bano: BanoDoc = serde_json::from_str(line)?;

    let housenumbers: Vec<HouseNumber> = bano
        .housenumbers
        .into_iter()
        .map(|(number, hn)| HouseNumber { number, lat: hn.lat, lon: hn.lon })
        .collect();

    // Fields that drive bitmap filtering
    let mut filters = HashMap::new();
    if !bano.doc_type.is_empty() {
        filters.insert("type".to_string(), bano.doc_type);
    }
    if let Some(p) = &bano.postcode {
        filters.insert("postcode".to_string(), p.clone());
    }

    // city + street go into extra so the index builder picks them up via FIELD_BOOSTS.
    // id + citycode are stored for context/label use only.
    let mut extra: HashMap<String, serde_json::Value> = HashMap::new();
    if let Some(city) = bano.city {
        extra.insert("city".to_string(), serde_json::Value::String(city));
    }
    if let Some(street) = bano.street {
        extra.insert("street".to_string(), serde_json::Value::String(street));
    }
    if let Some(id) = bano.id {
        extra.insert("id".to_string(), serde_json::Value::String(id));
    }
    if let Some(cc) = bano.citycode {
        extra.insert("citycode".to_string(), serde_json::Value::String(cc));
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
