//! Keep a country's index in sync, in one of two ways:
//!
//! - **from its sources** ([`sync`] without prebuilt indexes, and
//!   [`publish`]): the sources' `Last-Modified` / `ETag` are compared with
//!   those the index was built from; when one changed, the sources are
//!   downloaded next to the index and the index rebuilt (written
//!   atomically). Takes gigabytes of memory for France.
//! - **prebuilt** ([`sync`] with `GEOCODER_INDEX_BASE_URL`): the signed
//!   manifest `<base>/<cc>.json` is checked against the trusted keys and
//!   this build's versions, the index it names downloaded and checked
//!   against the manifest's size and SHA-256, opened once, then renamed
//!   over the current one. Nothing from the storage is trusted before its
//!   signature is.
//!
//! Either way, `<index>.source.json` records what the index is.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use futures::StreamExt;
use geocoder_core::signed::{self, Hasher, Manifest, SigningKey};
use reqwest::StatusCode;
use reqwest::header::{CONTENT_LENGTH, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tracing::{Instrument, info, info_span, warn};

pub use geocoder_core::signed::SourceVersion;

use crate::config::{Country, Prebuilt};

/// Largest signed manifest accepted (they are about 1 KB).
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown profile {0}")]
    UnknownProfile(String),
    #[error("unable to download {url}: {source}")]
    Request { url: String, source: reqwest::Error },
    #[error("{url} answered HTTP {status}")]
    Status { status: u16, url: String },
    #[error("download of {url} truncated or too long ({got} of {expected} bytes)")]
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
    #[error("manifest rejected: {0}")]
    Signature(#[from] signed::SignatureError),
    #[error("manifest rejected: {0}")]
    Manifest(String),
    #[error("downloaded index rejected: {0}")]
    Integrity(String),
}

/// What `<index>.source.json` records about the index.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceInfo {
    pub sources: Vec<SourceVersion>,
    pub profile: String,
    pub profile_version: u32,
    #[serde(default)]
    pub documents: usize,
    #[serde(default)]
    pub built_at: Option<DateTime<Utc>>,
    /// Of the index file, when it was downloaded prebuilt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Same sources (each with a validator), same versions, same profile.
fn same_sources(a: &[SourceVersion], b: &[SourceVersion]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.url == b.url
                && (a.last_modified.is_some() || a.etag.is_some())
                && a.last_modified == b.last_modified
                && a.etag == b.etag
        })
}

impl SourceInfo {
    fn same_as(&self, other: &SourceInfo) -> bool {
        self.profile == other.profile
            && self.profile_version == other.profile_version
            && same_sources(&self.sources, &other.sources)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// The index already matches its sources (or the published one).
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
    /// A prebuilt index was downloaded.
    Downloaded {
        country: String,
        documents: usize,
        built_at: String,
        download_seconds: f64,
    },
    /// An index and its signed manifest were written, to be uploaded.
    Published {
        country: String,
        documents: usize,
        file: String,
        manifest: String,
    },
}

impl SyncOutcome {
    /// Its `status`, for the span of the country.
    pub fn status(&self) -> &'static str {
        match self {
            Self::UpToDate { .. } => "up_to_date",
            Self::Built { .. } => "built",
            Self::Downloaded { .. } => "downloaded",
            Self::Published { .. } => "published",
        }
    }
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

fn client() -> Result<reqwest::Client, Error> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(3 * 3600))
        .build()
        .map_err(|source| Error::Request {
            url: String::new(),
            source,
        })
}

fn profile_of(country: &Country) -> Result<Arc<dyn geocoder_core::Profile>, Error> {
    geocoder_core::profile::by_name(country.profile)
        .ok_or_else(|| Error::UnknownProfile(country.profile.to_string()))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn round(d: Duration) -> f64 {
    (d.as_secs_f64() * 10.0).round() / 10.0
}

/// Brings the index of `country` up to date: from the prebuilt indexes when
/// configured, from its sources otherwise.
pub async fn sync(
    country: &Country,
    prebuilt: Option<&Prebuilt>,
    force: bool,
) -> Result<SyncOutcome, Error> {
    if let Some(parent) = country.index_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match prebuilt {
        Some(prebuilt) => fetch(country, prebuilt, force).await,
        None => rebuild(country, force).await,
    }
}

struct Remote {
    version: SourceVersion,
    /// After redirects: gives the file extension (data.public.lu `/r/<id>`
    /// links redirect to `…/addresses.csv`).
    final_url: String,
    length: Option<u64>,
}

/// The sources of `country` as they are now.
async fn probe_all(client: &reqwest::Client, country: &Country) -> Result<Vec<Remote>, Error> {
    let mut remotes = Vec::new();
    for url in &country.urls {
        remotes.push(probe(client, url).await?);
    }
    Ok(remotes)
}

async fn rebuild(country: &Country, force: bool) -> Result<SyncOutcome, Error> {
    let profile = profile_of(country)?;
    let client = client()?;
    let remotes = probe_all(&client, country).await?;
    let current = SourceInfo {
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
    let (outcome, info) = build(&client, country, &remotes, current, &country.index_path).await?;
    tokio::fs::write(
        sidecar_path(&country.index_path),
        serde_json::to_vec_pretty(&info)?,
    )
    .await?;
    Ok(outcome)
}

/// Downloads the sources next to `index_path` and builds the index there.
async fn build(
    client: &reqwest::Client,
    country: &Country,
    remotes: &[Remote],
    mut info: SourceInfo,
    index_path: &Path,
) -> Result<(SyncOutcome, SourceInfo), Error> {
    let profile = profile_of(country)?;
    // Download next to the index (same volume, no /tmp size surprise).
    let started = Instant::now();
    let mut inputs = Vec::new();
    let mut downloads = Vec::new();
    for (n, remote) in remotes.iter().enumerate() {
        if is_remote(&remote.version.url) {
            let ext = extension(&remote.final_url);
            let target = with_suffix(index_path, &format!(".download{n}.{ext}"));
            info!("[{}] downloading {}", country.code, remote.version.url);
            let result = download(client, &remote.version.url, &target, remote.length)
                .instrument(info_span!(
                    "geocoder.download",
                    sentry.op = "http.client",
                    url = %remote.version.url,
                    bytes = remote.length,
                ))
                .await;
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
    // Read as they stream, not part of the change detection: refreshed
    // whenever the address sources change.
    let cedex = if country.cedex && crate::cedex::enabled() {
        crate::cedex::table().await
    } else {
        None
    };
    let download_time = started.elapsed();

    // Build off the async runtime: CPU bound for minutes.
    info!("[{}] building {}", country.code, index_path.display());
    let (source, path, code) = (
        country.source,
        index_path.to_path_buf(),
        country.code.clone(),
    );
    let built = crate::telemetry::spawn_blocking(move || {
        geocoder_core::index::import_files(&inputs, source, profile, &path, cedex, |p| {
            info!(
                "[{code}] {} lines, {} documents, {:?}",
                p.lines, p.documents, p.elapsed
            );
        })
    })
    .await;
    remove_all(&downloads).await;
    let stats = built??;

    info.documents = stats.documents;
    info.built_at = Some(Utc::now());
    info!(
        "[{}] index built: {} documents ({} skipped, {} invalid lines) in {:?}",
        country.code, stats.documents, stats.skipped, stats.invalid, stats.elapsed
    );
    Ok((
        SyncOutcome::Built {
            country: country.code.clone(),
            documents: stats.documents,
            skipped: stats.skipped,
            invalid: stats.invalid,
            download_seconds: round(download_time),
            build_seconds: round(stats.elapsed),
        },
        info,
    ))
}

/// The published manifest of `country`, verified and matching this build;
/// `None` when nothing is published yet.
async fn published(
    client: &reqwest::Client,
    country: &Country,
    prebuilt: &Prebuilt,
) -> Result<Option<Manifest>, Error> {
    let url = format!("{}/{}.json", prebuilt.base_url, country.code);
    let Some(envelope) = get_small(client, &url, MAX_MANIFEST_BYTES)
        .instrument(info_span!("geocoder.manifest", sentry.op = "http.client", url = %url))
        .await?
    else {
        return Ok(None);
    };
    let manifest = signed::verify(&envelope, &prebuilt.keys)?;
    check_manifest(&manifest, country)?;
    Ok(Some(manifest))
}

/// A manifest signed by a trusted key still has to describe an index this
/// build can read, for this country.
fn check_manifest(manifest: &Manifest, country: &Country) -> Result<(), Error> {
    let profile = profile_of(country)?;
    let reject = |what: String| Err(Error::Manifest(what));
    if manifest.country != country.code {
        return reject(format!("for country {}", manifest.country));
    }
    if manifest.format_version != geocoder_core::index::format::VERSION {
        return reject(format!(
            "index format v{}, this build reads v{}",
            manifest.format_version,
            geocoder_core::index::format::VERSION
        ));
    }
    if manifest.profile != profile.name() || manifest.profile_version != profile.version() {
        return reject(format!(
            "profile {} v{}, this build has {} v{}",
            manifest.profile,
            manifest.profile_version,
            profile.name(),
            profile.version()
        ));
    }
    // Only ever appended to the base URL, but no path tricks either way.
    let name_ok = manifest.file.strip_suffix(".bin").is_some_and(|stem| {
        !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    if !name_ok {
        return reject(format!("file name {:?}", manifest.file));
    }
    if DateTime::parse_from_rfc3339(&manifest.built_at).is_err() {
        return reject(format!("build date {:?}", manifest.built_at));
    }
    Ok(())
}

async fn fetch(country: &Country, prebuilt: &Prebuilt, force: bool) -> Result<SyncOutcome, Error> {
    let client = client()?;
    let manifest = published(&client, country, prebuilt)
        .await?
        .ok_or_else(|| Error::Manifest(format!("no index published for {}", country.code)))?;

    let index_exists = tokio::fs::try_exists(&country.index_path)
        .await
        .unwrap_or(false);
    let local = index_exists
        .then(|| read_sidecar(&country.index_path))
        .flatten();
    if !force && let Some(local) = &local {
        if local.sha256.as_deref() == Some(manifest.sha256.as_str()) {
            info!("[{}] index up to date", country.code);
            return Ok(SyncOutcome::UpToDate {
                country: country.code.clone(),
            });
        }
        // An older (genuinely signed) manifest replayed: keep ours.
        let published_at = DateTime::parse_from_rfc3339(&manifest.built_at)
            .map(|d| d.with_timezone(&Utc))
            .ok();
        if let (Some(ours), Some(theirs)) = (local.built_at, published_at)
            && theirs < ours
        {
            return Err(Error::Manifest(format!(
                "published index built {theirs}, older than the local one ({ours}); --force to take it"
            )));
        }
    }

    let url = format!("{}/{}", prebuilt.base_url, manifest.file);
    let target = with_suffix(&country.index_path, ".download");
    info!(
        "[{}] downloading {url} ({} bytes)",
        country.code, manifest.size
    );
    let started = Instant::now();
    let checked = async {
        let sha256 = download_hashed(&client, &url, &target, manifest.size)
            .instrument(info_span!(
                "geocoder.download",
                sentry.op = "http.client",
                url = %url,
                bytes = manifest.size,
            ))
            .await?;
        if sha256 != manifest.sha256 {
            return Err(Error::Integrity(format!(
                "SHA-256 {sha256}, the manifest says {}",
                manifest.sha256
            )));
        }
        // Opens (format, profile, FST checksums) before it replaces anything.
        let path = target.clone();
        let documents = crate::telemetry::spawn_blocking(move || {
            geocoder_core::Geocoder::open(&path).map(|g| g.meta().documents as usize)
        })
        .await??;
        if documents != manifest.documents {
            return Err(Error::Integrity(format!(
                "{documents} documents, the manifest says {}",
                manifest.documents
            )));
        }
        Ok(())
    }
    .await;
    if let Err(e) = checked {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(e);
    }
    let download_time = started.elapsed();

    // Index first: a crash in between leaves the new index with the old
    // sidecar, whose hash no longer matches: fetched again next time.
    tokio::fs::rename(&target, &country.index_path).await?;
    let info = SourceInfo {
        sources: manifest.sources.clone(),
        profile: manifest.profile.clone(),
        profile_version: manifest.profile_version,
        documents: manifest.documents,
        built_at: DateTime::parse_from_rfc3339(&manifest.built_at)
            .ok()
            .map(|d| d.with_timezone(&Utc)),
        sha256: Some(manifest.sha256.clone()),
    };
    tokio::fs::write(
        sidecar_path(&country.index_path),
        serde_json::to_vec_pretty(&info)?,
    )
    .await?;
    info!(
        "[{}] index downloaded: {} documents, built {}",
        country.code, manifest.documents, manifest.built_at
    );
    Ok(SyncOutcome::Downloaded {
        country: country.code.clone(),
        documents: manifest.documents,
        built_at: manifest.built_at,
        download_seconds: round(download_time),
    })
}

/// Builds the index of `country` in `out` and writes, next to it, its
/// signed manifest (`<cc>.json`) and the content-addressed copy it names
/// (`<cc>-<sha256 prefix>.bin`), ready to upload. With `prebuilt` and
/// without `force`, nothing is built when the published index was made
/// from the same sources.
pub async fn publish(
    country: &Country,
    out: &Path,
    key: &SigningKey,
    prebuilt: Option<&Prebuilt>,
    force: bool,
) -> Result<SyncOutcome, Error> {
    let profile = profile_of(country)?;
    tokio::fs::create_dir_all(out).await?;
    let client = client()?;
    let remotes = probe_all(&client, country).await?;
    let current = SourceInfo {
        sources: remotes.iter().map(|r| r.version.clone()).collect(),
        profile: profile.name().to_string(),
        profile_version: profile.version(),
        ..Default::default()
    };
    if !force && let Some(prebuilt) = prebuilt {
        match published(&client, country, prebuilt).await {
            Ok(Some(manifest)) if same_sources(&manifest.sources, &current.sources) => {
                info!("[{}] published index up to date", country.code);
                return Ok(SyncOutcome::UpToDate {
                    country: country.code.clone(),
                });
            }
            Ok(_) => {}
            // A published manifest this build cannot use (older format,
            // other key): replaced.
            Err(e) => warn!("[{}] published manifest ignored: {e}", country.code),
        }
    }

    let index_path = out.join(format!("{}.bin", country.code));
    let (_, info) = build(&client, country, &remotes, current, &index_path).await?;
    let (sha256, size) = {
        let path = index_path.clone();
        tokio::task::spawn_blocking(move || signed::sha256_file(&path)).await??
    };
    let file = format!("{}-{}.bin", country.code, &sha256[..16]);
    tokio::fs::rename(&index_path, out.join(&file)).await?;
    let manifest = Manifest {
        country: country.code.clone(),
        format_version: geocoder_core::index::format::VERSION,
        profile: info.profile,
        profile_version: info.profile_version,
        file: file.clone(),
        size,
        sha256,
        documents: info.documents,
        built_at: info
            .built_at
            .map_or_else(now, |d| d.to_rfc3339_opts(SecondsFormat::Secs, true)),
        sources: info.sources,
    };
    let manifest_name = format!("{}.json", country.code);
    tokio::fs::write(out.join(&manifest_name), signed::sign(&manifest, key)).await?;
    info!("[{}] published {file} and {manifest_name}", country.code);
    Ok(SyncOutcome::Published {
        country: country.code.clone(),
        documents: manifest.documents,
        file,
        manifest: manifest_name,
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

/// A small document (a manifest), `None` on 404, an error past `limit`.
async fn get_small(
    client: &reqwest::Client,
    url: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, Error> {
    let request_error = |source| Error::Request {
        url: url.to_string(),
        source,
    };
    let response = client.get(url).send().await.map_err(request_error)?;
    // S3 answers 403 for a missing key without the ListBucket right.
    if matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
    ) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(Error::Status {
            status: response.status().as_u16(),
            url: url.to_string(),
        });
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.map_err(request_error)?);
        if body.len() as u64 > limit {
            return Err(Error::Truncated {
                url: url.to_string(),
                expected: limit,
                got: body.len() as u64,
            });
        }
    }
    Ok(Some(body))
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

/// Downloads exactly `size` bytes (more is an error, stopped right away)
/// and returns their SHA-256.
async fn download_hashed(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    size: u64,
) -> Result<String, Error> {
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
    let truncated = |got| Error::Truncated {
        url: url.to_string(),
        expected: size,
        got,
    };
    let mut file = tokio::fs::File::create(target).await?;
    let mut hasher = Hasher::default();
    let mut stream = response.bytes_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(request_error)?;
        written += chunk.len() as u64;
        if written > size {
            return Err(truncated(written));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    file.sync_all().await?;
    if written != size {
        return Err(truncated(written));
    }
    Ok(hasher.finish())
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

    fn lu() -> Country {
        crate::config::Config::new("lu", None, Path::new("/d"), |_| None)
            .unwrap()
            .countries
            .remove(0)
    }

    fn manifest() -> Manifest {
        let profile = geocoder_core::profile::by_name("lu").unwrap();
        Manifest {
            country: "lu".into(),
            format_version: geocoder_core::index::format::VERSION,
            profile: "lu".into(),
            profile_version: profile.version(),
            file: "lu-0123456789abcdef.bin".into(),
            size: 1,
            sha256: "00".repeat(32),
            documents: 1,
            built_at: "2026-11-15T22:00:00Z".into(),
            sources: Vec::new(),
        }
    }

    #[test]
    fn manifests_must_fit_this_build() {
        assert!(check_manifest(&manifest(), &lu()).is_ok());
        let bad = [
            Manifest {
                country: "be".into(),
                ..manifest()
            },
            Manifest {
                format_version: 1,
                ..manifest()
            },
            Manifest {
                profile_version: 999,
                ..manifest()
            },
            Manifest {
                file: "../lu.bin".into(),
                ..manifest()
            },
            Manifest {
                file: "lu.bin?x".into(),
                ..manifest()
            },
            Manifest {
                file: ".bin".into(),
                ..manifest()
            },
            Manifest {
                built_at: "yesterday".into(),
                ..manifest()
            },
        ];
        for m in bad {
            assert!(check_manifest(&m, &lu()).is_err(), "{m:?}");
        }
    }
}
