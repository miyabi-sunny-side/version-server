use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;
use version_server::{AppState, Store, github};

const DEFAULT_PORT: u16 = 3000;
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
    let raw = match env::var("PORT") {
        Ok(value) => Some(value),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            return Err("PORT must be a valid Unicode integer".into());
        }
    };
    Ok(SocketAddr::from((
        [0, 0, 0, 0],
        parse_port(raw.as_deref())?,
    )))
}

fn parse_port(raw: Option<&str>) -> Result<u16, Box<dyn Error>> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_PORT);
    };
    raw.parse::<u16>()
        .ok()
        .filter(|port| *port != 0 && raw.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| "PORT must be an integer from 1 to 65535".into())
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

#[cfg(test)]
mod tests {
    use super::parse_port;

    #[test]
    fn port_defaults_only_when_unset() {
        assert_eq!(parse_port(None).unwrap(), 3000);
        for (raw, expected) in [("1", 1), ("43127", 43127), ("65535", 65535)] {
            assert_eq!(parse_port(Some(raw)).unwrap(), expected);
        }
    }

    #[test]
    fn invalid_port_is_an_explicit_error() {
        for raw in [
            "",
            "0",
            "65536",
            "-1",
            "+3000",
            "3000 ",
            " 3000",
            "abc",
            "127.0.0.1:3000",
        ] {
            let error = parse_port(Some(raw)).unwrap_err();
            assert!(error.to_string().contains("PORT"), "{raw:?}: {error}");
        }
    }
}
