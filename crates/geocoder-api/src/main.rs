//! geocoder-api: one HTTP server for the address indexes of several
//! countries, answering like the French BAN API.
//!
//! - `geocoder-api serve`: serves the indexes found in `GEOCODER_DATA_DIR`
//!   (`<country>.bin`), reloading each when its file changes.
//! - `geocoder-api update [--country xx] [--force]`: downloads the sources
//!   and rebuilds the indexes that changed.

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod api;
mod config;
mod handle;
mod sync;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing::{error, info};
use tracing_subscriber::{EnvFilter, prelude::*};

use config::Config;

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
    /// Download the sources and rebuild the indexes that changed.
    Update {
        /// Only this country (repeatable); default: every configured one.
        #[arg(long = "country")]
        only: Vec<String>,
        /// Rebuild even when the sources did not change.
        #[arg(long)]
        force: bool,
    },
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
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        // Same layer as siren; its replacement is not settled yet.
        .with(
            #[allow(deprecated)]
            sentry::integrations::tracing::layer(),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::from_env(
        &cli.countries,
        cli.default_country.as_deref(),
        &cli.data_dir,
    )
    .unwrap_or_else(|e| {
        error!("Invalid configuration: {e}");
        std::process::exit(2)
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Cannot build Tokio runtime");
    let code = runtime.block_on(async move {
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
            Command::Update { only, force } => update(&config, &only, force).await,
        }
    });
    std::process::exit(code)
}

/// Each country on its own: one failing source does not block the others.
async fn update(config: &Config, only: &[String], force: bool) -> i32 {
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
        match sync::sync(country, force).await {
            Ok(outcome) => info!("{}", serde_json::to_string(&outcome).unwrap_or_default()),
            Err(e) => {
                error!("[{}] update failed: {e}", country.code);
                failed += 1;
            }
        }
    }
    if failed > 0 { 1 } else { 0 }
}
