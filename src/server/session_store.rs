//! Server-side web console sessions persisted in a dedicated SQLite database.
//!
//! Browsers only hold a random opaque token; the database stores its SHA-256
//! digest, so a leaked database cannot be replayed as cookies. Sessions are
//! never rotated on renewal (parallel requests and tabs would race), only
//! extended up to a fixed absolute lifetime.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// Bumped whenever the table layout changes; sessions from other versions are
/// dropped, signing everyone out once.
const SESSION_SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS web_sessions (
    token_hash BLOB PRIMARY KEY CHECK(length(token_hash) = 32),
    principal_kind TEXT NOT NULL CHECK(principal_kind IN ('admin', 'user')),
    principal_id TEXT NOT NULL,
    credential_tag BLOB NOT NULL CHECK(length(credential_tag) = 32),
    created_at_ms INTEGER NOT NULL,
    renewed_at_ms INTEGER NOT NULL,
    idle_expires_at_ms INTEGER NOT NULL,
    absolute_expires_at_ms INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS web_sessions_principal ON web_sessions(principal_kind, principal_id);
CREATE INDEX IF NOT EXISTS web_sessions_expiry ON web_sessions(idle_expires_at_ms);
DROP TABLE IF EXISTS principal_revocations;
"#;

/// Sessions are only renewed (and the cookie re-issued) this long after the
/// previous renewal, keeping writes rare for polling dashboards.
pub const RENEWAL_INTERVAL_MS: i64 = 5 * 60 * 1_000;
/// Oldest sessions beyond this count are dropped when a principal signs in again.
const MAX_SESSIONS_PER_PRINCIPAL: i64 = 20;
/// A session is valid until either deadline passes.
const VALID_SESSION: &str = "idle_expires_at_ms > ?2 AND absolute_expires_at_ms > ?2";

const TOKEN_BYTES: usize = 32;
const MAX_TOKEN_CHARS: usize = 64;

/// Who a console session acts for. User ids are stable record UUIDs, never the
/// credential itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    Admin,
    /// A downstream user, signed in with any of their credentials.
    User(String),
}

impl Principal {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User(_) => "user",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Admin => "root",
            Self::User(id) => id,
        }
    }

    pub const fn is_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    fn from_parts(kind: &str, id: String) -> Option<Self> {
        match kind {
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User(id)),
            _ => None,
        }
    }

    /// Idle and absolute lifetimes in milliseconds.
    pub const fn lifetime(&self) -> SessionLifetime {
        const HOUR: i64 = 60 * 60 * 1_000;
        const DAY: i64 = 24 * HOUR;
        match self {
            // A stolen admin cookie can mint long-lived API keys, so keep it short.
            Self::Admin => SessionLifetime {
                idle_ms: 24 * HOUR,
                absolute_ms: 7 * DAY,
            },
            // Read-only usage views: remember the browser for a long time.
            Self::User(_) => SessionLifetime {
                idle_ms: 30 * DAY,
                absolute_ms: 90 * DAY,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLifetime {
    pub idle_ms: i64,
    pub absolute_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub token_hash: [u8; 32],
    pub principal: Principal,
    pub credential_tag: [u8; 32],
    pub created_at_ms: i64,
    pub renewed_at_ms: i64,
    pub idle_expires_at_ms: i64,
    pub absolute_expires_at_ms: i64,
}

impl SessionRecord {
    /// When the browser cookie should expire.
    pub fn expires_at_ms(&self) -> i64 {
        self.idle_expires_at_ms.min(self.absolute_expires_at_ms)
    }
}

#[derive(Clone)]
pub struct SessionStore {
    connection: Arc<Mutex<Connection>>,
}

impl SessionStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create session database directory {}",
                    parent.display()
                )
            })?;
        }
        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open session database {}", path.display()))?;
        restrict_permissions(&path)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .context("failed to configure session database busy timeout")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable session database WAL mode")?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .context("failed to configure session database synchronization")?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .context("failed to read session database version")?;
        let reset = if version == SESSION_SCHEMA_VERSION {
            ""
        } else {
            "DROP TABLE IF EXISTS web_sessions;"
        };
        connection
            .execute_batch(&format!(
                "BEGIN; {reset} {SCHEMA} PRAGMA user_version = {SESSION_SCHEMA_VERSION}; COMMIT;"
            ))
            .context("failed to initialize session database schema")?;
        restrict_permissions(&path)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Creates a session and returns the opaque cookie token.
    pub async fn create(
        &self,
        principal: Principal,
        credential_tag: [u8; 32],
        now_ms: i64,
    ) -> Result<(String, SessionRecord)> {
        let mut raw = [0_u8; TOKEN_BYTES];
        raw[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        raw[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        let token = URL_SAFE_NO_PAD.encode(raw);
        let lifetime = principal.lifetime();
        let record = SessionRecord {
            token_hash: token_hash(&token),
            principal,
            credential_tag,
            created_at_ms: now_ms,
            renewed_at_ms: now_ms,
            idle_expires_at_ms: now_ms + lifetime.idle_ms,
            absolute_expires_at_ms: now_ms + lifetime.absolute_ms,
        };
        let stored = record.clone();
        self.run(move |connection| {
            connection.execute(
                "INSERT INTO web_sessions (token_hash, principal_kind, principal_id, credential_tag, created_at_ms, renewed_at_ms, idle_expires_at_ms, absolute_expires_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    stored.token_hash.as_slice(),
                    stored.principal.kind(),
                    stored.principal.id(),
                    stored.credential_tag.as_slice(),
                    stored.created_at_ms,
                    stored.renewed_at_ms,
                    stored.idle_expires_at_ms,
                    stored.absolute_expires_at_ms,
                ],
            )?;
            connection.execute(
                "DELETE FROM web_sessions WHERE principal_kind = ?1 AND principal_id = ?2 AND token_hash NOT IN (SELECT token_hash FROM web_sessions WHERE principal_kind = ?1 AND principal_id = ?2 ORDER BY created_at_ms DESC, token_hash LIMIT ?3)",
                params![stored.principal.kind(), stored.principal.id(), MAX_SESSIONS_PER_PRINCIPAL],
            )?;
            Ok(())
        })
        .await?;
        Ok((token, record))
    }

    /// Returns the unexpired session for a cookie token.
    pub async fn find(&self, token: &str, now_ms: i64) -> Result<Option<SessionRecord>> {
        let Some(hash) = valid_token_hash(token) else {
            return Ok(None);
        };
        self.run(move |connection| {
            let row = connection
                .query_row(
                    &format!("SELECT principal_kind, principal_id, credential_tag, created_at_ms, renewed_at_ms, idle_expires_at_ms, absolute_expires_at_ms FROM web_sessions WHERE token_hash = ?1 AND {VALID_SESSION}"),
                    params![hash.as_slice(), now_ms],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    },
                )
                .optional()?;
            Ok(row.and_then(|(kind, id, tag, created, renewed, idle, absolute)| {
                Some(SessionRecord {
                    token_hash: hash,
                    principal: Principal::from_parts(&kind, id)?,
                    credential_tag: tag.try_into().ok()?,
                    created_at_ms: created,
                    renewed_at_ms: renewed,
                    idle_expires_at_ms: idle,
                    absolute_expires_at_ms: absolute,
                })
            }))
        })
        .await
    }

    /// Extends a valid session's idle deadline, capped at its absolute deadline,
    /// at most once per [`RENEWAL_INTERVAL_MS`]. Returns `true` only when this
    /// call performed the renewal, so concurrent requests re-issue the cookie once.
    pub async fn renew_if_due(&self, record: &mut SessionRecord, now_ms: i64) -> Result<bool> {
        if now_ms - record.renewed_at_ms < RENEWAL_INTERVAL_MS {
            return Ok(false);
        }
        let hash = record.token_hash;
        let idle_ms = record.principal.lifetime().idle_ms;
        let next_idle = (now_ms + idle_ms).min(record.absolute_expires_at_ms);
        let updated = self
            .run(move |connection| {
                Ok(connection.execute(
                    &format!("UPDATE web_sessions SET renewed_at_ms = ?2, idle_expires_at_ms = ?3 WHERE token_hash = ?1 AND renewed_at_ms <= ?4 AND {VALID_SESSION}"),
                    params![hash.as_slice(), now_ms, next_idle, now_ms - RENEWAL_INTERVAL_MS],
                )?)
            })
            .await?;
        if updated == 1 {
            record.renewed_at_ms = now_ms;
            record.idle_expires_at_ms = next_idle;
        }
        Ok(updated == 1)
    }

    pub async fn revoke(&self, token_hash: [u8; 32]) -> Result<()> {
        self.run(move |connection| {
            connection.execute(
                "DELETE FROM web_sessions WHERE token_hash = ?1",
                params![token_hash.as_slice()],
            )?;
            Ok(())
        })
        .await
    }

    /// Deletes every session of one principal. Callers serialize this with
    /// sign-in (see `AppState::credential_lock`) so no session can be created
    /// concurrently from the credential being revoked.
    pub async fn revoke_principal(&self, principal: &Principal) -> Result<usize> {
        let kind = principal.kind();
        let id = principal.id().to_owned();
        self.run(move |connection| {
            Ok(connection.execute(
                "DELETE FROM web_sessions WHERE principal_kind = ?1 AND principal_id = ?2",
                params![kind, id],
            )?)
        })
        .await
    }

    pub async fn purge_expired(&self, now_ms: i64) -> Result<usize> {
        self.run(move |connection| {
            Ok(connection.execute(
                "DELETE FROM web_sessions WHERE idle_expires_at_ms <= ?1 OR absolute_expires_at_ms <= ?1",
                params![now_ms],
            )?)
        })
        .await
    }

    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("session database lock poisoned");
            operation(&connection)
        })
        .await
        .context("session database task failed")?
    }
}

/// SHA-256 digest of a cookie token, as stored in the database.
pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn valid_token_hash(token: &str) -> Option<[u8; 32]> {
    if token.is_empty() || token.len() > MAX_TOKEN_CHARS {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(token).ok()?;
    (decoded.len() == TOKEN_BYTES).then(|| token_hash(token))
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    for suffix in ["", "-wal", "-shm"] {
        let mut candidate = path.as_os_str().to_os_string();
        candidate.push(suffix);
        let candidate = PathBuf::from(candidate);
        match std::fs::metadata(&candidate) {
            Ok(metadata) => {
                let mut permissions = metadata.permissions();
                permissions.set_mode(0o600);
                std::fs::set_permissions(&candidate, permissions)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (SessionStore, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "codex-router-sessions-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        (SessionStore::open(path.clone()).unwrap(), path)
    }

    fn cleanup(path: PathBuf) {
        for suffix in ["", "-wal", "-shm"] {
            let mut candidate = path.as_os_str().to_os_string();
            candidate.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(candidate));
        }
    }

    #[tokio::test]
    async fn creates_finds_renews_and_revokes_sessions() {
        let (store, path) = store();
        let now = 1_800_000_000_000;
        let principal = Principal::User("00000000-0000-4000-8000-000000000001".into());
        let (token, created) = store.create(principal.clone(), [7; 32], now).await.unwrap();
        assert_eq!(token.len(), 43);

        let mut found = store.find(&token, now + 1).await.unwrap().unwrap();
        assert_eq!(found, created);
        assert!(store.find("not-a-token", now).await.unwrap().is_none());

        // Renewal is throttled, and a stale copy cannot renew twice.
        assert!(!store.renew_if_due(&mut found, now + 1_000).await.unwrap());
        let mut stale = found.clone();
        let lifetime = principal.lifetime();
        let later = now + lifetime.idle_ms - 1;
        assert!(store.renew_if_due(&mut found, later).await.unwrap());
        assert_eq!(found.idle_expires_at_ms, later + lifetime.idle_ms);
        stale.renewed_at_ms = now - RENEWAL_INTERVAL_MS;
        assert!(!store.renew_if_due(&mut stale, later + 1).await.unwrap());
        assert!(
            store
                .find(&token, now + lifetime.idle_ms + 1)
                .await
                .unwrap()
                .is_some()
        );

        // Renewal never extends beyond the absolute deadline.
        let mut at = later;
        while at + lifetime.idle_ms < found.absolute_expires_at_ms {
            at += lifetime.idle_ms - 1;
            assert!(store.renew_if_due(&mut found, at).await.unwrap());
        }
        assert_eq!(found.idle_expires_at_ms, found.absolute_expires_at_ms);
        assert!(
            store
                .find(&token, found.absolute_expires_at_ms)
                .await
                .unwrap()
                .is_none()
        );

        store.revoke(created.token_hash).await.unwrap();
        assert!(store.find(&token, now + 2).await.unwrap().is_none());
        cleanup(path);
    }

    #[tokio::test]
    async fn revokes_all_sessions_of_a_principal() {
        let (store, path) = store();
        let now = 1_800_000_000_000;
        let principal = Principal::User("00000000-0000-4000-8000-000000000002".into());
        let (first, _) = store.create(principal.clone(), [1; 32], now).await.unwrap();
        let (second, _) = store.create(principal.clone(), [1; 32], now).await.unwrap();
        let (admin, _) = store.create(Principal::Admin, [2; 32], now).await.unwrap();
        assert_eq!(store.revoke_principal(&principal).await.unwrap(), 2);
        assert!(store.find(&first, now + 1).await.unwrap().is_none());
        assert!(store.find(&second, now + 1).await.unwrap().is_none());
        assert!(store.find(&admin, now + 1).await.unwrap().is_some());

        let expired = now + Principal::Admin.lifetime().idle_ms;
        assert_eq!(store.purge_expired(expired).await.unwrap(), 1);
        cleanup(path);
    }

    #[tokio::test]
    async fn caps_sessions_per_principal() {
        let (store, path) = store();
        let now = 1_800_000_000_000;
        let mut tokens = Vec::new();
        for offset in 0..=MAX_SESSIONS_PER_PRINCIPAL {
            let (token, _) = store
                .create(Principal::Admin, [3; 32], now + offset)
                .await
                .unwrap();
            tokens.push(token);
        }
        assert!(store.find(&tokens[0], now + 100).await.unwrap().is_none());
        assert!(store.find(&tokens[1], now + 100).await.unwrap().is_some());
        cleanup(path);
    }
}
