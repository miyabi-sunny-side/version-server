//! The one place a release is written down.
//!
//! Both inputs — the webhook and the poller — hand a [`ReleaseCandidate`] to
//! [`Store::ingest`], which decides in one transaction whether it is news: a tag
//! the repo has not shown yet, and not older than the one it shows now. Only
//! news becomes an event; a redelivered or already-known release just refreshes
//! the snapshot. Consumers read the snapshot (`releases`) for "what is current"
//! and the append-only `events` for "what changed since".

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS releases (
  repo         TEXT PRIMARY KEY,
  tag          TEXT NOT NULL,
  published_at TEXT,
  assets_json  TEXT NOT NULL,
  source       TEXT NOT NULL,
  received_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
  id   INTEGER PRIMARY KEY AUTOINCREMENT,
  repo TEXT NOT NULL,
  tag  TEXT NOT NULL,
  at   TEXT NOT NULL
);
";

/// One published asset of a release, as much of it as GitHub tells us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub digest: Option<String>,
}

/// What an input learned about a repo's latest release. Where it came from is
/// part of the record, so a reader can tell a webhook delivery from a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCandidate {
    pub repo: String,
    pub tag: String,
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
    pub source: &'static str,
}

/// The current release of one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    pub repo: String,
    pub tag: String,
    pub published_at: Option<String>,
    pub assets: Vec<Asset>,
    pub source: String,
    pub received_at: String,
}

/// One change of a repo's current tag, in the order the store saw them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub id: i64,
    pub repo: String,
    pub tag: String,
    pub at: String,
}

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Poisoned,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "sqlite: {error}"),
            Self::Json(error) => write!(f, "json: {error}"),
            Self::Poisoned => write!(f, "store lock poisoned"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &str) -> Result<Self, StoreError> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record what an input learned. Returns the event this made, if it was news.
    ///
    /// News is a tag the repo does not currently show, unless the candidate is
    /// older than the current one (a late webhook for an earlier release): then
    /// nothing moves, because "latest" means latest published, not latest heard.
    pub fn ingest(&self, candidate: &ReleaseCandidate) -> Result<Option<Event>, StoreError> {
        let now = now_rfc3339();
        let conn = self.conn.lock().map_err(|_| StoreError::Poisoned)?;
        let tx = conn.unchecked_transaction()?;
        let current: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT tag, published_at FROM releases WHERE repo = ?1",
                [&candidate.repo],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let assets_json = serde_json::to_string(&candidate.assets)?;
        let event = match current {
            Some((tag, _)) if tag == candidate.tag => {
                // Known release: keep the snapshot fresh, but it is not news.
                tx.execute(
                    "UPDATE releases SET published_at = ?2, assets_json = ?3, source = ?4,
                            received_at = ?5
                     WHERE repo = ?1",
                    params![
                        candidate.repo,
                        candidate.published_at,
                        assets_json,
                        candidate.source,
                        now
                    ],
                )?;
                None
            }
            Some((_, current_at))
                if is_older(candidate.published_at.as_deref(), current_at.as_deref()) =>
            {
                None
            }
            _ => {
                tx.execute(
                    "INSERT INTO releases (repo, tag, published_at, assets_json, source, received_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(repo) DO UPDATE SET tag = excluded.tag,
                        published_at = excluded.published_at, assets_json = excluded.assets_json,
                        source = excluded.source, received_at = excluded.received_at",
                    params![
                        candidate.repo,
                        candidate.tag,
                        candidate.published_at,
                        assets_json,
                        candidate.source,
                        now
                    ],
                )?;
                tx.execute(
                    "INSERT INTO events (repo, tag, at) VALUES (?1, ?2, ?3)",
                    params![candidate.repo, candidate.tag, now],
                )?;
                Some(Event {
                    id: tx.last_insert_rowid(),
                    repo: candidate.repo.clone(),
                    tag: candidate.tag.clone(),
                    at: now,
                })
            }
        };
        tx.commit()?;
        Ok(event)
    }

    /// Every repo's current release, by repo name.
    pub fn latest_all(&self) -> Result<Vec<Release>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Poisoned)?;
        let mut statement = conn.prepare(
            "SELECT repo, tag, published_at, assets_json, source, received_at
             FROM releases ORDER BY repo",
        )?;
        let rows = statement.query_map([], release_from_row)?;
        rows.map(|row| row.map_err(StoreError::from).and_then(|r| r))
            .collect()
    }

    pub fn latest(&self, repo: &str) -> Result<Option<Release>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Poisoned)?;
        conn.query_row(
            "SELECT repo, tag, published_at, assets_json, source, received_at
             FROM releases WHERE repo = ?1",
            [repo],
            release_from_row,
        )
        .optional()?
        .transpose()
    }

    /// Events with an id greater than `since`, oldest first, at most `limit`.
    pub fn events_since(&self, since: i64, limit: usize) -> Result<Vec<Event>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Poisoned)?;
        let mut statement = conn.prepare(
            "SELECT id, repo, tag, at FROM events WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = statement.query_map(params![since, limit], |row| {
            Ok(Event {
                id: row.get(0)?,
                repo: row.get(1)?,
                tag: row.get(2)?,
                at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

type ReleaseRow = Result<Release, StoreError>;

fn release_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReleaseRow> {
    let assets_json: String = row.get(3)?;
    Ok(serde_json::from_str(&assets_json)
        .map_err(StoreError::from)
        .and_then(|assets| {
            Ok(Release {
                repo: row.get(0)?,
                tag: row.get(1)?,
                published_at: row.get(2)?,
                assets,
                source: row.get(4)?,
                received_at: row.get(5)?,
            })
        }))
}

/// Whether `candidate` was published before `current`. Unknown dates never
/// count as older, so a release without a date can still become current.
fn is_older(candidate: Option<&str>, current: Option<&str>) -> bool {
    match (candidate, current) {
        (Some(candidate), Some(current)) => {
            match (parse_rfc3339(candidate), parse_rfc3339(current)) {
                (Some(candidate), Some(current)) => candidate < current,
                _ => false,
            }
        }
        _ => false,
    }
}

fn parse_rfc3339(raw: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(raw, &Rfc3339).ok()
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        repo: &str,
        tag: &str,
        at: Option<&str>,
        source: &'static str,
    ) -> ReleaseCandidate {
        ReleaseCandidate {
            repo: repo.into(),
            tag: tag.into(),
            published_at: at.map(str::to_owned),
            assets: vec![Asset {
                name: "a".into(),
                url: "https://example.test/a".into(),
                digest: None,
            }],
            source,
        }
    }

    #[test]
    fn a_new_tag_is_news_and_the_same_tag_is_not() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .ingest(&candidate(
                "o/r",
                "v1.0.0",
                Some("2026-09-03T10:00:00Z"),
                "webhook",
            ))
            .unwrap();
        assert_eq!(first.as_ref().map(|e| e.id), Some(1));
        let again = store
            .ingest(&candidate(
                "o/r",
                "v1.0.0",
                Some("2026-09-03T10:00:00Z"),
                "poll",
            ))
            .unwrap();
        assert!(again.is_none(), "a redelivery is not news");
        let latest = store.latest("o/r").unwrap().unwrap();
        assert_eq!(
            latest.source, "poll",
            "the snapshot still records the latest input"
        );
        let next = store
            .ingest(&candidate(
                "o/r",
                "v1.1.0",
                Some("2026-09-03T11:00:00Z"),
                "webhook",
            ))
            .unwrap()
            .unwrap();
        assert_eq!(next.id, 2);
        assert_eq!(store.latest("o/r").unwrap().unwrap().tag, "v1.1.0");
    }

    #[test]
    fn an_older_release_arriving_late_does_not_roll_the_latest_back() {
        let store = Store::open_in_memory().unwrap();
        store
            .ingest(&candidate(
                "o/r",
                "v1.1.0",
                Some("2026-09-03T11:00:00Z"),
                "webhook",
            ))
            .unwrap();
        let late = store
            .ingest(&candidate(
                "o/r",
                "v1.0.0",
                Some("2026-09-03T10:00:00Z"),
                "webhook",
            ))
            .unwrap();
        assert!(late.is_none());
        assert_eq!(store.latest("o/r").unwrap().unwrap().tag, "v1.1.0");
        // Without a date on either side there is nothing to compare: it lands.
        let undated = store
            .ingest(&candidate("o/r", "v2.0.0", None, "poll"))
            .unwrap();
        assert!(undated.is_some());
    }

    #[test]
    fn events_page_forward_from_since() {
        let store = Store::open_in_memory().unwrap();
        for tag in ["v1", "v2", "v3"] {
            store.ingest(&candidate("o/r", tag, None, "poll")).unwrap();
        }
        let page = store.events_since(1, 1).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, 2);
        assert_eq!(store.events_since(3, 10).unwrap().len(), 0);
        assert_eq!(store.latest_all().unwrap().len(), 1);
    }
}
