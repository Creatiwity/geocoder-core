//! Rust side of the differential tests against addok (see
//! `scripts/diff_pipeline.py`): same JSON lines protocol as the Python
//! oracle, on the same functions.
//!
//! Ops: `tokens` (index-time processing), `query` (query-time processing),
//! `hn` (housenumber key), `index` (token boosts), `score` (result
//! processors on one document).

use std::io::{BufRead, Write};

use geocoder_core::IndexBuilder;
use geocoder_core::doc::Document;
use geocoder_core::profile::{self, preprocess, preprocess_query};
use geocoder_core::search::results::{Context, process};
use geocoder_core::text::ascii;
use serde_json::{Value, json};

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "fr".into());
    let profile = profile::by_name(&name).expect("unknown profile");
    let mut builder = IndexBuilder::new(profile.clone());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let req: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let s = |k: &str| req[k].as_str().unwrap_or_default().to_string();
        let res = match req["op"].as_str().unwrap() {
            "tokens" => json!(
                preprocess(profile.as_ref(), &s("s"))
                    .iter()
                    .map(|t| t.value.clone())
                    .collect::<Vec<_>>()
            ),
            "query" => {
                let tokens = preprocess_query(profile.as_ref(), &ascii(s("q").trim()));
                json!(tokens
                    .iter()
                    .map(|t| json!({"t": t.value, "kind": t.is_housenumber().then_some("housenumber"), "pos": t.positions}))
                    .collect::<Vec<_>>())
            }
            "hn" => json!(
                preprocess(profile.as_ref(), &s("s"))
                    .iter()
                    .map(|t| t.value.as_str())
                    .collect::<String>()
            ),
            "index" => {
                let doc: Document = serde_json::from_value(req["doc"].clone()).unwrap();
                let tokens = builder.field_tokens(&doc);
                json!(
                    tokens
                        .into_iter()
                        .map(|(t, b)| (t, json!(b)))
                        .collect::<serde_json::Map<String, Value>>()
                )
            }
            "score" => {
                let mut doc: Document = serde_json::from_value(req["doc"].clone()).unwrap();
                if let Some(Value::Object(hns)) = doc.remove("housenumbers") {
                    let mut prepared = serde_json::Map::new();
                    for (number, mut data) in hns {
                        let key: String = preprocess(profile.as_ref(), &number)
                            .iter()
                            .map(|t| t.value.as_str())
                            .collect();
                        data["raw"] = Value::String(number);
                        prepared.insert(key, data);
                    }
                    doc.insert("housenumbers".into(), Value::Object(prepared));
                }
                let query = ascii(s("q").trim());
                let tokens = preprocess_query(profile.as_ref(), &query);
                let mut hns: Vec<_> = tokens.iter().filter(|t| t.is_housenumber()).collect();
                hns.sort_by(|a, b| a.positions.cmp(&b.positions));
                let housenumber: String = hns.iter().map(|t| t.value.as_str()).collect();
                let filters = req
                    .get("filters")
                    .and_then(|f| f.as_object())
                    .cloned()
                    .unwrap_or_default();
                let ctx = Context {
                    profile: profile.as_ref(),
                    query: &query,
                    autocomplete: req
                        .get("autocomplete")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    lat: req.get("lat").and_then(|v| v.as_f64()),
                    lon: req.get("lon").and_then(|v| v.as_f64()),
                    check_housenumber: filters.get("type").is_none_or(|t| t == "housenumber"),
                    only_housenumber: filters.get("type").is_some_and(|t| t == "housenumber"),
                    housenumber: &housenumber,
                };
                match process(&ctx, 0, doc) {
                    Some(r) => {
                        json!({"labels": r.labels, "score": r.score(), "housenumber": r.housenumber, "label": r.label()})
                    }
                    None => Value::Null,
                }
            }
            op => panic!("unknown op {op}"),
        };
        writeln!(out, "{res}").unwrap();
    }
}
