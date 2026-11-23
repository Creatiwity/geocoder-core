//! HTTP server.
//!
//! - `/search`, `/reverse`: BAN API compatible, plus `country`.
//! - `/countries`: what is served, and how fresh it is.
//! - `/health/live` (process only) and `/health/ready` (an index is loaded,
//!   not shutting down), mounted outside the Sentry and trace layers.
//! - `/openapi.json`, `/scalar`.

mod error;
mod geojson;
mod search;
mod trace;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
use serde::Serialize;
use tracing::info;
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

use crate::config::Config;
use crate::handle::Indexes;
use crate::sync::read_sidecar;

pub struct Context {
    pub config: Config,
    pub indexes: Arc<Indexes>,
    /// Set on SIGTERM, so readiness fails while in-flight requests drain.
    pub shutting_down: Arc<AtomicBool>,
}

#[derive(OpenApi)]
#[openapi(info(
    title = "Geocoder API",
    description = "Address geocoding for France (BAN), Luxembourg (BD-Adresses) and Belgium (BeST Address). `/search` and `/reverse` answer like the BAN API (api-adresse.data.gouv.fr), with a `country` parameter.",
    license(name = "MIT", url = "https://opensource.org/licenses/MIT")
))]
struct ApiDoc;

#[derive(Debug, Serialize, ToSchema)]
pub struct CountryStatus {
    pub code: String,
    /// Used when `country` is not given.
    pub default: bool,
    pub loaded: bool,
    pub documents: Option<usize>,
    pub profile: String,
    pub attribution: &'static str,
    pub licence: &'static str,
    /// When the index was built, and from which versions of the sources.
    pub built_at: Option<DateTime<Utc>>,
    #[schema(value_type = Vec<Object>)]
    pub sources: Vec<crate::sync::SourceVersion>,
}

/// Served countries
#[utoipa::path(
    get,
    path = "/countries",
    responses((status = 200, description = "Configured countries and their index", body = Vec<CountryStatus>))
)]
async fn countries(State(context): State<Arc<Context>>) -> Json<Vec<CountryStatus>> {
    let statuses = context
        .config
        .countries
        .iter()
        .map(|c| {
            let geocoder = context.indexes.get(&c.code).and_then(|h| h.get());
            let sidecar = read_sidecar(&c.index_path);
            CountryStatus {
                code: c.code.clone(),
                default: c.code == context.config.default_country,
                loaded: geocoder.is_some(),
                documents: geocoder.as_ref().map(|g| g.meta().documents as usize),
                profile: c.profile.to_string(),
                attribution: c.attribution,
                licence: c.licence,
                built_at: sidecar.as_ref().and_then(|s| s.built_at),
                sources: sidecar.map(|s| s.sources).unwrap_or_default(),
            }
        })
        .collect();
    Json(statuses)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    loaded_indexes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shutting_down: Option<bool>,
}

fn no_store(status: StatusCode, body: Health) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

async fn live() -> Response {
    no_store(
        StatusCode::OK,
        Health {
            status: "ok",
            loaded_indexes: None,
            shutting_down: None,
        },
    )
}

/// Ready once one index is loaded: a country still being built answers 503
/// on its own, the others are served. Before any index is built (first
/// install), ready all the same: searches answer 503, but a deployment does
/// not wait for a build that only runs once it is rolled out.
async fn ready(State(context): State<Arc<Context>>) -> Response {
    let shutting_down = context.shutting_down.load(Ordering::Relaxed);
    let loaded = context.indexes.loaded();
    let none_built = !context
        .config
        .countries
        .iter()
        .any(|c| c.index_path.exists());
    let ok = !shutting_down && (loaded > 0 || none_built);
    no_store(
        if ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Health {
            status: if ok { "ok" } else { "unavailable" },
            loaded_indexes: Some(loaded),
            shutting_down: Some(shutting_down),
        },
    )
}

pub fn app(context: Arc<Context>) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(search::search))
        .routes(routes!(search::reverse))
        .routes(routes!(countries))
        .split_for_parts();

    // Serialized once: the spec never changes after startup.
    let spec = Bytes::from(api.to_json().expect("Unable to serialize the OpenAPI spec"));
    let probes = Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route(
            "/openapi.json",
            get(move || async move { ([(header::CONTENT_TYPE, "application/json")], spec) }),
        );

    router
        .merge(Scalar::with_url("/scalar", api))
        .layer(cors_layer())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(SentryHttpLayer::new().enable_transaction())
        .layer(axum::middleware::from_fn(trace::client_context))
        .layer(NewSentryLayer::new_from_top())
        .layer(axum::middleware::from_fn(trace::traceparent))
        // Merged after the layers on purpose: probes stay out of Sentry
        // transactions and request traces.
        .merge(probes.layer(cors_layer()))
        .with_state(context)
}

pub async fn serve(addr: SocketAddr, context: Context, shutdown_delay: Duration) {
    let shutting_down = Arc::clone(&context.shutting_down);
    let app = app(Arc::new(context));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("Unable to listen on {addr}: {e}"));
    info!("Listening on {addr}");
    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal(shutting_down, shutdown_delay))
        .await
        .expect("server error");
}

fn cors_layer() -> tower_http::cors::CorsLayer {
    tower_http::cors::CorsLayer::new()
        .allow_methods([Method::GET])
        .allow_origin(tower_http::cors::Any)
}

/// On SIGTERM (the orchestrator), readiness starts failing and the server
/// keeps serving for `delay`, the time for the pod to leave the load
/// balancer; then in-flight requests are drained. Ctrl+C stops right away.
async fn shutdown_signal(shutting_down: Arc<AtomicBool>, delay: Duration) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Unable to listen for Ctrl+C");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Unable to listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Ctrl+C received, shutting down");
            shutting_down.store(true, Ordering::Relaxed);
        }
        _ = terminate => {
            info!("SIGTERM received, failing readiness for {delay:?} before shutting down");
            shutting_down.store(true, Ordering::Relaxed);
            tokio::time::sleep(delay).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use geocoder_core::{IndexBuilder, profile};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    fn context(dir: &std::path::Path) -> Arc<Context> {
        let mut builder = IndexBuilder::new(profile::by_name("lu").unwrap());
        let doc = json!({"id": "lu_7984", "type": "street", "name": "Kaesfurterstrooss",
            "postcode": "9755", "citycode": "0501", "city": "Hupperdange", "context": "Clervaux",
            "importance": 0.5, "lat": 50.0948, "lon": 6.0629,
            "housenumbers": {"20": {"id": "lu_210309", "lat": 50.094894, "lon": 6.062941}}});
        builder
            .add_document(serde_json::from_value(doc).unwrap())
            .unwrap();
        builder.build(&dir.join("lu.bin")).unwrap();
        let config = Config::new("lu,be", None, dir, |_| None).unwrap();
        Arc::new(Context {
            indexes: Indexes::open(&config),
            config,
            shutting_down: Arc::default(),
        })
    }

    async fn get(context: &Arc<Context>, uri: &str) -> (StatusCode, Value) {
        let response = app(Arc::clone(context))
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn search_answers_like_the_ban() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());

        let (status, body) = get(&context, "/search?q=20%20kaesfurterstrooss%20L-9755").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["type"], "FeatureCollection");
        assert_eq!(body["licence"], "CC0-1.0");
        let feature = &body["features"][0];
        assert_eq!(
            feature["geometry"]["coordinates"],
            json!([6.062941, 50.094894])
        );
        let p = &feature["properties"];
        assert_eq!(p["label"], "20 Kaesfurterstrooss 9755 Hupperdange");
        assert_eq!(p["type"], "housenumber");
        assert_eq!(p["id"], "lu_210309");
        assert_eq!(p["name"], "20 Kaesfurterstrooss");
        assert_eq!(p["street"], "Kaesfurterstrooss");

        let (status, body) = get(&context, "/reverse?lat=50.0949&lon=6.0629").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["features"][0]["properties"]["housenumber"], "20");
    }

    /// The search runs on a blocking thread: its span must still nest in the
    /// request's transaction, not start one of its own.
    #[tokio::test]
    async fn search_span_nests_in_the_request_transaction() {
        use sentry::SentryFutureExt;
        use sentry::protocol::EnvelopeItem;
        use tracing_subscriber::prelude::*;

        static SUBSCRIBER: std::sync::Once = std::sync::Once::new();
        SUBSCRIBER.call_once(|| {
            // Global: the blocking thread does not see a thread-local one.
            #[allow(deprecated)]
            let layer = sentry::integrations::tracing::layer();
            tracing_subscriber::registry().with(layer).init();
        });
        let transport = sentry::test::TestTransport::new();
        let mut options = sentry::ClientOptions::new().traces_sample_rate(1.0);
        options.dsn = "https://public@sentry.invalid/1".parse().ok();
        options.transport = Some(Arc::new(Arc::clone(&transport)));
        let client = sentry::Client::from_config(options);
        let hub = Arc::new(sentry::Hub::new(
            Some(Arc::new(client)),
            Arc::new(Default::default()),
        ));

        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());
        let (status, _) = get(&context, "/search?q=kaesfurterstrooss")
            .bind_hub(Arc::clone(&hub))
            .await;
        assert_eq!(status, StatusCode::OK);
        hub.client().unwrap().flush(None);

        let transactions: Vec<_> = transport
            .fetch_and_clear_envelopes()
            .into_iter()
            .flat_map(|e| e.into_items().collect::<Vec<_>>())
            .filter_map(|item| match item {
                EnvelopeItem::Transaction(t) => Some(t),
                _ => None,
            })
            .collect();
        let names: Vec<_> = transactions.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, [Some("GET /search".to_string())]);
        let ops: Vec<&str> = transactions[0]
            .spans
            .iter()
            .filter_map(|s| s.op.as_deref())
            .collect();
        for op in ["geocoder.search", "geocoder.collect", "geocoder.rank"] {
            assert!(ops.contains(&op), "{op} missing from {ops:?}");
        }
        let search = transactions[0]
            .spans
            .iter()
            .find(|s| s.op.as_deref() == Some("geocoder.search"))
            .unwrap();
        assert_eq!(search.data["results"], json!(1));
        assert_eq!(search.data["profile"], json!("lu"));
    }

    #[tokio::test]
    async fn ready_before_the_first_build_only() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new("lu", None, dir.path(), |_| None).unwrap();
        let context = Arc::new(Context {
            indexes: Indexes::open(&config),
            config,
            shutting_down: Arc::default(),
        });
        assert_eq!(get(&context, "/health/ready").await.0, StatusCode::OK);
        // A file that does not load: not ready.
        std::fs::write(dir.path().join("lu.bin"), b"broken").unwrap();
        assert_eq!(
            get(&context, "/health/ready").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn errors_and_probes() {
        let dir = tempfile::tempdir().unwrap();
        let context = context(dir.path());

        let (status, body) = get(&context, "/search?q=ab").await;
        assert_eq!(
            (status, body["code"].clone()),
            (StatusCode::BAD_REQUEST, json!(400))
        );
        let (status, _) = get(&context, "/search?q=rue%20x&country=de").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Configured, not built yet.
        let (status, _) = get(&context, "/search?q=rue%20x&country=be").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (_, body) = get(&context, "/countries").await;
        assert_eq!(body[0]["loaded"], true);
        assert_eq!(body[1]["loaded"], false);

        assert_eq!(get(&context, "/health/ready").await.0, StatusCode::OK);
        context.shutting_down.store(true, Ordering::Relaxed);
        assert_eq!(
            get(&context, "/health/ready").await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(get(&context, "/health/live").await.0, StatusCode::OK);
        assert_eq!(get(&context, "/openapi.json").await.0, StatusCode::OK);
    }
}
