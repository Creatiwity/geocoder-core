use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use geocoder_core::{Geocoder, GeocoderError, SearchOpts, SearchResult};
use serde_json::json;

const USAGE: &str = "usage: geocoder-search [options] <index.bin> <query...>
       geocoder-search [options] --batch <index.bin>   (one query per stdin line, JSON lines out)

  --lat F --lon F      center
  --limit N            number of results (default 5)
  --filter key=value   filter, repeatable
  --no-autocomplete    like addok's autocomplete=0
  --json               JSON output";

fn main() -> Result<(), GeocoderError> {
    let mut args = std::env::args().skip(1);
    let mut opts = SearchOpts::default();
    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
    let (mut batch, mut json) = (false, false);
    let mut positional: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lat" => opts.lat = args.next().and_then(|v| v.parse().ok()),
            "--lon" => opts.lon = args.next().and_then(|v| v.parse().ok()),
            "--limit" => opts.limit = args.next().and_then(|v| v.parse().ok()),
            "--filter" => {
                if let Some((k, v)) = args.next().as_deref().and_then(|kv| kv.split_once('=')) {
                    filters
                        .entry(k.to_string())
                        .or_default()
                        .push(v.to_string());
                }
            }
            "--no-autocomplete" => opts.autocomplete = Some(false),
            "--batch" => batch = true,
            "--json" => json = true,
            "-h" | "--help" => usage(),
            _ => positional.push(arg),
        }
    }
    opts.filters = filters;
    let Some(index_path) = positional.first() else {
        usage()
    };
    let geocoder = Geocoder::open(&PathBuf::from(index_path))?;

    if batch {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for line in std::io::stdin().lock().lines() {
            let query = line?;
            let results = geocoder.search(&query, opts.clone());
            let features: Vec<_> = results.iter().map(to_json).collect();
            writeln!(out, "{}", json!({"query": query, "features": features}))?;
        }
        return Ok(());
    }

    if positional.len() < 2 {
        usage();
    }
    let query = positional[1..].join(" ");
    let results = geocoder.search(&query, opts);
    if json {
        println!("{}", json!(results.iter().map(to_json).collect::<Vec<_>>()));
    } else if results.is_empty() {
        println!("no results");
    } else {
        for (i, r) in results.iter().enumerate() {
            let hn = r
                .housenumber
                .as_deref()
                .map(|h| format!("  hn={h}"))
                .unwrap_or_default();
            println!(
                "{:2}. [{:.3}] {} ({:.5}, {:.5}){hn}",
                i + 1,
                r.score,
                r.label,
                r.lat,
                r.lon
            );
        }
    }
    Ok(())
}

fn to_json(r: &SearchResult) -> serde_json::Value {
    json!({
        "label": r.label,
        "score": r.score,
        "housenumber": r.housenumber,
        "type": r.doc.get("type"),
        "id": r.doc.get("id"),
        "lat": r.lat,
        "lon": r.lon,
    })
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(1)
}
