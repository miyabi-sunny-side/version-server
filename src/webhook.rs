//! GitHub release and push webhooks: prove who sent them, then read what we need.
//!
//! The signature is checked over the raw body before anything is parsed, so a
//! body that fails the check is never deserialised. Releases count on `published`;
//! branch pushes are separate deployment signals, excluding deletions.

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::{
    github::PayloadRelease,
    store::{Branch, ReleaseCandidate},
};

/// Whether `header` (the `X-Hub-Signature-256` value, `sha256=<hex>`) is the
/// HMAC of `body` under `secret`. Constant-time on the digest comparison.
#[must_use]
pub fn verify_signature(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(hex_digest) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_digest) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

#[derive(Deserialize)]
struct Payload {
    action: String,
    release: PayloadRelease,
    repository: PayloadRepository,
}

#[derive(Deserialize)]
struct PayloadRepository {
    full_name: String,
}

#[derive(Deserialize)]
struct PushPayload {
    #[serde(rename = "ref")]
    reference: String,
    after: String,
    deleted: bool,
    repository: PayloadRepository,
}

/// Branch pushes only; tag pushes and deletions cannot trigger deployment.
pub fn parse_push(body: &[u8]) -> Result<Option<Branch>, serde_json::Error> {
    let payload: PushPayload = serde_json::from_slice(body)?;
    let Some(branch) = payload.reference.strip_prefix("refs/heads/") else {
        return Ok(None);
    };
    if payload.deleted {
        return Ok(None);
    }
    if branch.is_empty()
        || payload.after.len() != 40
        || !payload.after.bytes().all(|b| b.is_ascii_hexdigit())
        || payload.after.bytes().all(|b| b == b'0')
    {
        return Err(serde::de::Error::custom("invalid branch or commit SHA"));
    }
    Ok(Some(Branch {
        repo: payload.repository.full_name,
        branch: branch.to_owned(),
        sha: payload.after,
    }))
}

/// The release a `published` payload announces. `Ok(None)` for any other action.
pub fn parse_published(body: &[u8]) -> Result<Option<ReleaseCandidate>, serde_json::Error> {
    let payload: Payload = serde_json::from_slice(body)?;
    if payload.action != "published" {
        return Ok(None);
    }
    Ok(Some(
        payload
            .release
            .into_candidate(payload.repository.full_name, "webhook"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn the_signature_must_match_the_body_under_the_secret() {
        let body = br#"{"action":"published"}"#;
        let good = sign("s", body);
        assert!(verify_signature("s", body, Some(&good)));
        assert!(!verify_signature("other", body, Some(&good)));
        assert!(!verify_signature("s", b"tampered", Some(&good)));
        assert!(!verify_signature("s", body, None));
        assert!(!verify_signature("s", body, Some("sha256=zz")));
        assert!(!verify_signature("s", body, Some("sha1=abcd")));
    }

    #[test]
    fn only_published_releases_are_read() {
        let body = br#"{"action":"published","release":{"tag_name":"v1.2.3","published_at":"2026-09-03T10:00:00Z","assets":[{"name":"x","browser_download_url":"https://e/x"}]},"repository":{"full_name":"o/r"}}"#;
        let candidate = parse_published(body).unwrap().unwrap();
        assert_eq!(candidate.repo, "o/r");
        assert_eq!(candidate.tag, "v1.2.3");
        assert_eq!(candidate.assets[0].digest, None);
        assert_eq!(candidate.source, "webhook");

        let edited =
            br#"{"action":"edited","release":{"tag_name":"v1"},"repository":{"full_name":"o/r"}}"#;
        assert!(parse_published(edited).unwrap().is_none());
        assert!(parse_published(b"not json").is_err());
    }
}
