//! Round trip evaluation for the address registers (no reference API to
//! compare with, unlike the BAN): sample addresses of the register, write
//! them the ways people do, geocode them and check the first result lands
//! on the address.
//!
//! cargo run --release --example roundtrip -- --source lu|be [--sample N] \
//!     [--failures out.tsv] <index.bin> <register files...>

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use geocoder_core::readers::grouped::AddressRow;
use geocoder_core::readers::{Source, be, for_each_input, lu};
use geocoder_core::search::results::haversine_km;
use geocoder_core::{Geocoder, SearchOpts};

#[derive(Default)]
struct Stats {
    total: usize,
    /// First result is the address (≤ 30 m, housenumber matched).
    exact: usize,
    /// First result within 250 m (right street).
    near: usize,
    empty: usize,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut source = Source::Lu;
    let mut sample = 2000usize;
    // Keep one row out of `every`: a deterministic spread over the register.
    let mut every = 50usize;
    let mut failures: Option<PathBuf> = None;
    let mut positional = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => source = Source::from_name(&args.next().unwrap()).expect("source"),
            "--sample" => sample = args.next().unwrap().parse().expect("sample"),
            "--every" => every = args.next().unwrap().parse().expect("every"),
            "--failures" => failures = args.next().map(PathBuf::from),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let (index, inputs) = positional.split_first().expect("index and inputs");
    let geocoder = Geocoder::open(index).expect("index");

    let mut rows: Vec<AddressRow> = Vec::new();
    let mut seen = 0usize;
    for input in inputs {
        read(input, source, |row| {
            if seen.is_multiple_of(every) {
                rows.push(row);
            }
            seen += 1;
        });
    }
    // Spread the sample over every input (Belgium: one file per region).
    let stride = (rows.len() / sample.max(1)).max(1);
    let picked: Vec<&AddressRow> = rows.iter().step_by(stride).take(sample).collect();

    let prefix = if source == Source::Lu { "L-" } else { "" };
    let mut stats: BTreeMap<&str, Stats> = BTreeMap::new();
    let mut out: Option<Box<dyn Write>> = failures
        .as_deref()
        .map(|p| Box::new(std::fs::File::create(p).unwrap()) as Box<dyn Write>);
    for (n, row) in picked.iter().enumerate() {
        let street = &row.street_names[0];
        let city = &row.city_names[0];
        let (hn, cp) = (&row.housenumber, &row.postcode);
        let mut variants: Vec<(&str, String)> = vec![
            ("number first", format!("{hn} {street} {cp} {city}")),
            (
                "number after",
                format!("{street} {hn}, {prefix}{cp} {city}"),
            ),
            ("no postcode", format!("{hn} {street} {city}")),
            ("lowercase ascii", fold(&format!("{street} {hn} {city}"))),
            ("typo", format!("{hn} {} {cp} {city}", typo(street, n))),
            ("street only", format!("{street} {cp} {city}")),
        ];
        if let (Some(street2), Some(city2)) = (row.street_names.get(1), row.city_names.get(1)) {
            variants.push(("second language", format!("{street2} {hn}, {cp} {city2}")));
        } else if let Some(street2) = row.street_names.get(1) {
            variants.push(("second language", format!("{street2} {hn}, {cp} {city}")));
        }
        for (kind, query) in variants {
            let results = geocoder.search(
                &query,
                SearchOpts {
                    limit: Some(1),
                    autocomplete: Some(false),
                    ..Default::default()
                },
            );
            let s = stats.entry(kind).or_default();
            s.total += 1;
            let Some(first) = results.first() else {
                s.empty += 1;
                log(&mut out, kind, &query, "-", f64::NAN);
                continue;
            };
            let km = haversine_km(first.lat, first.lon, row.lat, row.lon);
            let street_only = kind == "street only";
            let exact = km <= 0.03
                && first
                    .housenumber
                    .as_deref()
                    .is_some_and(|h| h.eq_ignore_ascii_case(hn));
            if exact {
                s.exact += 1;
            }
            if km <= 0.25 {
                s.near += 1;
            }
            let failed = if street_only { km > 0.25 } else { !exact };
            if failed {
                log(&mut out, kind, &query, &first.label, km);
            }
        }
    }

    println!("{} sampled addresses out of {}", picked.len(), seen);
    println!("| variant | queries | exact address | ≤ 250 m | no result |");
    println!("|---|---:|---:|---:|---:|");
    for (kind, s) in &stats {
        let pct = |v: usize| 100.0 * v as f64 / s.total.max(1) as f64;
        println!(
            "| {kind} | {} | {:.1} % | {:.1} % | {:.1} % |",
            s.total,
            pct(s.exact),
            pct(s.near),
            pct(s.empty)
        );
    }
}

fn read(path: &Path, source: Source, mut f: impl FnMut(AddressRow)) {
    for_each_input(path, |reader| {
        match source {
            Source::Lu => lu::for_each_row(reader, &mut f)?,
            Source::Be => be::for_each_row(reader, &mut f)?,
            _ => panic!("registers only"),
        };
        Ok(())
    })
    .expect("read register");
}

fn fold(s: &str) -> String {
    geocoder_core::text::ascii(s)
}

/// Drop one letter of the longest word, deterministically.
fn typo(street: &str, n: usize) -> String {
    let words: Vec<&str> = street.split(' ').collect();
    let Some((i, word)) = words
        .iter()
        .enumerate()
        .max_by_key(|(_, w)| w.chars().count())
    else {
        return street.to_string();
    };
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 5 {
        return street.to_string();
    }
    let at = 1 + n % (chars.len() - 2);
    let mangled: String = chars
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != at)
        .map(|(_, c)| c)
        .collect();
    let mut words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    words[i] = mangled;
    words.join(" ")
}

fn log(out: &mut Option<Box<dyn Write>>, kind: &str, query: &str, got: &str, km: f64) {
    if let Some(out) = out {
        let _ = writeln!(out, "{kind}\t{query}\t{got}\t{km:.3}");
    }
}
