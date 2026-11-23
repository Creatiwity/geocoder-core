//! Request middlewares, the same as siren's.

use axum::http::{HeaderName, HeaderValue};
use axum::{extract::Request, middleware::Next, response::Response};
use sentry::protocol::{SpanId, TraceId, User};

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
    use super::*;

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
