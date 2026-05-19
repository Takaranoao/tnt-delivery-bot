use crate::config::Config;
use crate::db::{AddResult, DbHandle, UnsubResult};
use crate::fetch::ApiFetcher;
use crate::model::{is_unknown_status, render_status, render_summary};
use crate::token_parse::parse_token;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

#[derive(BotCommands, Clone, Debug, PartialEq)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    Start,
    Help,
    List,
    Stop(String),
}

#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub fetcher: Arc<dyn ApiFetcher>,
    pub cfg: Arc<Config>,
}

const HELP: &str = "发我 T&T 配送 token 或追踪链接即可开始追踪，例如:\n\
`3abc128856`\n\
`https://tmstracking.tntsupermarket.us/#/3abc128856`\n\n\
/list 查看在追订单\n\
/stop <token> 停止追踪";

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Decide and perform the join flow for a parsed token. Returns the reply text.
/// Pure of teloxide so it is unit-testable.
pub async fn handle_token(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    token: &str,
) -> String {
    let now = now_ts();
    match state.fetcher.fetch(token).await {
        Ok(resp) if resp.err == 0 && resp.result.is_some() => {
            let result = resp.result.unwrap();
            if is_unknown_status(&result) {
                // Exception: existing subscriber keeps subscription, gets status.
                let subbed = state
                    .db
                    .is_subscribed(user_id, token.to_string())
                    .await
                    .unwrap_or(false);
                if subbed {
                    return format!("你已在追踪，当前状态如下\n{}", render_status(&result));
                }
                return "⚠️ 该订单当前状态为 UNKNOWN(可能 token 无效或订单尚未生成)，未加入追踪;请确认 token 或稍后重试".to_string();
            }
            match state
                .db
                .add_subscription(user_id, chat_id, token.to_string(), rand::random::<i64>(), now)
                .await
            {
                Ok(AddResult::Added { snapshot }) => {
                    if snapshot.is_none() {
                        let snap = serde_json::to_string(&result).unwrap_or_default();
                        let _ = state
                            .db
                            .store_snapshot_if_null(token.to_string(), snap, now)
                            .await;
                    }
                    render_status(&result)
                }
                Ok(AddResult::AlreadySubscribed { .. }) => {
                    format!("你已在追踪，当前状态如下\n{}", render_status(&result))
                }
                Err(e) => format!("内部错误，请稍后重试: {e}"),
            }
        }
        // err != 0 / no result / transport error → still add + count failure.
        _ => {
            let rand = rand::random::<i64>();
            match state
                .db
                .add_subscription(user_id, chat_id, token.to_string(), rand, now)
                .await
            {
                Ok(_) => {
                    let _ = state
                        .db
                        .record_poll_failure(token.to_string(), now)
                        .await;
                    format!(
                        "已加入追踪，但当前查询失败，将自动重试;连续失败 {} 次后停止",
                        state.cfg.max_fetch_failures
                    )
                }
                Err(e) => format!("内部错误，请稍后重试: {e}"),
            }
        }
    }
}

async fn on_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: AppState,
) -> ResponseResult<()> {
    let uid = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let text = match cmd {
        Command::Start | Command::Help => HELP.to_string(),
        Command::List => {
            let rows = state.db.list_user(uid).await.unwrap_or_default();
            if rows.is_empty() {
                "你当前没有在追踪的订单。".to_string()
            } else {
                let now = now_ts();
                let mut s = String::from("你在追踪:\n");
                for r in rows {
                    let summary = r
                        .last_snapshot
                        .as_deref()
                        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                        .map(|v| render_summary(&v))
                        .unwrap_or_else(|| "(尚无状态)".to_string());
                    let remain_h =
                        state.cfg.token_ttl_hours - (now - r.created_at) / 3600;
                    s.push_str(&format!(
                        "• {} · {} · 剩余~{}h\n",
                        r.token,
                        summary,
                        remain_h.max(0)
                    ));
                }
                s.trim_end().to_string()
            }
        }
        Command::Stop(tok) => {
            let tok = tok.trim();
            if tok.is_empty() {
                "用法: /stop <token>".to_string()
            } else {
                match state.db.unsubscribe(uid, tok.to_string()).await {
                    Ok(UnsubResult::Removed) => format!("已停止追踪 {tok}"),
                    Ok(UnsubResult::NotSubscribed) => {
                        format!("你未在追踪 {tok}")
                    }
                    Err(e) => format!("内部错误: {e}"),
                }
            }
        }
    };
    bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

async fn on_text(bot: Bot, msg: Message, state: AppState) -> ResponseResult<()> {
    let uid = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let chat_id = msg.chat.id.0;
    let body = msg.text().unwrap_or_default();
    let reply = match parse_token(body) {
        Some(tok) => handle_token(&state, uid, chat_id, &tok).await,
        None => HELP.to_string(),
    };
    bot.send_message(msg.chat.id, reply).await?;
    Ok(())
}

pub fn build_handler() -> teloxide::dispatching::UpdateHandler<teloxide::RequestError> {
    Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(on_command),
        )
        .branch(dptree::endpoint(on_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::fake::FakeFetcher;

    fn state(fetcher: FakeFetcher) -> AppState {
        // in-memory-ish: use a temp file db.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.sqlite").to_string_lossy().to_string();
        std::mem::forget(dir); // keep file alive for the test process
        AppState {
            db: crate::db::spawn_db_actor(&p).unwrap(),
            fetcher: Arc::new(fetcher),
            cfg: Arc::new(Config {
                bot_token: "x".into(),
                db_path: p,
                tick_seconds: 10,
                poll_period_ticks: 12,
                token_ttl_hours: 24,
                max_fetch_failures: 5,
                api_base: "http://unused".into(),
                http_proxy: None,
            }),
        }
    }

    #[tokio::test]
    async fn unknown_status_is_rejected_when_not_subscribed() {
        let f = FakeFetcher::new();
        f.push_ok("tok", r#"{"err":0,"result":{"order_id":"O","status":"UNKNOWN"}}"#);
        let st = state(f);
        let reply = handle_token(&st, 1, 1, "tok").await;
        assert!(reply.contains("未加入追踪"));
        assert!(!st.db.is_subscribed(1, "tok".into()).await.unwrap());
    }

    #[tokio::test]
    async fn known_status_subscribes_and_stores_first_snapshot() {
        let f = FakeFetcher::new();
        f.push_ok(
            "tok",
            r#"{"err":0,"result":{"order_id":"O","status":"PROCESS"}}"#,
        );
        let st = state(f);
        let reply = handle_token(&st, 1, 1, "tok").await;
        assert!(reply.contains("已加入追踪 O"));
        assert!(st.db.is_subscribed(1, "tok".into()).await.unwrap());
        let due = st.db.due_tokens().await.unwrap();
        assert_eq!(due.len(), 1);
        assert!(due[0].last_snapshot.is_some()); // first snapshot stored
    }

    #[tokio::test]
    async fn fetch_failure_still_subscribes() {
        let f = FakeFetcher::new();
        f.push_err("tok", "down");
        let st = state(f);
        let reply = handle_token(&st, 1, 1, "tok").await;
        assert!(reply.contains("将自动重试"));
        assert!(st.db.is_subscribed(1, "tok".into()).await.unwrap());
    }
}
