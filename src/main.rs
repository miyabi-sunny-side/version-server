use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;
use version_server::{AppState, Store, github};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:3000";
const DEFAULT_DB_PATH: &str = "data/version-server.db";
const DEFAULT_POLL_SECS: u64 = 60;
const GITHUB_API: &str = "https://api.github.com";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind_addr = bind_addr_from_env()?;
    let db_path = env::var("VERSION_SERVER_DB").unwrap_or_else(|_| DEFAULT_DB_PATH.to_owned());
    if let Some(parent) = std::path::Path::new(&db_path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let store = Arc::new(Store::open(&db_path)?);
    // Secrets stay in env and out of the log: only their presence is reported.
    let webhook_secret = env::var("GITHUB_WEBHOOK_SECRET")
        .ok()
        .filter(|s| !s.is_empty());
    info!(webhook = webhook_secret.is_some(), db = %db_path, "configured");
    let state = AppState::new(store.clone(), webhook_secret);

    let repos = watch_repos_from_env();
    if repos.is_empty() {
        info!("WATCH_REPOS is empty: polling disabled, webhook only");
    } else {
        let token = env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty());
        let interval = env::var("POLL_SECS")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(DEFAULT_POLL_SECS);
        let api_base = env::var("GITHUB_API_URL").unwrap_or_else(|_| GITHUB_API.to_owned());
        info!(
            repos = repos.len(),
            interval,
            token = token.is_some(),
            "polling enabled"
        );
        tokio::spawn(github::run(
            github::Poller::new(&api_base, token, repos),
            store,
            Duration::from_secs(interval),
            state.notify.clone(),
        ));
    }

    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");
    axum::serve(listener, version_server::app(state, "client/dist"))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("server stopped");
    Ok(())
}

fn bind_addr_from_env() -> Result<SocketAddr, Box<dyn Error>> {
    env::var("APP_BIND_ADDR")
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned())
        .parse()
        .map_err(Into::into)
}

/// `WATCH_REPOS`: `org/repo` names separated by commas, blanks ignored.
fn watch_repos_from_env() -> Vec<String> {
    env::var("WATCH_REPOS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|repo| !repo.is_empty())
        .map(str::to_owned)
        .collect()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
