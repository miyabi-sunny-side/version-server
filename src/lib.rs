//! version-server: one place that watches GitHub Releases and tells the LAN.
//!
//! Two inputs write through one gate ([`store::Store::ingest`]): the webhook
//! ([`webhook`]) for immediacy and the poller ([`github`]) as insurance. Three
//! outputs read the result: the current version per repo, the append-only
//! event log paged by id, and the same log as a live SSE stream.

// A spike: every fallible call returns `StoreError`, documented once on the type
// rather than as an `# Errors` section on each method.
#![allow(clippy::missing_errors_doc)]

pub mod github;
pub mod store;
pub mod webhook;

use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::Stream;
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{info, warn};

pub use store::Store;

/// The most events one page or one backfill batch returns.
const MAX_EVENTS: usize = 500;
const DEFAULT_EVENTS: usize = 100;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    /// `GITHUB_WEBHOOK_SECRET`. Without it the webhook route refuses everything,
    /// because an unverifiable delivery is worth nothing.
    pub webhook_secret: Option<String>,
    /// Woken on every new event; streams re-read the store when it fires.
    pub notify: broadcast::Sender<()>,
}

impl AppState {
    #[must_use]
    pub fn new(store: Arc<Store>, webhook_secret: Option<String>) -> Self {
        let (notify, _) = broadcast::channel(64);
        Self {
            store,
            webhook_secret,
            notify,
        }
    }
}

pub fn app(state: AppState, static_dir: impl AsRef<Path>) -> Router {
    let static_dir = static_dir.as_ref().to_path_buf();
    let api = Router::new()
        .route("/health", get(api_health))
        .fallback(api_not_found);
    let v1 = Router::new()
        .route("/versions", get(versions))
        .route("/versions/{org}/{repo}", get(version_of))
        .route("/events", get(events))
        .route("/events/stream", get(events_stream))
        .fallback(api_not_found);

    Router::new()
        .route("/healthz", get(healthz))
        .route("/webhook/github", post(github_webhook))
        .nest("/api", api)
        .nest("/v1", v1)
        .fallback_service(
            ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok\n"
}

async fn api_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn api_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "API route not found\n")
}

fn internal(error: impl std::fmt::Display) -> Response {
    warn!(%error, "request failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()
}

/// `POST /webhook/github`: 401 before reading an unverified body, 204 for a
/// delivery that announces no published release, 200 once it is recorded.
async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(secret) = state.webhook_secret.as_deref() else {
        return (StatusCode::UNAUTHORIZED, "webhook secret not configured\n").into_response();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|value| value.to_str().ok());
    if !webhook::verify_signature(secret, &body, signature) {
        return (StatusCode::UNAUTHORIZED, "bad signature\n").into_response();
    }
    // A ping (or any other event) carries no release: acknowledged, not recorded.
    let event = headers
        .get("x-github-event")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("release");
    if event != "release" {
        return StatusCode::NO_CONTENT.into_response();
    }
    let candidate = match webhook::parse_published(&body) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            return (StatusCode::BAD_REQUEST, format!("bad payload: {error}\n")).into_response();
        }
    };
    match state.store.ingest(&candidate) {
        Ok(Some(event)) => {
            info!(repo = %event.repo, tag = %event.tag, "release seen by webhook");
            let _ = state.notify.send(());
            (StatusCode::OK, Json(event)).into_response()
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({ "recorded": false })),
        )
            .into_response(),
        Err(error) => internal(error),
    }
}

async fn versions(State(state): State<AppState>) -> Response {
    match state.store.latest_all() {
        Ok(releases) => Json(releases).into_response(),
        Err(error) => internal(error),
    }
}

async fn version_of(
    State(state): State<AppState>,
    UrlPath((org, repo)): UrlPath<(String, String)>,
) -> Response {
    match state.store.latest(&format!("{org}/{repo}")) {
        Ok(Some(release)) => Json(release).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no release recorded\n").into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    since: i64,
    limit: Option<usize>,
}

impl EventsQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(DEFAULT_EVENTS).clamp(1, MAX_EVENTS)
    }
}

async fn events(State(state): State<AppState>, Query(query): Query<EventsQuery>) -> Response {
    match state.store.events_since(query.since, query.limit()) {
        Ok(events) => Json(events).into_response(),
        Err(error) => internal(error),
    }
}

/// `GET /v1/events/stream?since=<id>`: everything after `since` first, then each
/// event as it lands. The broadcast only says "something happened"; the events
/// themselves are re-read from the store from the last id sent, so nothing
/// between the backfill and the first wake-up is lost or sent twice.
async fn events_stream(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let mut wake = state.notify.subscribe();
    let store = state.store.clone();
    let mut cursor = query.since;
    let (tx, rx) = tokio::sync::mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        loop {
            match store.events_since(cursor, MAX_EVENTS) {
                Ok(batch) => {
                    for event in batch {
                        cursor = event.id;
                        let data = serde_json::to_string(&event).unwrap_or_default();
                        let sse = SseEvent::default()
                            .id(event.id.to_string())
                            .event("release")
                            .data(data);
                        // A closed receiver is the client hanging up: stop.
                        if tx.send(sse).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => warn!(%error, "stream read failed"),
            }
            match wake.recv().await {
                Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok);
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    fn state() -> AppState {
        AppState::new(Arc::new(Store::open_in_memory().unwrap()), None)
    }

    #[tokio::test]
    async fn liveness_is_lightweight_plain_text() {
        let response = app(state(), "client/dist")
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            "ok\n"
        );
    }

    #[tokio::test]
    async fn api_health_returns_stable_json() {
        let response = app(state(), "client/dist")
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            r#"{"status":"ok"}"#
        );
    }

    #[tokio::test]
    async fn the_webhook_refuses_everything_without_a_secret() {
        let response = app(state(), "client/dist")
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/github")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unknown_api_routes_do_not_fall_back_to_the_spa() {
        for uri in ["/api/missing", "/v1/missing"] {
            let response = app(state(), "client/dist")
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn unknown_client_routes_return_the_spa_with_success() {
        let response = app(state(), "client")
            .oneshot(
                Request::builder()
                    .uri("/somewhere")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-type").unwrap(), "text/html");
    }
}
