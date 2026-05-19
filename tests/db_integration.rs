use tnt_delivery_bot::db::{AddResult, UnsubResult, spawn_db_actor};

fn tmp_db() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.sqlite").to_string_lossy().to_string();
    (dir, path)
}

#[tokio::test]
async fn add_dedup_snapshot_failure_purge() {
    let (_d, path) = tmp_db();
    let db = spawn_db_actor(&path).unwrap();

    // First add → Added, snapshot None.
    let r = db
        .add_subscription(1, 1, "tok".into(), 42, 1000)
        .await
        .unwrap();
    assert_eq!(r, AddResult::Added { snapshot: None });

    // Same user same token → AlreadySubscribed.
    let r = db
        .add_subscription(1, 1, "tok".into(), 42, 2000)
        .await
        .unwrap();
    assert_eq!(r, AddResult::AlreadySubscribed { snapshot: None });
    assert!(db.is_subscribed(1, "tok".into()).await.unwrap());

    // Second user → Added (token shared).
    let r = db
        .add_subscription(2, 2, "tok".into(), 42, 2000)
        .await
        .unwrap();
    assert_eq!(r, AddResult::Added { snapshot: None });

    // Store first snapshot only if null.
    db.store_snapshot_if_null("tok".into(), "{\"a\":1}".into(), 3000)
        .await
        .unwrap();
    db.store_snapshot_if_null("tok".into(), "{\"a\":2}".into(), 3100)
        .await
        .unwrap();
    let due = db.due_tokens().await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].last_snapshot.as_deref(), Some("{\"a\":1}"));

    // Subscribers fan-out list.
    let mut subs = db.subscribers("tok".into()).await.unwrap();
    subs.sort();
    assert_eq!(subs, vec![(1, 1), (2, 2)]);

    // Failure count increments then purge.
    assert_eq!(db.record_poll_failure("tok".into(), 4000).await.unwrap(), 1);
    assert_eq!(db.record_poll_failure("tok".into(), 4100).await.unwrap(), 2);
    db.record_poll_success("tok".into(), "{\"a\":9}".into(), 4200)
        .await
        .unwrap();
    assert_eq!(db.record_poll_failure("tok".into(), 4300).await.unwrap(), 1); // reset by success

    db.purge_token("tok".into()).await.unwrap();
    assert!(db.due_tokens().await.unwrap().is_empty());
}

#[tokio::test]
async fn expiry_and_unsubscribe() {
    let (_d, path) = tmp_db();
    let db = spawn_db_actor(&path).unwrap();

    db.add_subscription(1, 1, "old".into(), 1, 0).await.unwrap();
    db.record_poll_success("old".into(), "{\"order_id\":\"OID\"}".into(), 10)
        .await
        .unwrap();

    // Not expired yet at now=3600 with ttl=24h.
    assert!(db.expire_sweep(3600, 24).await.unwrap().is_empty());
    // Expired at now = created_at(0) + 24h.
    let exp = db.expire_sweep(24 * 3600, 24).await.unwrap();
    assert_eq!(exp.len(), 1);
    assert_eq!(exp[0].token, "old");
    assert_eq!(exp[0].order_id.as_deref(), Some("OID"));
    assert_eq!(exp[0].subscribers, vec![(1, 1)]);
    assert!(db.due_tokens().await.unwrap().is_empty());

    // Unsubscribe: last subscriber removal purges the token.
    db.add_subscription(7, 7, "z".into(), 1, 0).await.unwrap();
    assert_eq!(
        db.unsubscribe(7, "z".into()).await.unwrap(),
        UnsubResult::Removed
    );
    assert_eq!(
        db.unsubscribe(7, "z".into()).await.unwrap(),
        UnsubResult::NotSubscribed
    );
    assert!(db.due_tokens().await.unwrap().is_empty());
}
