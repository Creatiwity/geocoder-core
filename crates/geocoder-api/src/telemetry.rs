//! Traces and monitors, like siren:
//!
//! - Sentry (`SENTRY_DSN`): errors, HTTP transactions, the spans of
//!   geocoder-core (`geocoder.search`, `geocoder.import`...) and of the jobs,
//!   Crons check-ins of the `update` CronJob (`--crontab`).
//! - OpenTelemetry (`OTEL_EXPORTER_OTLP_ENDPOINT`, plus the standard `OTEL_*`
//!   variables): the same spans, exported over OTLP/HTTP.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use opentelemetry::global;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use sentry::protocol::{
    MonitorCheckIn, MonitorCheckInStatus, MonitorConfig, MonitorSchedule, Uuid,
};

const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Kept for [`exit`], which cannot reach the provider owned by `main`.
static PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

/// The OTLP exporter, only when an endpoint is configured. Runs before the
/// subscriber exists: a failure goes to stderr, not to `tracing`.
pub fn init_otlp() -> Option<SdkTracerProvider> {
    std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;
    global::set_text_map_propagator(TraceContextPropagator::new());
    let exporter = match span_exporter() {
        Ok(exporter) => exporter,
        Err(e) => {
            eprintln!("OTLP export disabled: {e}");
            return None;
        }
    };
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    global::set_tracer_provider(provider.clone());
    let _ = PROVIDER.set(provider.clone());
    Some(provider)
}

fn span_exporter()
-> Result<opentelemetry_otlp::SpanExporter, opentelemetry_otlp::ExporterBuildError> {
    opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
}

/// Exits after sending what is still buffered: `process::exit` runs no
/// destructor, so neither the Sentry guard nor the OTLP batch would flush,
/// and the error of a failed job would be lost.
pub fn exit(code: i32) -> ! {
    if let Some(client) = sentry::Hub::current().client() {
        client.flush(Some(FLUSH_TIMEOUT));
    }
    if let Some(provider) = PROVIDER.get()
        && let Err(e) = provider.shutdown()
    {
        tracing::warn!("OTLP shutdown error: {e}");
    }
    std::process::exit(code)
}

/// `spawn_blocking` keeping the caller's span and Sentry hub: without them,
/// the spans opened in the closure (a search) would start transactions of
/// their own instead of nesting in the request's.
pub fn spawn_blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let span = tracing::Span::current();
    let hub = sentry::Hub::current();
    tokio::task::spawn_blocking(move || sentry::Hub::run(hub, || span.in_scope(f)))
}

/// A Sentry Crons monitor for a scheduled job: `in_progress` when created,
/// then `ok` or `error` with the duration. The monitor is created (or its
/// schedule updated) by the first check-in. Nothing without `SENTRY_DSN`.
pub struct CronMonitor {
    slug: String,
    check_in_id: Uuid,
    schedule: String,
    started: Instant,
}

impl CronMonitor {
    pub fn start(slug: &str, crontab: &str) -> Self {
        let monitor = Self {
            slug: slug.to_string(),
            check_in_id: sentry::types::random_uuid(),
            schedule: crontab.to_string(),
            started: Instant::now(),
        };
        monitor.send(MonitorCheckInStatus::InProgress, None);
        monitor
    }

    pub fn finish(self, ok: bool) {
        let status = if ok {
            MonitorCheckInStatus::Ok
        } else {
            MonitorCheckInStatus::Error
        };
        self.send(status, Some(self.started.elapsed().as_secs_f64()));
    }

    fn send(&self, status: MonitorCheckInStatus, duration: Option<f64>) {
        let Some(client) = sentry::Hub::current().client() else {
            return;
        };
        let check_in = MonitorCheckIn {
            check_in_id: self.check_in_id,
            monitor_slug: self.slug.clone(),
            status,
            environment: client.options().environment.as_ref().map(|e| e.to_string()),
            duration,
            monitor_config: Some(MonitorConfig {
                schedule: MonitorSchedule::Crontab {
                    value: self.schedule.clone(),
                },
                checkin_margin: Some(5),
                // A build from the sources takes ~10 min (France).
                max_runtime: Some(120),
                timezone: Some("UTC".into()),
                failure_issue_threshold: Some(1),
                recovery_threshold: Some(1),
            }),
        };
        client.send_envelope(check_in.into());
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use opentelemetry::trace::{Tracer, TracerProvider as _};
    use opentelemetry_otlp::WithExportConfig;

    use super::*;

    /// Without an HTTP client feature the exporter does not build, and
    /// `init_otlp` would export nothing.
    #[test]
    fn the_span_exporter_builds() {
        span_exporter().expect("opentelemetry-otlp needs an HTTP client feature");
    }

    /// A span reaches an OTLP/HTTP endpoint through the batch processor.
    #[test]
    fn spans_are_exported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
        let received = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(endpoint)
            .build()
            .unwrap();
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        provider.tracer("test").in_span("geocoder.search", |_| {});
        provider.force_flush().unwrap();

        let request = received.join().unwrap();
        assert!(request.starts_with("POST /v1/traces "), "{request}");
        provider.shutdown().unwrap();
    }
}
