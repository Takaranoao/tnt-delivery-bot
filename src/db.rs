use anyhow::Result;
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS tokens (
  token          TEXT PRIMARY KEY,
  rand           INTEGER NOT NULL,
  created_at     INTEGER NOT NULL,
  last_snapshot  TEXT,
  last_polled_at INTEGER,
  fail_count     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS subscriptions (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id   INTEGER NOT NULL,
  chat_id   INTEGER NOT NULL,
  token     TEXT NOT NULL REFERENCES tokens(token) ON DELETE CASCADE,
  joined_at INTEGER NOT NULL,
  rand      INTEGER NOT NULL,
  UNIQUE(user_id, token)
);
"#;

#[derive(Debug, Clone)]
pub struct TokenRow {
    pub token: String,
    pub rand: i64,
    pub last_snapshot: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UserTokenRow {
    pub token: String,
    pub last_snapshot: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct ExpiredToken {
    pub token: String,
    pub order_id: Option<String>,
    pub subscribers: Vec<(i64, i64)>, // (user_id, chat_id)
}

#[derive(Debug, PartialEq)]
pub enum AddResult {
    Added { snapshot: Option<String> },
    AlreadySubscribed { snapshot: Option<String> },
}

#[derive(Debug, PartialEq)]
pub enum UnsubResult {
    Removed,
    NotSubscribed,
}

enum DbCmd {
    AddSubscription {
        user_id: i64,
        chat_id: i64,
        token: String,
        rand: i64,
        now: i64,
        resp: oneshot::Sender<Result<AddResult>>,
    },
    IsSubscribed {
        user_id: i64,
        token: String,
        resp: oneshot::Sender<Result<bool>>,
    },
    DueTokens {
        resp: oneshot::Sender<Result<Vec<TokenRow>>>,
    },
    RecordPollSuccess {
        token: String,
        snapshot: String,
        now: i64,
        resp: oneshot::Sender<Result<()>>,
    },
    StoreSnapshotIfNull {
        token: String,
        snapshot: String,
        now: i64,
        resp: oneshot::Sender<Result<()>>,
    },
    RecordPollFailure {
        token: String,
        now: i64,
        resp: oneshot::Sender<Result<i64>>,
    },
    Subscribers {
        token: String,
        resp: oneshot::Sender<Result<Vec<(i64, i64)>>>,
    },
    PurgeToken {
        token: String,
        resp: oneshot::Sender<Result<()>>,
    },
    ExpireSweep {
        now: i64,
        ttl_hours: i64,
        resp: oneshot::Sender<Result<Vec<ExpiredToken>>>,
    },
    ListUser {
        user_id: i64,
        resp: oneshot::Sender<Result<Vec<UserTokenRow>>>,
    },
    Unsubscribe {
        user_id: i64,
        token: String,
        resp: oneshot::Sender<Result<UnsubResult>>,
    },
}

#[derive(Clone)]
pub struct DbHandle {
    tx: mpsc::Sender<DbCmd>,
}

macro_rules! call {
    ($self:ident, $variant:ident { $($f:ident),* $(,)? }) => {{
        let (resp, rx) = oneshot::channel();
        $self
            .tx
            .send(DbCmd::$variant { $($f,)* resp })
            .await
            .map_err(|_| anyhow::anyhow!("db actor gone"))?;
        rx.await.map_err(|_| anyhow::anyhow!("db actor dropped reply"))?
    }};
}

impl DbHandle {
    pub async fn add_subscription(
        &self,
        user_id: i64,
        chat_id: i64,
        token: String,
        rand: i64,
        now: i64,
    ) -> Result<AddResult> {
        call!(
            self,
            AddSubscription {
                user_id,
                chat_id,
                token,
                rand,
                now
            }
        )
    }
    pub async fn is_subscribed(&self, user_id: i64, token: String) -> Result<bool> {
        call!(self, IsSubscribed { user_id, token })
    }
    pub async fn due_tokens(&self) -> Result<Vec<TokenRow>> {
        call!(self, DueTokens {})
    }
    pub async fn record_poll_success(
        &self,
        token: String,
        snapshot: String,
        now: i64,
    ) -> Result<()> {
        call!(
            self,
            RecordPollSuccess {
                token,
                snapshot,
                now
            }
        )
    }
    pub async fn store_snapshot_if_null(
        &self,
        token: String,
        snapshot: String,
        now: i64,
    ) -> Result<()> {
        call!(
            self,
            StoreSnapshotIfNull {
                token,
                snapshot,
                now
            }
        )
    }
    pub async fn record_poll_failure(&self, token: String, now: i64) -> Result<i64> {
        call!(self, RecordPollFailure { token, now })
    }
    pub async fn subscribers(&self, token: String) -> Result<Vec<(i64, i64)>> {
        call!(self, Subscribers { token })
    }
    pub async fn purge_token(&self, token: String) -> Result<()> {
        call!(self, PurgeToken { token })
    }
    pub async fn expire_sweep(&self, now: i64, ttl_hours: i64) -> Result<Vec<ExpiredToken>> {
        call!(self, ExpireSweep { now, ttl_hours })
    }
    pub async fn list_user(&self, user_id: i64) -> Result<Vec<UserTokenRow>> {
        call!(self, ListUser { user_id })
    }
    pub async fn unsubscribe(&self, user_id: i64, token: String) -> Result<UnsubResult> {
        call!(self, Unsubscribe { user_id, token })
    }
}

fn order_id_from_snapshot(snap: &Option<String>) -> Option<String> {
    let s = snap.as_ref()?;
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    v.get("order_id")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string())
}

fn subscribers_of(conn: &Connection, token: &str) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare("SELECT user_id, chat_id FROM subscriptions WHERE token = ?1")?;
    let rows = stmt
        .query_map([token], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn handle(conn: &Connection, cmd: DbCmd) {
    match cmd {
        DbCmd::AddSubscription {
            user_id,
            chat_id,
            token,
            rand,
            now,
            resp,
        } => {
            let r = (|| -> Result<AddResult> {
                conn.execute(
                    "INSERT INTO tokens(token, rand, created_at) VALUES(?1, ?2, ?3)
                     ON CONFLICT(token) DO UPDATE SET created_at = excluded.created_at",
                    rusqlite::params![token, rand, now],
                )?;
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO subscriptions(user_id, chat_id, token, joined_at, rand)
                     VALUES(?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![user_id, chat_id, token, now, rand],
                )?;
                let snapshot: Option<String> = conn.query_row(
                    "SELECT last_snapshot FROM tokens WHERE token = ?1",
                    [&token],
                    |r| r.get(0),
                )?;
                Ok(if inserted == 1 {
                    AddResult::Added { snapshot }
                } else {
                    AddResult::AlreadySubscribed { snapshot }
                })
            })();
            let _ = resp.send(r);
        }
        DbCmd::IsSubscribed {
            user_id,
            token,
            resp,
        } => {
            let r = conn
                .query_row(
                    "SELECT 1 FROM subscriptions WHERE user_id = ?1 AND token = ?2",
                    rusqlite::params![user_id, token],
                    |_| Ok(()),
                )
                .map(|_| true)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(false),
                    other => Err(other),
                })
                .map_err(anyhow::Error::from);
            let _ = resp.send(r);
        }
        DbCmd::DueTokens { resp } => {
            let r = (|| -> Result<Vec<TokenRow>> {
                let mut stmt = conn.prepare("SELECT token, rand, last_snapshot FROM tokens")?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(TokenRow {
                            token: r.get(0)?,
                            rand: r.get(1)?,
                            last_snapshot: r.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })();
            let _ = resp.send(r);
        }
        DbCmd::RecordPollSuccess {
            token,
            snapshot,
            now,
            resp,
        } => {
            let r = conn
                .execute(
                    "UPDATE tokens SET last_snapshot = ?2, last_polled_at = ?3, fail_count = 0
                     WHERE token = ?1",
                    rusqlite::params![token, snapshot, now],
                )
                .map(|_| ())
                .map_err(anyhow::Error::from);
            let _ = resp.send(r);
        }
        DbCmd::StoreSnapshotIfNull {
            token,
            snapshot,
            now,
            resp,
        } => {
            let r = conn
                .execute(
                    "UPDATE tokens SET last_snapshot = ?2, last_polled_at = ?3, fail_count = 0
                     WHERE token = ?1 AND last_snapshot IS NULL",
                    rusqlite::params![token, snapshot, now],
                )
                .map(|_| ())
                .map_err(anyhow::Error::from);
            let _ = resp.send(r);
        }
        DbCmd::RecordPollFailure { token, now, resp } => {
            let r = (|| -> Result<i64> {
                conn.execute(
                    "UPDATE tokens SET fail_count = fail_count + 1, last_polled_at = ?2
                     WHERE token = ?1",
                    rusqlite::params![token, now],
                )?;
                let fc: i64 = conn.query_row(
                    "SELECT fail_count FROM tokens WHERE token = ?1",
                    [&token],
                    |r| r.get(0),
                )?;
                Ok(fc)
            })();
            let _ = resp.send(r);
        }
        DbCmd::Subscribers { token, resp } => {
            let _ = resp.send(subscribers_of(conn, &token));
        }
        DbCmd::PurgeToken { token, resp } => {
            let r = (|| -> Result<()> {
                let tx = conn.unchecked_transaction()?;
                // subscriptions rows cascade away (PRAGMA foreign_keys = ON +
                // subscriptions.token REFERENCES tokens(token) ON DELETE CASCADE).
                tx.execute("DELETE FROM tokens WHERE token = ?1", [&token])?;
                tx.commit()?;
                Ok(())
            })();
            let _ = resp.send(r);
        }
        DbCmd::ExpireSweep {
            now,
            ttl_hours,
            resp,
        } => {
            let r = (|| -> Result<Vec<ExpiredToken>> {
                let cutoff = now - ttl_hours * 3600;
                let rows = {
                    let mut stmt = conn.prepare(
                        "SELECT token, last_snapshot FROM tokens WHERE created_at <= ?1",
                    )?;
                    stmt.query_map([cutoff], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let tx = conn.unchecked_transaction()?;
                let mut out = Vec::new();
                for (token, snap) in rows {
                    let subscribers = subscribers_of(&tx, &token)?;
                    let order_id = order_id_from_snapshot(&snap);
                    // subscriptions rows cascade away via ON DELETE CASCADE.
                    tx.execute("DELETE FROM tokens WHERE token = ?1", [&token])?;
                    out.push(ExpiredToken {
                        token,
                        order_id,
                        subscribers,
                    });
                }
                tx.commit()?;
                Ok(out)
            })();
            let _ = resp.send(r);
        }
        DbCmd::ListUser { user_id, resp } => {
            let r = (|| -> Result<Vec<UserTokenRow>> {
                let mut stmt = conn.prepare(
                    "SELECT s.token, t.last_snapshot, t.created_at
                     FROM subscriptions s JOIN tokens t ON t.token = s.token
                     WHERE s.user_id = ?1 ORDER BY s.joined_at",
                )?;
                let rows = stmt
                    .query_map([user_id], |r| {
                        Ok(UserTokenRow {
                            token: r.get(0)?,
                            last_snapshot: r.get(1)?,
                            created_at: r.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })();
            let _ = resp.send(r);
        }
        DbCmd::Unsubscribe {
            user_id,
            token,
            resp,
        } => {
            let r = (|| -> Result<UnsubResult> {
                let removed = conn.execute(
                    "DELETE FROM subscriptions WHERE user_id = ?1 AND token = ?2",
                    rusqlite::params![user_id, token],
                )?;
                if removed == 0 {
                    return Ok(UnsubResult::NotSubscribed);
                }
                let remaining: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM subscriptions WHERE token = ?1",
                    [&token],
                    |r| r.get(0),
                )?;
                if remaining == 0 {
                    conn.execute("DELETE FROM tokens WHERE token = ?1", [&token])?;
                }
                Ok(UnsubResult::Removed)
            })();
            let _ = resp.send(r);
        }
    }
}

/// Open the DB, run migrations, spawn the actor task. Returns a cloneable handle.
pub fn spawn_db_actor(db_path: &str) -> Result<DbHandle> {
    let conn = Connection::open(db_path)?;
    // journal_mode is persisted in the DB header, so this single startup
    // connection sets WAL once for the file. WAL keeps the DB consistent
    // for hot copies / `sqlite3 .backup` and survives crashes better.
    // (execute_batch uses sqlite3_exec, which discards the row WAL returns.)
    conn.execute_batch("PRAGMA journal_mode = WAL;\nPRAGMA foreign_keys = ON;")?;
    conn.execute_batch(MIGRATION)?;
    let (tx, mut rx) = mpsc::channel::<DbCmd>(256);
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            handle(&conn, cmd);
        }
    });
    Ok(DbHandle { tx })
}
