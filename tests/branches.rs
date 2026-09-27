use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt;
use version_server::{AppState, Store, app};

async fn push(
    state: &AppState,
    branch: &str,
    sha: &str,
    deleted: bool,
    valid_signature: bool,
) -> StatusCode {
    let body = json!({"ref": branch, "after": sha, "deleted": deleted,
        "repository": {"full_name": "o/r"}})
    .to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
    mac.update(body.as_bytes());
    let signature = if valid_signature {
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    } else {
        "sha256=bad".to_owned()
    };
    app(state.clone(), "client")
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header("x-github-event", "push")
                .header("x-hub-signature-256", signature)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn read(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let response = app(state.clone(), "client")
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn signed_pushes_persist_per_branch_without_changing_release_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let state = AppState::new(
        Arc::new(Store::open(path.to_str().unwrap()).unwrap()),
        Some("secret".into()),
    );
    let uri = "/v1/branches/o/r/master";
    assert_eq!(read(&state, uri).await.0, StatusCode::NOT_FOUND);
    let first = "a".repeat(40);
    let second = "b".repeat(40);
    assert_eq!(
        push(&state, "refs/heads/master", &first, false, false).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(read(&state, uri).await.0, StatusCode::NOT_FOUND);
    for sha in [&first, &second, &second] {
        assert_eq!(
            push(&state, "refs/heads/master", sha, false, true).await,
            StatusCode::OK
        );
        let (status, value) = read(&state, uri).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["sha"], *sha);
        assert_eq!(value["branch"], "master");
        assert_eq!(value["repo"], "o/r");
    }
    assert_eq!(
        push(&state, "refs/heads/topic/nested", &first, false, true).await,
        StatusCode::OK
    );
    assert_eq!(
        read(&state, "/v1/branches/o/r/topic/nested").await.1["sha"],
        first
    );
    assert_eq!(read(&state, uri).await.1["sha"], second);
    assert_eq!(read(&state, "/v1/events").await.1, json!([]));
    assert_eq!(read(&state, "/v1/versions").await.1, json!([]));
    drop(state);
    let reopened = AppState::new(Arc::new(Store::open(path.to_str().unwrap()).unwrap()), None);
    assert_eq!(read(&reopened, uri).await.1["sha"], second);
}

#[tokio::test]
async fn tags_deletions_and_invalid_pushes_never_become_update_signals() {
    let state = AppState::new(
        Arc::new(Store::open_in_memory().unwrap()),
        Some("secret".into()),
    );
    for (branch, sha, deleted, expected) in [
        (
            "refs/tags/v1.0.0",
            "a".repeat(40),
            false,
            StatusCode::NO_CONTENT,
        ),
        (
            "refs/heads/master",
            "0".repeat(40),
            true,
            StatusCode::NO_CONTENT,
        ),
        (
            "refs/heads/master",
            "$(touch /tmp/injected)".into(),
            false,
            StatusCode::BAD_REQUEST,
        ),
        (
            "refs/heads/master",
            "0".repeat(40),
            false,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(push(&state, branch, &sha, deleted, true).await, expected);
    }
    assert_eq!(
        read(&state, "/v1/branches/o/r/master").await.0,
        StatusCode::NOT_FOUND
    );
}
