//! Keep a country's index in sync with its sources.
//!
//! The sources' `Last-Modified` / `ETag` are kept in a sidecar file next to
//! the index (`<index>.source.json`): when none changed, nothing is done
//! unless forced. Otherwise the sources are downloaded next to the index,
//! the index is rebuilt (written atomically) and the downloads removed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use reqwest::header::{CONTENT_LENGTH, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::config::Country;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown profile {0}")]
    UnknownProfile(String),
    #[error("unable to download {url}: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("{url} answered HTTP {status}")]
    Status { status: u16, url: String },
    #[error("download of {url} truncated ({got} of {expected} bytes)")]
    Truncated {
        url: String,
        expected: u64,
        got: u64,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unable to build the index: {0}")]
    Build(#[from] geocoder_core::GeocoderError),
    #[error("index build interrupted: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("unable to write the sidecar: {0}")]
    Json(#[from] serde_json::Error),
}

/// One source, as last seen.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceVersion {
    pub url: String,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
}

/// What `<index>.source.json` records about the build.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceInfo {
    pub sources: Vec<SourceVersion>,
    pub profile: String,
    pub profile_version: u32,
    #[serde(default)]
    pub documents: usize,
    #[serde(default)]
    pub built_at: Option<DateTime<Utc>>,
}

impl SourceInfo {
    /// Same sources, same versions, same profile.
    fn same_as(&self, other: &SourceInfo) -> bool {
        self.profile == other.profile
            && self.profile_version == other.profile_version
            && self.sources.len() == other.sources.len()
            && self.sources.iter().zip(&other.sources).all(|(a, b)| {
                a.url == b.url
                    && (a.last_modified.is_some() || a.etag.is_some())
                    && a.last_modified == b.last_modified
                    && a.etag == b.etag
            })
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// The index already matches the sources.
    UpToDate { country: String },
    /// The index was (re)built.
    Built {
        country: String,
        documents: usize,
        skipped: usize,
        invalid: usize,
        download_seconds: f64,
        build_seconds: f64,
    },
}

pub fn sidecar_path(index: &Path) -> PathBuf {
    with_suffix(index, ".source.json")
}

/// The sidecar of an index, if any.
pub fn read_sidecar(index: &Path) -> Option<SourceInfo> {
    let bytes = std::fs::read(sidecar_path(index)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn is_remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Extension of the last path segment of a URL (`zip`, `gz`, `csv`…).
fn extension(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 8 => ext.to_lowercase(),
        _ => String::new(),
    }
}

struct Remote {
    version: SourceVersion,
    /// After redirects: gives the file extension (data.public.lu `/r/<id>`
    /// links redirect to `…/addresses.csv`).
    final_url: String,
    length: Option<u64>,
}

pub async fn sync(country: &Country, force: bool) -> Result<SyncOutcome, Error> {
    let profile = geocoder_core::profile::by_name(country.profile)
        .ok_or_else(|| Error::UnknownProfile(country.profile.to_string()))?;
    if let Some(parent) = country.index_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(3 * 3600))
        .build()
        .map_err(|source| Error::Request {
            url: String::new(),
            source,
        })?;

    // What the sources are now.
    let mut remotes = Vec::new();
    for url in &country.urls {
        remotes.push(probe(&client, url).await?);
    }
    let mut current = SourceInfo {
        sources: remotes.iter().map(|r| r.version.clone()).collect(),
        profile: profile.name().to_string(),
        profile_version: profile.version(),
        ..Default::default()
    };

    let index_exists = tokio::fs::try_exists(&country.index_path)
        .await
        .unwrap_or(false);
    if !force
        && index_exists
        && let Some(previous) = read_sidecar(&country.index_path)
        && previous.same_as(&current)
    {
        info!("[{}] index up to date", country.code);
        return Ok(SyncOutcome::UpToDate {
            country: country.code.clone(),
        });
    }

    // Download next to the index (same volume, no /tmp size surprise).
    let started = Instant::now();
    let mut inputs = Vec::new();
    let mut downloads = Vec::new();
    for (n, remote) in remotes.iter().enumerate() {
        if is_remote(&remote.version.url) {
            let ext = extension(&remote.final_url);
            let target = with_suffix(&country.index_path, &format!(".download{n}.{ext}"));
            info!("[{}] downloading {}", country.code, remote.version.url);
            let result = download(&client, &remote.version.url, &target, remote.length).await;
            downloads.push(target.clone());
            if let Err(e) = result {
                remove_all(&downloads).await;
                return Err(e);
            }
            inputs.push(target);
        } else {
            inputs.push(PathBuf::from(&remote.version.url));
        }
    }
    let download_time = started.elapsed();

    // Build off the async runtime: CPU bound for minutes.
    info!(
        "[{}] building {}",
        country.code,
        country.index_path.display()
    );
    let (source, index_path, code) = (
        country.source,
        country.index_path.clone(),
        country.code.clone(),
    );
    let built = tokio::task::spawn_blocking(move || {
        geocoder_core::index::import_files(&inputs, source, profile, &index_path, |p| {
            info!(
                "[{code}] {} lines, {} documents, {:?}",
                p.lines, p.documents, p.elapsed
            );
        })
    })
    .await;
    remove_all(&downloads).await;
    let stats = built??;

    current.documents = stats.documents;
    current.built_at = Some(Utc::now());
    tokio::fs::write(
        sidecar_path(&country.index_path),
        serde_json::to_vec_pretty(&current)?,
    )
    .await?;
    info!(
        "[{}] index built: {} documents ({} skipped, {} invalid lines) in {:?}",
        country.code, stats.documents, stats.skipped, stats.invalid, stats.elapsed
    );
    let round = |d: Duration| (d.as_secs_f64() * 10.0).round() / 10.0;
    Ok(SyncOutcome::Built {
        country: country.code.clone(),
        documents: stats.documents,
        skipped: stats.skipped,
        invalid: stats.invalid,
        download_seconds: round(download_time),
        build_seconds: round(stats.elapsed),
    })
}

async fn probe(client: &reqwest::Client, url: &str) -> Result<Remote, Error> {
    if !is_remote(url) {
        let modified = tokio::fs::metadata(url).await?.modified()?;
        return Ok(Remote {
            version: SourceVersion {
                url: url.to_string(),
                last_modified: Some(DateTime::<Utc>::from(modified).to_rfc2822()),
                etag: None,
            },
            final_url: url.to_string(),
            length: None,
        });
    }
    let head = client
        .head(url)
        .send()
        .await
        .map_err(|source| Error::Request {
            url: url.to_string(),
            source,
        })?;
    if !head.status().is_success() {
        return Err(Error::Status {
            status: head.status().as_u16(),
            url: url.to_string(),
        });
    }
    let header = |name| {
        head.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    Ok(Remote {
        version: SourceVersion {
            url: url.to_string(),
            last_modified: header(LAST_MODIFIED),
            etag: header(ETAG),
        },
        final_url: head.url().to_string(),
        length: header(CONTENT_LENGTH).and_then(|l| l.parse().ok()),
    })
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    expected: Option<u64>,
) -> Result<(), Error> {
    let request_error = |source| Error::Request {
        url: url.to_string(),
        source,
    };
    let response = client.get(url).send().await.map_err(request_error)?;
    if !response.status().is_success() {
        return Err(Error::Status {
            status: response.status().as_u16(),
            url: url.to_string(),
        });
    }
    let mut file = tokio::fs::File::create(target).await?;
    let mut stream = response.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(request_error)?;
        written += chunk.len() as u64;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    if let Some(expected) = expected
        && written != expected
    {
        return Err(Error::Truncated {
            url: url.to_string(),
            expected,
            got: written,
        });
    }
    Ok(())
}

async fn remove_all(paths: &[PathBuf]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(last_modified: Option<&str>, etag: Option<&str>) -> SourceInfo {
        SourceInfo {
            sources: vec![SourceVersion {
                url: "u".into(),
                last_modified: last_modified.map(str::to_string),
                etag: etag.map(str::to_string),
            }],
            profile: "fr".into(),
            profile_version: 3,
            ..Default::default()
        }
    }

    #[test]
    fn freshness() {
        assert!(info(Some("a"), None).same_as(&info(Some("a"), None)));
        assert!(!info(Some("a"), None).same_as(&info(Some("b"), None)));
        // Without any validator, always rebuild.
        assert!(!info(None, None).same_as(&info(None, None)));
        // A new profile version needs a rebuild.
        let mut other = info(Some("a"), None);
        other.profile_version = 4;
        assert!(!info(Some("a"), None).same_as(&other));
        // A source added.
        let mut more = info(Some("a"), None);
        more.sources.push(more.sources[0].clone());
        assert!(!info(Some("a"), None).same_as(&more));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("https://x/a/addresses.csv"), "csv");
        assert_eq!(extension("https://x/openaddress-bebru.zip?x=1"), "zip");
        assert_eq!(extension("https://x/f.ndjson.gz"), "gz");
        assert_eq!(extension("https://x/r/5cadc5b8-6a7d"), "");
    }
}
