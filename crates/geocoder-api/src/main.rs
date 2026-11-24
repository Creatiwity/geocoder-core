//! geocoder-api: one HTTP server for the address indexes of several
//! countries, answering like the French BAN API.
//!
//! - `geocoder-api serve`: serves the indexes found in `GEOCODER_DATA_DIR`
//!   (`<country>.bin`), reloading each when its file changes.
//! - `geocoder-api update [--country xx] [--force]`: brings the indexes up
//!   to date: downloads the prebuilt ones (`GEOCODER_INDEX_BASE_URL`, signed
//!   manifests), or downloads the sources and rebuilds what changed.
//! - `geocoder-api publish --out <dir>`: builds the indexes whose sources
//!   changed since the published ones and signs their manifests
//!   (`GEOCODER_SIGNING_KEY`), for the CI to upload.
//! - `geocoder-api keygen`: a new signing key (private on stdout, public on
//!   stderr).

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod api;
mod cedex;
mod config;
mod handle;
mod sync;
mod telemetry;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use sentry::SentryFutureExt;
use tracing::{Instrument, error, info, info_span};

use config::Config;
use geocoder_core::signed::SigningKey;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Comma separated: fr, lu, be.
    #[arg(long, env = "GEOCODER_COUNTRIES", default_value = "fr", global = true)]
    countries: String,
    /// Country searched without a `country` parameter (default: the first).
    #[arg(long, env = "GEOCODER_DEFAULT_COUNTRY", global = true)]
    default_country: Option<String>,
    /// Directory of the indexes (`<country>.bin`) and their downloads.
    #[arg(long, env = "GEOCODER_DATA_DIR", default_value = "data", global = true)]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the indexes over HTTP.
    Serve {
        #[arg(long, env = "HOST", default_value = "0.0.0.0")]
        host: String,
        #[arg(long, env = "PORT", default_value_t = 8080)]
        port: u16,
        /// How often index files are checked for a new build.
        #[arg(long, env = "GEOCODER_RELOAD_INTERVAL_SECONDS", default_value_t = 60)]
        reload_interval_seconds: u64,
        /// After SIGTERM, readiness fails for this long before shutting down.
        #[arg(long, env = "SHUTDOWN_DELAY_SECONDS", default_value_t = 5)]
        shutdown_delay_seconds: u64,
    },
    /// Bring the indexes up to date: the prebuilt ones when
    /// GEOCODER_INDEX_BASE_URL is set, rebuilt from their sources otherwise.
    Update {
        /// Only this country (repeatable); default: every configured one.
        #[arg(long = "country")]
        only: Vec<String>,
        /// Download or rebuild even when nothing changed.
        #[arg(long)]
        force: bool,
        /// Schedule of the CronJob running this: reported to Sentry Crons
        /// (monitor `geocoder-update`), which alerts on a missed or failed run.
        #[arg(long)]
        crontab: Option<String>,
    },
    /// Build the indexes whose sources changed since the published ones
    /// (GEOCODER_INDEX_BASE_URL), with their signed manifests, into `out`.
    Publish {
        #[arg(long)]
        out: PathBuf,
        /// Only this country (repeatable); default: every configured one.
        #[arg(long = "country")]
        only: Vec<String>,
        /// Build even when the published index is up to date.
        #[arg(long)]
        force: bool,
    },
    /// Generate a signing key: the private key (GEOCODER_SIGNING_KEY) on
    /// stdout, the public one (GEOCODER_INDEX_PUBLIC_KEYS) on stderr.
    Keygen,
    /// Print the public key of GEOCODER_SIGNING_KEY.
    PublicKey,
}

fn main() {
    let _guard = sentry::init((
        std::env::var("SENTRY_DSN").ok(),
        sentry::ClientOptions::new()
            .maybe_release(sentry::release_name!())
            .environment(std::env::var("GEOCODER_ENV").unwrap_or("development".into()))
            .traces_sample_rate(
                std::env::var("SENTRY_TRACES_SAMPLE_RATE")
                    .ok()
                    .and_then(|r| r.parse().ok())
                    .unwrap_or(0.1),
            ),
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Cannot build Tokio runtime");
    runtime.block_on(async {
        telemetry::init();
        let code = run(Cli::parse()).bind_hub(sentry::Hub::current()).await;
        telemetry::exit(code)
    })
}

async fn run(cli: Cli) -> i32 {
    match cli.command {
        Command::Keygen => return keygen(),
        Command::PublicKey => {
            return match signing_key() {
                Some(key) => {
                    println!("{}", key.public_key());
                    0
                }
                None => 2,
            };
        }
        _ => {}
    }
    let prebuilt = match config::Prebuilt::from_env() {
        Ok(prebuilt) => prebuilt,
        Err(e) => {
            error!("Invalid configuration: {e}");
            return 2;
        }
    };
    let config = match Config::from_env(
        &cli.countries,
        cli.default_country.as_deref(),
        &cli.data_dir,
    ) {
        Ok(config) => config,
        Err(e) => {
            error!("Invalid configuration: {e}");
            return 2;
        }
    };
    info!(
        "Countries: {} (default {}), prebuilt indexes {}",
        config
            .countries
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        config.default_country,
        prebuilt
            .as_ref()
            .map_or("off (built from the sources)".to_string(), |p| p
                .base_url
                .clone()),
    );

    match cli.command {
        Command::Serve {
            host,
            port,
            reload_interval_seconds,
            shutdown_delay_seconds,
        } => {
            let addr: SocketAddr = format!("{host}:{port}")
                .parse()
                .unwrap_or_else(|e| panic!("Invalid address {host}:{port}: {e}"));
            let indexes = handle::Indexes::open(&config);
            indexes.watch(Duration::from_secs(reload_interval_seconds.max(1)));
            api::serve(
                addr,
                api::Context {
                    config,
                    indexes,
                    shutting_down: Arc::default(),
                },
                Duration::from_secs(shutdown_delay_seconds),
            )
            .await;
            0
        }
        Command::Update {
            only,
            force,
            crontab,
        } => {
            let monitor = crontab
                .as_deref()
                .map(|tab| telemetry::CronMonitor::start("geocoder-update", tab));
            let code = for_each(&config, &only, |country| {
                sync::sync(country, prebuilt.as_ref(), force)
            })
            .instrument(job_span("update", &config))
            .await;
            if let Some(monitor) = monitor {
                monitor.finish(code == 0);
            }
            code
        }
        Command::Publish { out, only, force } => {
            let Some(key) = signing_key() else {
                return 2;
            };
            info!("Signing with {}", key.public_key());
            for_each(&config, &only, |country| {
                sync::publish(country, &out, &key, prebuilt.as_ref(), force)
            })
            .instrument(job_span("publish", &config))
            .await
        }
        Command::Keygen | Command::PublicKey => unreachable!(),
    }
}

/// The root span of a job: a Sentry transaction, its countries as children.
fn job_span(job: &'static str, config: &Config) -> tracing::Span {
    let countries: Vec<&str> = config.countries.iter().map(|c| c.code.as_str()).collect();
    info_span!(
        "geocoder.job",
        sentry.name = format!("geocoder-api {job}"),
        sentry.op = "job",
        job,
        countries = countries.join(","),
    )
}

fn signing_key() -> Option<SigningKey> {
    let key = std::env::var("GEOCODER_SIGNING_KEY")
        .ok()
        .and_then(|k| k.parse::<SigningKey>().ok());
    if key.is_none() {
        error!("GEOCODER_SIGNING_KEY is missing or invalid (64 hex digits)");
    }
    key
}

/// Prints a new key pair; the private key never goes to a log.
fn keygen() -> i32 {
    let mut seed = [0u8; 32];
    if let Err(e) = getrandom::fill(&mut seed) {
        eprintln!("No randomness available: {e}");
        return 1;
    }
    let key = SigningKey::from_seed(seed);
    println!("{}", key.to_hex());
    eprintln!("public key: {}", key.public_key());
    0
}

/// Each country on its own: one failing source does not block the others.
async fn for_each<'a, F, Fut>(config: &'a Config, only: &[String], run: F) -> i32
where
    F: Fn(&'a config::Country) -> Fut,
    Fut: std::future::Future<Output = Result<sync::SyncOutcome, sync::Error>>,
{
    for code in only {
        if config.country(&code.to_lowercase()).is_none() {
            error!("Country {code} is not configured");
            return 2;
        }
    }
    let mut failed = 0;
    for country in &config.countries {
        if !only.is_empty() && !only.iter().any(|c| c.eq_ignore_ascii_case(&country.code)) {
            continue;
        }
        let span = info_span!(
            "geocoder.sync",
            sentry.op = "geocoder.sync",
            country = %country.code,
            status = tracing::field::Empty,
        );
        match run(country).instrument(span.clone()).await {
            Ok(outcome) => {
                span.record("status", outcome.status());
                info!("{}", serde_json::to_string(&outcome).unwrap_or_default());
            }
            Err(e) => {
                span.record("status", "failed");
                error!("[{}] update failed: {e}", country.code);
                failed += 1;
            }
        }
    }
    if failed > 0 { 1 } else { 0 }
}
