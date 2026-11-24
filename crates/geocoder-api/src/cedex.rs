//! The CEDEX table built into the `fr` index ([`geocoder_core::postal`]),
//! from the open sources ([`geocoder_core::readers::cedex`]), each streamed
//! from its URL: nothing is stored, FINESS+ (~800 MB of JSON) is parsed one
//! structure at a time.
//!
//! A source that fails is skipped with a warning: the index is built with
//! what the others gave, or without a table. `GEOCODER_CEDEX=false` skips
//! them all.

use std::io::{BufReader, Read};
use std::time::Duration;

use flate2::read::MultiGzDecoder;
use geocoder_core::postal::{CedexTable, CedexTableBuilder};
use geocoder_core::readers::cedex::{read_annuaire, read_finess, read_list};
use reqwest::blocking::Client;
use serde::Deserialize;
use tracing::{info, info_span, warn};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Where the sources are.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The CEDEX list rebuilt from SIRENE in 2017 (CSV), pinned to its last
    /// full version: the file has since been emptied.
    pub list: String,
    /// The data.gouv.fr dataset of the FINESS+ structures: its latest
    /// monthly file is read (their names change every month).
    pub finess_dataset: String,
    /// The Annuaire de l'administration, JSON export.
    pub annuaire: String,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            list: "https://raw.githubusercontent.com/cquest/geocodage-spd/\
                   20ff02d9e44771eb817c0ff08b5ea54b7379e99b/insee-sirene/cedex.csv"
                .into(),
            finess_dataset: "https://www.data.gouv.fr/api/1/datasets/finess-structures-1/".into(),
            annuaire: "https://api-lannuaire.service-public.gouv.fr/api/explore/v2.1/catalog/\
                       datasets/api-lannuaire-administration/exports/json\
                       ?select=adresse,code_insee_commune"
                .into(),
        }
    }
}

/// Whether the CEDEX table is built (`GEOCODER_CEDEX`, on by default).
pub fn enabled() -> bool {
    !matches!(
        std::env::var("GEOCODER_CEDEX").as_deref().map(str::trim),
        Ok("false" | "0" | "off")
    )
}

/// The table from the default sources, off the async runtime.
pub async fn table() -> Option<CedexTable> {
    crate::telemetry::spawn_blocking(|| build(&Sources::default()))
        .await
        .unwrap_or_else(|e| {
            warn!("CEDEX table not built: {e}");
            None
        })
}

/// Reads every source into a table; `None` when none gave anything.
pub fn build(sources: &Sources) -> Option<CedexTable> {
    let span = info_span!(
        "geocoder.cedex",
        sentry.op = "geocoder.cedex",
        codes = tracing::field::Empty,
        localities = tracing::field::Empty,
    );
    let _entered = span.enter();
    let client = match client() {
        Ok(client) => client,
        Err(e) => {
            warn!("CEDEX table not built: {e}");
            return None;
        }
    };
    let mut table = CedexTableBuilder::new();
    let mut read =
        |name: &str, run: &mut dyn FnMut(&mut CedexTableBuilder) -> Result<usize, Error>| match run(
            &mut table,
        ) {
            Ok(kept) => info!("CEDEX: {kept} addresses from {name}"),
            Err(e) => warn!("CEDEX: {name} skipped: {e}"),
        };
    read("the 2017 list", &mut |t| {
        Ok(read_list(get(&client, &sources.list)?, t)?)
    });
    read("FINESS+", &mut |t| {
        let url = latest_finess(&client, &sources.finess_dataset)?;
        info!("CEDEX: reading {url}");
        let body = BufReader::with_capacity(1 << 20, get(&client, &url)?);
        Ok(read_finess(
            BufReader::with_capacity(1 << 20, MultiGzDecoder::new(body)),
            t,
        )?)
    });
    read("the Annuaire de l'administration", &mut |t| {
        Ok(read_annuaire(
            BufReader::with_capacity(1 << 20, get(&client, &sources.annuaire)?),
            t,
        )?)
    });
    if table.codes() == 0 {
        warn!("CEDEX: no source available, the index has no table");
        return None;
    }
    let table = table.build();
    span.record("codes", table.codes());
    span.record("localities", table.localities());
    info!(
        "CEDEX table: {} codes, {} localities",
        table.codes(),
        table.localities()
    );
    Some(table)
}

fn client() -> Result<Client, Error> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(10))
        // The blocking client applies it to each wait on the body, not to
        // the whole transfer.
        .timeout(Duration::from_secs(120))
        .build()?)
}

/// A response body, read as it arrives.
fn get(client: &Client, url: &str) -> Result<impl Read, Error> {
    Ok(client.get(url).send()?.error_for_status()?)
}

#[derive(Deserialize)]
struct Dataset {
    resources: Vec<Resource>,
}

#[derive(Deserialize)]
struct Resource {
    title: String,
    url: String,
}

/// The latest `finess-structures-mensuel-YYYYMM.json.gz` of the dataset:
/// the names sort by month.
fn latest_finess(client: &Client, dataset: &str) -> Result<String, Error> {
    let dataset: Dataset = serde_json::from_reader(get(client, dataset)?)?;
    dataset
        .resources
        .into_iter()
        .filter(|r| r.title.starts_with("finess-structures-mensuel-"))
        .max_by(|a, b| a.title.cmp(&b.title))
        .map(|r| r.url)
        .ok_or_else(|| "no monthly FINESS+ file in the dataset".into())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, Write};
    use std::net::TcpListener;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    /// Serves `files` (path → body) on a local port; anything else is a 404.
    fn serve(files: HashMap<&'static str, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut line = String::new();
                let mut reader = std::io::BufReader::new(&stream);
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                }
                let path = line.split(' ').nth(1).unwrap_or_default();
                let path = path.split('?').next().unwrap_or_default();
                let (status, body) = match files.get(path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        base
    }

    fn gzip(data: &str) -> Vec<u8> {
        let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
        gz.write_all(data.as_bytes()).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn reads_the_three_sources() {
        let finess = r#"{"pmej": [{"adresse": [{"codePostal": "01003", "ligneAcheminement": "BOURG EN BRESSE CEDEX", "cogCommune": "01053"}]}]}"#;
        let mut files = HashMap::new();
        files.insert(
            "/list.csv",
            b"cedex,libelle,insee\n92074,PARIS LA DEFENSE CEDEX,92062\n".to_vec(),
        );
        files.insert("/finess-202610.json.gz", gzip(finess));
        files.insert(
            "/annuaire.json",
            br#"[{"adresse": "[{\"code_postal\": \"06414\", \"nom_commune\": \"Cannes Cedex\"}]", "code_insee_commune": "06029"}]"#.to_vec(),
        );
        let base = serve(files.clone());
        // The dataset lists two months: the latest is read.
        let dataset = format!(
            r#"{{"resources": [
                {{"title": "finess-structures-mensuel-202609.json.gz", "url": "{base}/missing.json.gz"}},
                {{"title": "finess-structures-mensuel-202610.json.gz", "url": "{base}/finess-202610.json.gz"}},
                {{"title": "finess-structures-journalier-20261124.json.gz", "url": "{base}/missing.json.gz"}}]}}"#
        );
        files.insert("/dataset", dataset.into_bytes());
        let base = serve(files);

        let table = build(&Sources {
            list: format!("{base}/list.csv"),
            finess_dataset: format!("{base}/dataset"),
            annuaire: format!("{base}/annuaire.json"),
        })
        .unwrap();
        assert_eq!(table.by_code("92074"), ["92062"]);
        assert_eq!(table.by_code("01003"), ["01053"]);
        assert_eq!(table.by_code("06414"), ["06029"]);
    }

    #[test]
    fn a_failing_source_is_skipped() {
        let mut files = HashMap::new();
        files.insert(
            "/list.csv",
            b"cedex,libelle,insee\n92074,PARIS LA DEFENSE CEDEX,92062\n".to_vec(),
        );
        let base = serve(files);
        let table = build(&Sources {
            list: format!("{base}/list.csv"),
            finess_dataset: format!("{base}/gone"),
            annuaire: format!("{base}/gone"),
        })
        .unwrap();
        assert_eq!(table.codes(), 1);
    }

    #[test]
    fn nothing_available_gives_no_table() {
        let base = serve(HashMap::new());
        assert!(
            build(&Sources {
                list: format!("{base}/gone"),
                finess_dataset: format!("{base}/gone"),
                annuaire: format!("{base}/gone"),
            })
            .is_none()
        );
    }

    /// The real sources (network): `cargo test -p geocoder-api -- --ignored
    /// real_sources --nocapture`.
    #[test]
    #[ignore]
    fn real_sources() {
        let started = std::time::Instant::now();
        let table = build(&Sources::default()).unwrap();
        eprintln!(
            "{} codes, {} localities in {:?}",
            table.codes(),
            table.localities(),
            started.elapsed()
        );
        assert!(table.codes() > 10_000);
        assert_eq!(table.by_code("92074"), ["92062"]);
    }
}
