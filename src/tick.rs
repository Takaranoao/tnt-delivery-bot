use crate::config::Config;
use crate::db::DbHandle;
use crate::fetch::ApiFetcher;
use crate::model::{diff_snapshots, is_completed_status, render_changes};
use crate::notify::{Notifier, NotifyError};
use crate::schedule::is_due;
use anyhow::Result;
use serde_json::Value;

/// Send a notification; on Forbidden, drop that user's subscription for `token`.
async fn notify_or_cleanup(
    notifier: &dyn Notifier,
    db: &DbHandle,
    token: &str,
    user_id: i64,
    chat_id: i64,
    text: &str,
) {
    if let Err(e) = notifier.send(chat_id, text).await {
        if e.is_forbidden() {
            let _ = db.unsubscribe(user_id, token.to_string()).await;
        } else if let NotifyError::Other(m) = e {
            log::warn!("notify failed for chat {chat_id}: {m}");
        }
    }
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// One scheduler iteration. `tick` is the monotonic counter (already incremented).
pub async fn run_tick_round(
    tick: u64,
    cfg: &Config,
    db: &DbHandle,
    fetcher: &dyn ApiFetcher,
    notifier: &dyn Notifier,
) -> Result<()> {
    let now = now_ts();

    // 1. Expiry sweep (token already deleted inside the actor).
    for exp in db.expire_sweep(now, cfg.token_ttl_hours).await? {
        let id = exp.order_id.clone().unwrap_or_else(|| exp.token.clone());
        let msg = format!("⏰ 已追踪满 {}h，停止追踪 {}", cfg.token_ttl_hours, id);
        for (uid, chat) in exp.subscribers {
            // token gone already; just try to send, ignore errors.
            let _ = notifier.send(chat, &msg).await;
            let _ = uid;
        }
    }

    // 2. Due tokens this tick.
    let tokens = db.due_tokens().await?;
    for row in tokens {
        if !is_due(tick, &row.token, row.rand, cfg.poll_period_ticks) {
            continue;
        }
        match fetcher.fetch(&row.token).await {
            Ok(resp) if resp.err == 0 && resp.result.is_some() => {
                let result: Value = resp.result.unwrap();
                let result_str = serde_json::to_string(&result)?;
                match &row.last_snapshot {
                    None => {
                        db.record_poll_success(row.token.clone(), result_str, now)
                            .await?;
                    }
                    Some(prev_str) => {
                        let prev: Value = serde_json::from_str(prev_str)
                            .unwrap_or(Value::Object(Default::default()));
                        let changes = diff_snapshots(&prev, &result);
                        if !changes.is_empty() {
                            let order_id = result
                                .get("order_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(&row.token)
                                .to_string();
                            let msg = render_changes(&order_id, &changes);
                            for (uid, chat) in
                                db.subscribers(row.token.clone()).await?
                            {
                                notify_or_cleanup(
                                    notifier, db, &row.token, uid, chat, &msg,
                                )
                                .await;
                            }
                        }
                        db.record_poll_success(row.token.clone(), result_str, now)
                            .await?;
                    }
                }
                // Order completed → push already done above; now notify all
                // subscribers it's finished and stop tracking (purge token).
                if is_completed_status(&result) {
                    let order_id = result
                        .get("order_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&row.token)
                        .to_string();
                    let msg = format!("✅ 订单 {order_id} 已完成，停止追踪");
                    for (uid, chat) in db.subscribers(row.token.clone()).await? {
                        let _ = notifier.send(chat, &msg).await;
                        let _ = uid;
                    }
                    db.purge_token(row.token.clone()).await?;
                }
            }
            // err != 0, or result missing, or transport error → a failure.
            _ => {
                let fc = db.record_poll_failure(row.token.clone(), now).await?;
                if fc >= cfg.max_fetch_failures {
                    let msg =
                        format!("❌ {} 持续查询失败，已停止追踪", row.token);
                    for (uid, chat) in db.subscribers(row.token.clone()).await? {
                        let _ = notifier.send(chat, &msg).await;
                        let _ = uid;
                    }
                    db.purge_token(row.token.clone()).await?;
                }
            }
        }
    }
    Ok(())
}
