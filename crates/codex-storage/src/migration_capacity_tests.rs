use super::*;
use tempfile::tempdir;

#[test]
fn full_v13_migration_rolls_back_and_retries_without_losing_metadata() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("upgrade.db");
    let store = Store::open(&path).unwrap();
    store
        .set_checkpoint("synthetic.jsonl", "rollout", 123, None, None)
        .unwrap();
    store
        .conn
        .execute_batch(
            "DROP INDEX idx_timeline_items_turn;
         DROP TABLE activity_maintenance;
         DELETE FROM schema_migrations WHERE version=14;
         INSERT INTO sessions(session_id,thread_id,source_path,observed_at)
         VALUES('synthetic-session','synthetic-thread','synthetic.jsonl','2026-09-01');",
        )
        .unwrap();
    for ordinal in 0..1000 {
        store
            .conn
            .execute(
                "INSERT INTO timeline_items(event_key,session_id,turn_id,item_type)
             VALUES(?1,'synthetic-session',?2,'tool')",
                params![
                    format!("event-{ordinal}"),
                    format!("synthetic-turn-{ordinal}-{}", "x".repeat(160))
                ],
            )
            .unwrap();
    }
    store.conn.execute_batch("VACUUM").unwrap();
    let pages: u64 = store
        .conn
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    let page_size: u64 = store
        .conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .unwrap();
    drop(store);
    let error = match Store::open_with_capacity(&path, (pages + 2) * page_size) {
        Ok(_) => panic!("fixture must exhaust the migration budget"),
        Err(error) => error,
    };
    assert!(error.chain().any(|cause| cause.downcast_ref::<rusqlite::Error>().is_some_and(|error| matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::DiskFull))));
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("SELECT max(version) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        raw.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='activity_maintenance'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(raw);
    for _ in 0..2 {
        let restored = Store::open(&path).unwrap();
        assert_eq!(restored.checkpoint("synthetic.jsonl").unwrap(), 123);
        assert_eq!(
            restored
                .conn
                .query_row("SELECT count(*) FROM timeline_items", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1000
        );
        assert_eq!(
            restored
                .conn
                .query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            14
        );
        assert_eq!(
            restored
                .conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='idx_timeline_items_turn'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}

#[test]
fn byte_budget_respects_page_size_and_does_not_grow_on_reopen() {
    let dir = tempdir().unwrap();
    for page_size in [1024_u64, 4096, 8192] {
        let path = dir.path().join(format!("pages-{page_size}.db"));
        let raw = Connection::open(&path).unwrap();
        raw.pragma_update(None, "page_size", page_size).unwrap();
        raw.execute_batch("VACUUM").unwrap();
        drop(raw);
        for _ in 0..2 {
            let store = Store::open(&path).unwrap();
            let actual_size: u64 = store
                .conn
                .pragma_query_value(None, "page_size", |r| r.get(0))
                .unwrap();
            let limit: u64 = store
                .conn
                .pragma_query_value(None, "max_page_count", |r| r.get(0))
                .unwrap();
            assert_eq!(actual_size, page_size);
            // Lock the released byte policy independently of the implementation
            // constant so restoring the old 512 MiB value fails this regression.
            assert_eq!(limit * actual_size, 1_073_741_824);
        }
    }
}

#[test]
fn exhausted_budget_rejects_write_without_partial_rows() {
    let dir = tempdir().unwrap();
    let store = Store::open_with_capacity(&dir.path().join("bounded.db"), 1024 * 1024).unwrap();
    store.conn.execute_batch("CREATE TABLE artificial_payload(id INTEGER PRIMARY KEY, data BLOB); INSERT INTO artificial_payload VALUES(1, zeroblob(16));").unwrap();
    let tx = store.conn.unchecked_transaction().unwrap();
    let error = tx
        .execute(
            "INSERT INTO artificial_payload VALUES(2,zeroblob(2097152))",
            [],
        )
        .unwrap_err();
    assert!(
        matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::DiskFull)
    );
    drop(tx);
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM artificial_payload", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
