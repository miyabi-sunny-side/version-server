//! The service seen from outside: webhook in, polling in, versions and events out.
//!
//! Every test runs against its own in-memory store, so nothing here touches the
//! disk or the network except the fake GitHub the poller test starts itself.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::Mutex;
use tower::ServiceExt;
use version_server::{AppState, Store, app, github::Poller};

const SECRET: &str = "s3cret";

fn state() -> AppState {
    AppState::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Some(SECRET.to_owned()),
    )
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn release_payload(action: &str, repo: &str, tag: &str, published_at: &str) -> Vec<u8> {
    json!({
        "action": action,
        "release": {
            "tag_name": tag,
            "published_at": published_at,
            "assets": [
                { "name": "app.tar.gz", "browser_download_url": "https://example.test/app.tar.gz", "digest": "sha256:abc" }
            ]
        },
        "repository": { "full_name": repo }
    })
    .to_string()
    .into_bytes()
}

async fn post_webhook(state: &AppState, body: Vec<u8>, signature: Option<&str>) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri("/webhook/github")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-github-event", "release");
    if let Some(signature) = signature {
        request = request.header("x-hub-signature-256", signature);
    }
    app(state.clone(), "client")
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap()
}

async fn get_json(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let response = app(state.clone(), "client")
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    // Refusals answer plain text; only success bodies are JSON.
    let value = if status.is_success() && !bytes.is_empty() {
        serde_json::from_slice(&bytes).unwrap()
    } else {
        Value::Null
    };
    (status, value)
}

fn ids(events: &Value) -> Vec<i64> {
    events
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn a_signed_published_release_is_recorded_once() {
    let state = state();
    let body = release_payload("published", "o/r", "v1.0.0", "2026-09-03T10:00:00Z");

    let response = post_webhook(&state, body.clone(), Some(&sign(&body))).await;
    assert_eq!(response.status(), StatusCode::OK);

    let (status, versions) = get_json(&state, "/v1/versions").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(versions.as_array().unwrap().len(), 1);
    assert_eq!(versions[0]["repo"], "o/r");
    assert_eq!(versions[0]["tag"], "v1.0.0");
    assert_eq!(versions[0]["source"], "webhook");
    assert_eq!(versions[0]["assets"][0]["digest"], "sha256:abc");
    let (_, events) = get_json(&state, "/v1/events").await;
    assert_eq!(ids(&events), [1]);

    // The same release delivered again (GitHub redelivers) adds no event.
    post_webhook(&state, body.clone(), Some(&sign(&body))).await;
    let (_, events) = get_json(&state, "/v1/events").await;
    assert_eq!(ids(&events), [1]);
}

#[tokio::test]
async fn a_bad_signature_is_refused_and_writes_nothing() {
    let state = state();
    let body = release_payload("published", "o/r", "v1.0.0", "2026-09-03T10:00:00Z");

    let response = post_webhook(&state, body.clone(), Some("sha256=deadbeef")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = post_webhook(&state, body.clone(), None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let (_, versions) = get_json(&state, "/v1/versions").await;
    assert!(versions.as_array().unwrap().is_empty());
    let (_, events) = get_json(&state, "/v1/events").await;
    assert!(events.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn other_release_actions_are_ignored() {
    let state = state();
    for action in ["created", "edited", "deleted", "prereleased"] {
        let body = release_payload(action, "o/r", "v1.0.0", "2026-09-03T10:00:00Z");
        let response = post_webhook(&state, body.clone(), Some(&sign(&body))).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "{action}");
    }
    let (_, events) = get_json(&state, "/v1/events").await;
    assert!(events.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn versions_answer_per_repo_and_events_page_in_id_order() {
    let state = state();
    for (repo, tag, at) in [
        ("o/a", "v1.0.0", "2026-09-03T10:00:00Z"),
        ("o/b", "v2.0.0", "2026-09-03T10:01:00Z"),
        ("o/a", "v1.1.0", "2026-09-03T10:02:00Z"),
        // An older release arriving late does not roll the latest back.
        ("o/a", "v0.9.0", "2026-09-01T00:00:00Z"),
    ] {
        let body = release_payload("published", repo, tag, at);
        post_webhook(&state, body.clone(), Some(&sign(&body))).await;
    }

    let (status, a) = get_json(&state, "/v1/versions/o/a").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(a["tag"], "v1.1.0");
    let (status, _) = get_json(&state, "/v1/versions/o/missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, all) = get_json(&state, "/v1/events").await;
    assert_eq!(ids(&all), [1, 2, 3]);
    let (_, page) = get_json(&state, "/v1/events?since=1&limit=1").await;
    assert_eq!(ids(&page), [2]);
    assert_eq!(page[0]["repo"], "o/b");
    let (_, rest) = get_json(&state, "/v1/events?since=2").await;
    assert_eq!(ids(&rest), [3]);
}

#[tokio::test]
async fn the_stream_backfills_then_stays_live() {
    let state = state();
    let body = release_payload("published", "o/a", "v1.0.0", "2026-09-03T10:00:00Z");
    post_webhook(&state, body.clone(), Some(&sign(&body))).await;

    let response = app(state.clone(), "client")
        .oneshot(
            Request::builder()
                .uri("/v1/events/stream?since=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut stream = response.into_body().into_data_stream();

    let first = String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.contains("id: 1"), "backfill first: {first}");
    assert!(first.contains(r#""tag":"v1.0.0""#), "{first}");

    let body = release_payload("published", "o/a", "v1.1.0", "2026-09-03T10:05:00Z");
    post_webhook(&state, body.clone(), Some(&sign(&body))).await;
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("a live event arrives")
        .unwrap()
        .unwrap();
    let second = String::from_utf8(second.to_vec()).unwrap();
    assert!(second.contains("id: 2"), "live second: {second}");
    assert!(second.contains(r#""tag":"v1.1.0""#), "{second}");
}

// --- polling against a fake GitHub ---

#[derive(Clone)]
struct FakeGithub {
    /// (etag, latest tag) served next; the test rotates it.
    current: Arc<Mutex<(String, String)>>,
    hits: Arc<Mutex<Vec<Option<String>>>>,
}

async fn fake_latest(State(fake): State<FakeGithub>, headers: HeaderMap) -> Response {
    let sent = headers
        .get(header::IF_NONE_MATCH)
        .map(|value| value.to_str().unwrap().to_owned());
    fake.hits.lock().await.push(sent.clone());
    let (etag, tag) = fake.current.lock().await.clone();
    if sent.as_deref() == Some(etag.as_str()) {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    (
        [(header::ETAG, etag)],
        axum::Json(json!({
            "tag_name": tag,
            "published_at": "2026-09-03T10:00:00Z",
            "assets": []
        })),
    )
        .into_response()
}

async fn serve_fake(fake: FakeGithub) -> SocketAddr {
    let router = Router::new()
        .route("/repos/{owner}/{repo}/releases/latest", get(fake_latest))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

#[tokio::test]
async fn polling_records_a_release_only_when_the_tag_changes() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let fake = FakeGithub {
        current: Arc::new(Mutex::new(("\"etag-1\"".to_owned(), "v1.0.0".to_owned()))),
        hits: Arc::new(Mutex::new(Vec::new())),
    };
    let addr = serve_fake(fake.clone()).await;
    let mut poller = Poller::new(&format!("http://{addr}"), None, vec!["o/r".to_owned()]);

    poller.poll_once(&store).await;
    poller.poll_once(&store).await;
    assert_eq!(store.events_since(0, 100).unwrap().len(), 1);
    assert_eq!(store.latest("o/r").unwrap().unwrap().source, "poll");

    let hits = fake.hits.lock().await.clone();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0], None, "the first poll has no ETag to send");
    assert_eq!(
        hits[1].as_deref(),
        Some("\"etag-1\""),
        "the second sends the ETag"
    );

    // A new release: new ETag, new tag -> one more event.
    *fake.current.lock().await = ("\"etag-2\"".to_owned(), "v1.1.0".to_owned());
    poller.poll_once(&store).await;
    let events = store.events_since(0, 100).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].tag, "v1.1.0");
}
