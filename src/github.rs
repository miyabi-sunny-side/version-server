//! The polling input: ask GitHub for each watched repo's latest release.
//!
//! This is the insurance behind the webhook. It asks with `If-None-Match`, so a
//! repo that has not changed costs one 304 and nothing is written; only a body
//! reaches [`Store::ingest`], which decides whether it is news.

use std::{collections::HashMap, sync::Arc, time::Duration};

use reqwest::{StatusCode, header};
use serde::Deserialize;
use tracing::{info, warn};

use crate::store::{Asset, ReleaseCandidate, Store};

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<LatestAsset>,
}

#[derive(Deserialize)]
struct LatestAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

pub struct Poller {
    client: reqwest::Client,
    api_base: String,
    token: Option<String>,
    repos: Vec<String>,
    etags: HashMap<String, String>,
}

impl Poller {
    /// `api_base` is `https://api.github.com` in production and a fake in tests.
    #[must_use]
    pub fn new(api_base: &str, token: Option<String>, repos: Vec<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("version-server")
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            api_base: api_base.trim_end_matches('/').to_owned(),
            token,
            repos,
            etags: HashMap::new(),
        }
    }

    /// One pass over every watched repo. Errors are logged, never fatal: the
    /// next pass tries again, and the webhook is still listening meanwhile.
    pub async fn poll_once(&mut self, store: &Store) -> Vec<crate::store::Event> {
        let mut events = Vec::new();
        for repo in self.repos.clone() {
            match self.poll_repo(&repo, store).await {
                Ok(Some(event)) => {
                    info!(repo, tag = %event.tag, "release seen by polling");
                    events.push(event);
                }
                Ok(None) => {}
                Err(error) => warn!(repo, %error, "poll failed"),
            }
        }
        events
    }

    async fn poll_repo(
        &mut self,
        repo: &str,
        store: &Store,
    ) -> Result<Option<crate::store::Event>, Box<dyn std::error::Error + Send + Sync>> {
        let mut request = self
            .client
            .get(format!("{}/repos/{repo}/releases/latest", self.api_base))
            .header(header::ACCEPT, "application/vnd.github+json");
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(etag) = self.etags.get(repo) {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = request.send().await?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        let response = response.error_for_status()?;
        if let Some(etag) = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
        {
            self.etags.insert(repo.to_owned(), etag.to_owned());
        }
        let latest: LatestRelease = response.json().await?;
        let candidate = ReleaseCandidate {
            repo: repo.to_owned(),
            tag: latest.tag_name,
            published_at: latest.published_at,
            assets: latest
                .assets
                .into_iter()
                .map(|asset| Asset {
                    name: asset.name,
                    url: asset.browser_download_url,
                    digest: asset.digest,
                })
                .collect(),
            source: "poll",
        };
        Ok(store.ingest(&candidate)?)
    }
}

/// Poll forever, every `interval`, telling `notify` about each event so open
/// streams wake up. Runs as a background task next to the HTTP server.
pub async fn run(
    mut poller: Poller,
    store: Arc<Store>,
    interval: Duration,
    notify: tokio::sync::broadcast::Sender<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let events = poller.poll_once(&store).await;
        if !events.is_empty() {
            // Nobody listening is not an error.
            let _ = notify.send(());
        }
    }
}
