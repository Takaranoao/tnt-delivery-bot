use tnt_delivery_bot::config::Config;
use tnt_delivery_bot::db::spawn_db_actor;
use tnt_delivery_bot::fetch::fake::FakeFetcher;
use tnt_delivery_bot::notify::fake::FakeNotifier;
use tnt_delivery_bot::schedule::slot;
use tnt_delivery_bot::tick::run_tick_round;

fn cfg() -> Config {
    Config {
        bot_token: "x".into(),
        db_path: ":memory:".into(),
        tick_seconds: 10,
        poll_period_ticks: 12,
        token_ttl_hours: 24,
        max_fetch_failures: 2,
        api_base: "http://unused".into(),
        http_proxy: None,
    }
}

fn tmp_path() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.sqlite").to_string_lossy().to_string();
    (dir, p)
}

/// Pick a tick value on which `token` is due.
fn due_tick(token: &str, rand: i64, n: u64) -> u64 {
    slot(token, rand, n)
}

/// Real wall-clock unix seconds, so a freshly-added token is within its TTL
/// (the expiry sweep in run_tick_round uses real `now`).
fn recent() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[tokio::test]
async fn first_snapshot_then_change_is_pushed() {
    let (_d, path) = tmp_path();
    let db = spawn_db_actor(&path).unwrap();
    let c = cfg();
    let rand = 7;
    db.add_subscription(1, 1, "tok".into(), rand, recent())
        .await
        .unwrap();

    let fetcher = FakeFetcher::new();
    let notifier = FakeNotifier::new();
    let t = due_tick("tok", rand, c.poll_period_ticks);

    // First poll: stores snapshot, no notification.
    fetcher.push_ok(
        "tok",
        r#"{"err":0,"result":{"order_id":"OID","status":"PROCESS"}}"#,
    );
    run_tick_round(t, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    assert!(notifier.sent.lock().unwrap().is_empty());

    // Second poll: status changes → push to subscriber.
    fetcher.push_ok(
        "tok",
        r#"{"err":0,"result":{"order_id":"OID","status":"DELIVERED"}}"#,
    );
    run_tick_round(t + c.poll_period_ticks, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    let sent = notifier.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, 1);
    assert!(sent[0].1.contains("状态: PROCESS → DELIVERED"));
}

#[tokio::test]
async fn repeated_failures_purge_and_notify() {
    let (_d, path) = tmp_path();
    let db = spawn_db_actor(&path).unwrap();
    let c = cfg(); // max_fetch_failures = 2
    let rand = 3;
    db.add_subscription(5, 5, "bad".into(), rand, recent())
        .await
        .unwrap();
    let fetcher = FakeFetcher::new();
    let notifier = FakeNotifier::new();
    let t = due_tick("bad", rand, c.poll_period_ticks);

    fetcher.push_err("bad", "boom");
    run_tick_round(t, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    assert!(notifier.sent.lock().unwrap().is_empty()); // 1st failure: silent

    fetcher.push_err("bad", "boom");
    run_tick_round(t + c.poll_period_ticks, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    {
        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.contains("持续查询失败"));
    }
    assert!(db.due_tokens().await.unwrap().is_empty()); // purged
}

#[tokio::test]
async fn expiry_notifies_all_subscribers() {
    let (_d, path) = tmp_path();
    let db = spawn_db_actor(&path).unwrap();
    let mut c = cfg();
    c.token_ttl_hours = 24;
    db.add_subscription(1, 1, "old".into(), 1, 0).await.unwrap();
    db.add_subscription(2, 2, "old".into(), 1, 0).await.unwrap();
    db.record_poll_success("old".into(), r#"{"order_id":"OID"}"#.into(), 1)
        .await
        .unwrap();
    let fetcher = FakeFetcher::new();
    let notifier = FakeNotifier::new();
    // created_at=0; at now well past 24h everything expires regardless of tick.
    // Use a tick where "old" is not due so only expiry path runs.
    run_tick_round(999_999, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    let sent = notifier.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|(_, m)| m.contains("停止追踪")));
}

#[tokio::test]
async fn completed_status_notifies_and_purges() {
    let (_d, path) = tmp_path();
    let db = spawn_db_actor(&path).unwrap();
    let c = cfg();
    let rand = 9;
    db.add_subscription(1, 1, "done".into(), rand, recent())
        .await
        .unwrap();
    let fetcher = FakeFetcher::new();
    let notifier = FakeNotifier::new();
    let t = due_tick("done", rand, c.poll_period_ticks);

    // First poll stores the snapshot (PROCESS); no notification.
    fetcher.push_ok(
        "done",
        r#"{"err":0,"result":{"order_id":"OID","status":"PROCESS"}}"#,
    );
    run_tick_round(t, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    assert!(notifier.sent.lock().unwrap().is_empty());

    // Next poll: status -> COMPLETED. Expect the change push AND a
    // "completed, tracking stopped" notice, then the token purged.
    fetcher.push_ok(
        "done",
        r#"{"err":0,"result":{"order_id":"OID","status":"COMPLETED"}}"#,
    );
    run_tick_round(t + c.poll_period_ticks, &c, &db, &fetcher, &notifier)
        .await
        .unwrap();
    {
        let sent = notifier.sent.lock().unwrap();
        assert_eq!(sent.len(), 2, "change push + completed notice");
        assert!(sent[0].1.contains("状态: PROCESS → COMPLETED"));
        assert!(sent[1].1.contains("已完成") && sent[1].1.contains("停止追踪"));
        assert!(sent.iter().all(|(ch, _)| *ch == 1));
    }
    assert!(
        db.due_tokens().await.unwrap().is_empty(),
        "token purged after COMPLETED"
    );
}
