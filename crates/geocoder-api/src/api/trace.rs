//! Request middlewares, the same as siren's.

use axum::extract::MatchedPath;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::{extract::Request, middleware::Next, response::Response};
use opentelemetry::Context;
use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use sentry::protocol::{SpanId, TraceId, User};
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Name of the request span made for OpenTelemetry only: the Sentry layer
/// skips it (`SentryHttpLayer` makes the request's transaction).
pub const OTEL_REQUEST_SPAN: &str = "http.server";

/// The request as an OpenTelemetry server span, child of the caller's W3C
/// `traceparent`: the OTLP trace joins the caller's, and a `parentbased_*`
/// sampler (`OTEL_TRACES_SAMPLER`) follows the caller's decision. The spans
/// of the handler (`geocoder.search`) and its logs nest in it.
pub async fn otel_request(request: Request, next: Next) -> Response {
    let span = {
        let method = request.method().as_str().to_owned();
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map_or_else(|| request.uri().path(), MatchedPath::as_str)
            .to_owned();
        let span = tracing::info_span!(
            "http.server",
            otel.name = %format!("{method} {route}"),
            otel.kind = "server",
            http.request.method = %method,
            http.route = %route,
            http.response.status_code = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        // From an empty context: without traceparent, a trace of its own,
        // whatever span the server runs in.
        let parent = TraceContextPropagator::new()
            .extract_with_context(&Context::new(), &Headers(request.headers()));
        // Fails only without an OpenTelemetry layer: nothing to join then.
        let _ = span.set_parent(parent);
        span
    };
    let response = next.run(request).instrument(span.clone()).await;
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());
    if status.is_server_error() {
        span.record("otel.status_code", "error");
    }
    response
}

struct Headers<'a>(&'a HeaderMap);

impl Extractor for Headers<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

/// W3C `traceparent` in, as `sentry-trace` for the Sentry layer, so the
/// request's transaction joins the caller's trace; `traceparent` out.
pub async fn traceparent(mut request: Request, next: Next) -> Response {
    let incoming = request
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_traceparent);

    let (trace_id, parent_span_id, flags) = match incoming {
        Some((t, p, f)) => (t, Some(p), f),
        None => (TraceId::default(), None, 1u8),
    };
    let our_span_id = SpanId::default();

    let sampled = if flags & 0x01 != 0 { "1" } else { "0" };
    let sentry_trace = match parent_span_id {
        Some(ref p) => format!("{trace_id}-{p}-{sampled}"),
        None => format!("{trace_id}-{our_span_id}-{sampled}"),
    };
    if let Ok(value) = HeaderValue::from_str(&sentry_trace) {
        request
            .headers_mut()
            .insert(HeaderName::from_static("sentry-trace"), value);
    }

    let mut response = next.run(request).await;

    let traceparent = format!("00-{trace_id}-{our_span_id}-{flags:02x}");
    if let Ok(value) = HeaderValue::from_str(&traceparent) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("traceparent"), value);
    }
    response
}

/// The client and tier set by the API key service behind the gateway
/// (`auth`), as the Sentry user and a `client.tier` tag.
pub async fn client_context(request: Request, next: Next) -> Response {
    // Owned before the await: a borrowed request would make the future !Send.
    let (client_id, tier) = {
        let header = |name: &str| {
            request
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        (header(CLIENT_ID_HEADER), header(TIER_HEADER))
    };
    if client_id.is_some() || tier.is_some() {
        sentry::configure_scope(|scope| {
            if let Some(id) = client_id {
                scope.set_user(Some(User {
                    id: Some(id),
                    ..Default::default()
                }));
            }
            if let Some(tier) = tier {
                scope.set_tag("client.tier", tier);
            }
        });
    }
    next.run(request).await
}

/// Set by the API key service of siren-backend, shared with siren.
const CLIENT_ID_HEADER: &str = "x-siren-client-id";
const TIER_HEADER: &str = "x-siren-tier";

fn parse_traceparent(header: &str) -> Option<(TraceId, SpanId, u8)> {
    let mut parts = header.splitn(4, '-');
    if parts.next()? != "00" {
        return None;
    }
    let trace_id = parts.next()?.parse().ok()?;
    let span_id = parts.next()?.parse().ok()?;
    let flags = u8::from_str_radix(parts.next()?, 16).ok()?;
    Some((trace_id, span_id, flags))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tower::ServiceExt;
    use tracing_subscriber::prelude::*;

    use super::*;

    /// The trace id and sampling of the handler's span, for a request with
    /// `traceparent`.
    async fn handler_trace(traceparent: Option<&str>) -> (String, bool) {
        let provider = SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let _guard = tracing::subscriber::set_default(subscriber);

        let app = Router::new()
            .route(
                "/search",
                get(|| async {
                    let cx = tracing::Span::current().context();
                    let span = cx.span();
                    let sc = span.span_context();
                    format!("{} {}", sc.trace_id(), sc.is_sampled())
                }),
            )
            .layer(axum::middleware::from_fn(otel_request));
        let mut request = axum::http::Request::get("/search?q=x");
        if let Some(tp) = traceparent {
            request = request.header("traceparent", tp);
        }
        let response = app
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        let (trace, sampled) = text.split_once(' ').unwrap();
        (trace.to_string(), sampled == "true")
    }

    #[tokio::test]
    async fn the_request_span_joins_the_callers_trace() {
        let (trace, sampled) = handler_trace(Some(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        ))
        .await;
        assert_eq!(trace, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert!(sampled);
        // Not sampled by the caller: not by us either (parent based).
        let (trace, sampled) = handler_trace(Some(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        ))
        .await;
        assert_eq!(trace, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert!(!sampled);
        // No caller: a trace of its own.
        let (trace, _) = handler_trace(None).await;
        assert_ne!(trace, "00000000000000000000000000000000");
        assert_ne!(trace, "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    #[test]
    fn traceparent_parsing() {
        let parsed = parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01");
        let (trace, span, flags) = parsed.unwrap();
        assert_eq!(trace.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(span.to_string(), "00f067aa0ba902b7");
        assert_eq!(flags, 1);
        assert!(
            parse_traceparent("01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").is_none()
        );
        assert!(parse_traceparent("00-nope-00f067aa0ba902b7-01").is_none());
    }
}
