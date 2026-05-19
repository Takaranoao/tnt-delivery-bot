use std::collections::HashSet;
use std::sync::Arc;
use tnt_delivery_bot::bot::{handle_token, AppState};
use tnt_delivery_bot::config::Config;
use tnt_delivery_bot::db::{spawn_db_actor, UnsubResult};
use tnt_delivery_bot::fetch::fake::FakeFetcher;
use tnt_delivery_bot::notify::fake::FakeNotifier;
use tnt_delivery_bot::schedule::slot;
use tnt_delivery_bot::tick::run_tick_round;

fn cfg(path: &str) -> Config {
    Config {
        bot_token: "x".into(),
        db_path: path.into(),
        tick_seconds: 10,
        poll_period_ticks: 12,
        token_ttl_hours: 24,
        max_fetch_failures: 3,
        api_base: "http://unused".into(),
        http_proxy: None,
    }
}

fn tmp() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("e2e.sqlite").to_string_lossy().to_string();
    (d, p)
}

/// Full lifecycle through the real user entry point `handle_token`:
/// user1 joins -> user2 joins same token (shared) -> a due tick with a
/// status change fans out to BOTH subscribers -> user1 /stop -> the next
/// change only notifies the remaining subscriber.
#[tokio::test]
async fn join_poll_diff_unsubscribe_end_to_end() {
    let (_d, path) = tmp();
    let c = cfg(&path);
    let db = spawn_db_actor(&path).unwrap();
    let fetcher = Arc::new(FakeFetcher::new());
    let state = AppState {
        db: db.clone(),
        fetcher: fetcher.clone(),
        cfg: Arc::new(c.clone()),
    };

    // User 1 joins via a real token message; first fetch = PROCESS.
    fetcher.push_ok(
        "TK",
        r#"{"err":0,"result":{"order_id":"OID9","status":"PROCESS"}}"#,
    );
    let reply = handle_token(&state, 1, 1001, "TK").await;
    assert!(reply.contains("已加入追踪 OID9"), "join receipt was: {reply}");
    assert!(db.is_subscribed(1, "TK".into()).await.unwrap());

    // User 2 joins the same (shared) token; fetch still PROCESS.
    fetcher.push_ok(
        "TK",
        r#"{"err":0,"result":{"order_id":"OID9","status":"PROCESS"}}"#,
    );
    let reply2 = handle_token(&state, 2, 1002, "TK").await;
    assert!(
        reply2.contains("已加入追踪 OID9") || reply2.contains("你已在追踪"),
        "2nd subscriber reply was: {reply2}"
    );
    assert!(db.is_subscribed(2, "TK".into()).await.unwrap());

    // One shared token row; first snapshot stored at join. Read its rand
    // (chosen randomly inside handle_token) to compute a guaranteed-due tick.
    let rows = db.due_tokens().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].last_snapshot.is_some(), "first snapshot stored at join");
    let rand = rows[0].rand;
    let t = slot("TK", rand, c.poll_period_ticks);

    // Due tick, status PROCESS -> DELIVERED: both subscribers notified.
    let notifier = FakeNotifier::new();
    fetcher.push_ok(
        "TK",
        r#"{"err":0,"result":{"order_id":"OID9","status":"DELIVERED"}}"#,
    );
    run_tick_round(t, &c, &db, fetcher.as_ref(), &notifier)
        .await
        .unwrap();
    {
        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 2, "both subscribers notified");
        let chats: HashSet<i64> = sent.iter().map(|(ch, _)| *ch).collect();
        assert_eq!(chats, HashSet::from([1001, 1002]));
        assert!(sent.iter().all(|(_, m)| m.contains("状态: PROCESS → DELIVERED")));
    }

    // User 1 stops; token remains tracked for user 2.
    assert_eq!(
        db.unsubscribe(1, "TK".into()).await.unwrap(),
        UnsubResult::Removed
    );
    assert!(!db.is_subscribed(1, "TK".into()).await.unwrap());
    assert!(db.is_subscribed(2, "TK".into()).await.unwrap());

    // Next due tick, DELIVERED -> COMPLETED: user 2 gets the change push AND
    // the "completed, tracking stopped" notice; then the token is purged.
    let notifier2 = FakeNotifier::new();
    fetcher.push_ok(
        "TK",
        r#"{"err":0,"result":{"order_id":"OID9","status":"COMPLETED"}}"#,
    );
    run_tick_round(t + c.poll_period_ticks, &c, &db, fetcher.as_ref(), &notifier2)
        .await
        .unwrap();
    {
        let sent = notifier2.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|(ch, _)| *ch == 1002));
        assert!(sent[0].1.contains("状态: DELIVERED → COMPLETED"));
        assert!(sent[1].1.contains("已完成") && sent[1].1.contains("停止追踪"));
    }
    assert!(
        db.due_tokens().await.unwrap().is_empty(),
        "token purged after COMPLETED"
    );
    assert!(!db.is_subscribed(2, "TK".into()).await.unwrap());
}

/// Join while the API is failing still subscribes; repeated failures across
/// the join + a due tick reach max_fetch_failures -> purge + notify.
#[tokio::test]
async fn join_then_repeated_failure_purges_end_to_end() {
    let (_d, path) = tmp();
    let mut c = cfg(&path);
    c.max_fetch_failures = 2;
    let db = spawn_db_actor(&path).unwrap();
    let fetcher = Arc::new(FakeFetcher::new());
    let state = AppState {
        db: db.clone(),
        fetcher: fetcher.clone(),
        cfg: Arc::new(c.clone()),
    };

    // Join while the API is down: still subscribes, records failure #1.
    fetcher.push_err("BAD", "boom");
    let reply = handle_token(&state, 7, 7007, "BAD").await;
    assert!(reply.contains("将自动重试"), "fail-join reply was: {reply}");
    assert!(db.is_subscribed(7, "BAD".into()).await.unwrap());

    let rows = db.due_tokens().await.unwrap();
    let rand = rows.iter().find(|r| r.token == "BAD").unwrap().rand;
    let t = slot("BAD", rand, c.poll_period_ticks);

    // One more failing due-tick reaches fail_count == max (2): purge + notify.
    let notifier = FakeNotifier::new();
    fetcher.push_err("BAD", "boom");
    run_tick_round(t, &c, &db, fetcher.as_ref(), &notifier)
        .await
        .unwrap();
    {
        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, 7007);
        assert!(sent[0].1.contains("持续查询失败"));
    }
    assert!(
        db.due_tokens().await.unwrap().is_empty(),
        "token purged after reaching max_fetch_failures"
    );
}
