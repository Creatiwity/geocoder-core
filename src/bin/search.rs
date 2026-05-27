use std::collections::HashMap;
use std::path::PathBuf;

use geocoder_core::{Geocoder, GeocoderError, SearchOpts};

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);

    let mut lat: Option<f64> = None;
    let mut lon: Option<f64> = None;
    let mut limit: Option<usize> = None;
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    let mut positional: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lat" => { lat = args.next().and_then(|v| v.parse().ok()); }
            "--lon" => { lon = args.next().and_then(|v| v.parse().ok()); }
            "--limit" => { limit = args.next().and_then(|v| v.parse().ok()); }
            "--filter" => {
                if let Some(kv) = args.next() {
                    if let Some((k, v)) = kv.split_once('=') {
                        filters.entry(k.to_string()).or_default().push(v.to_string());
                    }
                }
            }
            _ => positional.push(arg),
        }
    }

    if positional.len() < 2 {
        eprintln!("usage: geocoder-search [--lat F] [--lon F] [--limit N] [--filter key=val] <index.bin> <query...>");
        std::process::exit(1);
    }

    let index_path: PathBuf = positional[0].clone().into();
    let query = positional[1..].join(" ");

    let geocoder = Geocoder::open(&index_path)?;
    let opts = SearchOpts { lat, lon, limit, filters };
    let results = geocoder.search(&query, opts);

    if results.is_empty() {
        println!("no results");
        return Ok(());
    }

    for (i, r) in results.iter().enumerate() {
        let label = r.label();
        let score = r.score;
        let lat = r.doc["lat"].as_f64().unwrap_or(0.0);
        let lon = r.doc["lon"].as_f64().unwrap_or(0.0);
        let hn = r.matched_housenumber.as_deref().unwrap_or("");
        println!(
            "{:2}. [{:.3}] {} ({:.5}, {:.5}){}",
            i + 1,
            score,
            label,
            lat,
            lon,
            if hn.is_empty() { String::new() } else { format!("  hn={}", hn) }
        );
    }

    Ok(())
}
