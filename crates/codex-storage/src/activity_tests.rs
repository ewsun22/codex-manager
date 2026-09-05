//! Artificial metadata fixtures only; never opens a user's Codex home.

use super::*;
use codex_core::{RolloutNormalizer, TimelineItem};
use serde_json::{Value, json};
use tempfile::tempdir;

fn event(kind: &str, ordinal: usize, payload: Value) -> String {
    json!({
        "type": kind,
        "ordinal": ordinal,
        "timestamp": "2026-08-27T00:00:00Z",
        "payload": payload,
    })
    .to_string()
}

fn token_event(ordinal: usize, total: i64) -> String {
    event(
        "event_msg",
        ordinal,
        json!({"type":"token_count","info":{"total_token_usage":{
            "input_tokens":total,"cached_input_tokens":0,"cache_write_input_tokens":0,
            "output_tokens":0,"reasoning_output_tokens":0,"total_tokens":total,
        }}}),
    )
}

fn context_lines() -> Vec<String> {
    vec![
        event(
            "session_meta",
            0,
            json!({"session_id":"fixture-session","model_provider":"openai"}),
        ),
        event(
            "event_msg",
            1,
            json!({"type":"task_started","turn_id":"fixture-turn"}),
        ),
        token_event(2, 10),
        event(
            "turn_context",
            3,
            json!({"turn_id":"fixture-turn","model":"model-a","effort":"low"}),
        ),
        event(
            "turn_context",
            4,
            json!({"turn_id":"fixture-turn","model":"model-b","effort":"high"}),
        ),
        token_event(5, 20),
        event(
            "turn_context",
            6,
            json!({"turn_id":"fixture-turn","model":"model-c","effort":"medium"}),
        ),
        event(
            "event_msg",
            7,
            json!({"type":"task_complete","turn_id":"fixture-turn"}),
        ),
    ]
}

fn ingest_batches(store: &mut Store, lines: &[String], boundaries: &[usize]) {
    let mut normalizer = RolloutNormalizer::default();
    for (index, line) in lines.iter().enumerate() {
        normalizer.parse_line(line, Some(index as u64));
        if boundaries.contains(&(index + 1)) || index + 1 == lines.len() {
            let resume = normalizer.resume_state();
            store
                .commit_ingest(CommitIngest {
                    path: "artificial-rollout",
                    source_kind: "rollout",
                    next_offset: (index + 1) as i64,
                    file_identity: Some("artificial-file"),
                    snapshot: &normalizer.incremental_snapshot(),
                    resume_state: &resume,
                    rebuild: false,
                    unparsed_events: 0,
                })
                .unwrap();
            normalizer = RolloutNormalizer::from_resume_state(resume);
        }
    }
}

#[test]
fn late_context_has_identical_cost_and_activity_at_every_batch_boundary() {
    let dir = tempdir().unwrap();
    let lines = context_lines();
    let filter = ActivityFilter {
        view: Some("modelCalls".into()),
        ..Default::default()
    };
    let mut whole = Store::open(&dir.path().join("whole.db")).unwrap();
    ingest_batches(&mut whole, &lines, &[]);
    let expected_cost = serde_json::to_value(whole.cost_inputs().unwrap()).unwrap();
    let expected_rows = serde_json::to_value(whole.page_activity(&filter).unwrap()).unwrap();
    assert_eq!(expected_cost[0]["model"], "model-c");
    assert_eq!(expected_cost[1]["model"], "model-b");

    let mut boundaries = (1..lines.len())
        .map(|split| vec![split])
        .collect::<Vec<_>>();
    boundaries.push((1..lines.len()).collect());
    boundaries.push(vec![3, 5]); // Both late contexts arrive after the first durable call.
    for (case, splits) in boundaries.iter().enumerate() {
        let mut store = Store::open(&dir.path().join(format!("split-{case}.db"))).unwrap();
        ingest_batches(&mut store, &lines, splits);
        assert_eq!(
            serde_json::to_value(store.cost_inputs().unwrap()).unwrap(),
            expected_cost,
            "{splits:?}"
        );
        assert_eq!(
            serde_json::to_value(store.page_activity(&filter).unwrap()).unwrap(),
            expected_rows,
            "{splits:?}"
        );
    }

    let mut parser = RolloutNormalizer::default();
    for line in &lines {
        parser.parse_line(line, None);
    }
    let calls = parser.snapshot().model_calls;
    assert_eq!(calls[0].model, None);
    assert_eq!(calls[0].effort, None);
    assert_eq!(calls[1].model.as_deref(), Some("model-b"));
    assert_eq!(calls[1].effort.as_deref(), Some("high"));
}

#[test]
fn incomplete_turn_keeps_valid_calls_without_claiming_complete_usage() {
    let dir = tempdir().unwrap();
    let mut lines = context_lines();
    lines.truncate(3);
    lines.push(token_event(3, TokenUsage::MAX_FIELD_VALUE + 1));
    lines.push(token_event(4, 20));
    for split in 0..=lines.len() {
        let mut store = Store::open(&dir.path().join(format!("partial-{split}.db"))).unwrap();
        ingest_batches(&mut store, &lines, &[split]);
        let turns = store.page_activity(&ActivityFilter::default()).unwrap();
        assert_eq!(turns.total, 1);
        let row = &turns.items[0];
        assert!(row.has_model_call);
        assert_eq!(row.usage.total_tokens, 10); // Valid observed subset remains available internally.
        assert!(
            serde_json::to_value(&row.usage_available)
                .unwrap()
                .as_object()
                .unwrap()
                .values()
                .all(|value| value == &Value::Bool(false))
        );
        let calls = store
            .page_activity(&ActivityFilter {
                view: Some("modelCalls".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(calls.total, 1);
        assert!(calls.items[0].usage_available.total_tokens);
        let costs = store.cost_inputs().unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].input_tokens, 10);
    }
}

fn completed_snapshot(turn_id: &str, completed_at: &str) -> Snapshot {
    let mut value = super::tests::snapshot(
        "retention-session",
        turn_id,
        "/artificial/project",
        "completed",
        Some((turn_id, 10, 10)),
    );
    value.session.as_mut().unwrap().started_at = Some(completed_at.into());
    value.turns[0].started_at = Some(completed_at.into());
    value.turns[0].completed_at = Some(completed_at.into());
    value.model_calls[0].occurred_at = Some(completed_at.into());
    value
}

fn commit_snapshot(store: &mut Store, snapshot: &Snapshot, offset: i64, rebuild: bool) {
    let resume = ResumeState {
        session: snapshot.session.clone(),
        current_turn: snapshot.turns.last().cloned(),
        last_cumulative_usage: snapshot
            .model_calls
            .last()
            .map(|call| call.cumulative_usage.clone()),
        unavailable_turn_id: None,
    };
    store
        .commit_ingest(CommitIngest {
            path: "artificial-retention-rollout",
            source_kind: "rollout",
            next_offset: offset,
            file_identity: Some("artificial-retention-file"),
            snapshot,
            resume_state: &resume,
            rebuild,
            unparsed_events: 0,
        })
        .unwrap();
}

#[test]
fn retention_survives_initial_ingest_restart_and_rebuild_without_losing_checkpoint() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("retention.db");
    let mut store = Store::open(&db).unwrap();
    let initial_revision = store.activity_revision().unwrap();
    assert_eq!(store.prune_retention("2026-08-28T00:00:00Z").unwrap(), 0);
    assert_eq!(store.activity_revision().unwrap(), initial_revision);
    let old = completed_snapshot("old", "2020-01-01T00:00:00Z");
    commit_snapshot(&mut store, &old, 100, false);
    assert_eq!(
        store
            .page_activity(&ActivityFilter::default())
            .unwrap()
            .total,
        0
    );
    assert!(store.cost_inputs().unwrap().is_empty());
    assert_eq!(
        store.checkpoint("artificial-retention-rollout").unwrap(),
        100
    );
    assert!(
        store
            .ingest_checkpoint("artificial-retention-rollout")
            .unwrap()
            .resume_state
            .unwrap()
            .last_cumulative_usage
            .is_some()
    );
    drop(store);

    let mut store = Store::open(&db).unwrap();
    commit_snapshot(&mut store, &old, 200, true);
    assert_eq!(
        store
            .page_activity(&ActivityFilter::default())
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store.checkpoint("artificial-retention-rollout").unwrap(),
        200
    );
    let recent = completed_snapshot("new", "2026-08-29T00:00:00Z");
    commit_snapshot(&mut store, &recent, 300, false);
    assert_eq!(
        store
            .page_activity(&ActivityFilter::default())
            .unwrap()
            .total,
        1
    );
    let revision = store.activity_revision().unwrap();
    assert_eq!(store.prune_retention("2026-08-28T01:00:00Z").unwrap(), 0);
    assert_eq!(store.activity_revision().unwrap(), revision);
    assert_eq!(store.prune_retention("2026-08-30T00:00:00Z").unwrap(), 1);
    assert_ne!(store.activity_revision().unwrap(), revision);
    assert_eq!(
        store.checkpoint("artificial-retention-rollout").unwrap(),
        300
    );
    let revision = store.activity_revision().unwrap();
    assert_eq!(store.prune_retention("2026-08-30T00:00:00Z").unwrap(), 0);
    assert_eq!(store.activity_revision().unwrap(), revision);
}

fn timeline(turn_id: Option<&str>, key: &str) -> TimelineItem {
    TimelineItem {
        event_key: key.into(),
        session_id: "retention-session".into(),
        turn_id: turn_id.map(str::to_owned),
        ordinal: None,
        occurred_at: Some("2026-08-20T00:00:00Z".into()),
        item_type: "message".into(),
        role: Some("assistant".into()),
        phase: None,
        tool_name: None,
        content_utf8_bytes: Some(1),
    }
}

#[test]
fn retention_cleans_associated_metadata_and_preserves_unverifiable_tasks() {
    let dir = tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("mixed.db")).unwrap();
    let mut combined = completed_snapshot("old", "2026-08-27T00:00:00Z");
    let cases = [
        ("offset-old", "completed", Some("2026-08-28T00:30:00+01:00")),
        ("offset-new", "completed", Some("2026-08-27T23:30:00-01:00")),
        ("fallback-start", "aborted", None),
        ("running", "running", None),
        ("unknown", "unknown", None),
        ("undated", "completed", None),
        ("invalid-date", "completed", Some("not-a-date")),
        ("numeric-date", "completed", Some("123")),
        ("date-only", "completed", Some("2026-08-20")),
        (
            "invalid-calendar",
            "completed",
            Some("2026-02-30T00:00:00Z"),
        ),
        (
            "precise-old",
            "completed",
            Some("2026-08-27T23:59:59.999999Z"),
        ),
        (
            "precise-new",
            "completed",
            Some("2026-08-28T00:00:00.000001Z"),
        ),
        ("equal-cutoff", "completed", Some("2026-08-28T00:00:00Z")),
    ];
    for (id, status, completed) in cases {
        let mut task = completed_snapshot(id, "2026-08-29T00:00:00Z");
        task.turns[0].status = status.into();
        task.turns[0].completed_at = completed.map(str::to_owned);
        task.turns[0].started_at = Some("2026-08-27T00:00:00Z".into());
        if id == "undated" {
            task.turns[0].started_at = None;
        }
        combined.turns.extend(task.turns);
        combined.model_calls.extend(task.model_calls);
    }
    combined.timeline = vec![
        timeline(Some("old"), "old-item"),
        timeline(Some("offset-new"), "new-item"),
        timeline(None, "unassigned-old-item"),
    ];
    commit_snapshot(&mut store, &combined, 100, false);
    let revision = store.activity_revision().unwrap();
    assert_eq!(store.prune_retention("2026-08-28T00:00:00Z").unwrap(), 4);
    assert_ne!(store.activity_revision().unwrap(), revision);
    let rows = store.page_activity(&ActivityFilter::default()).unwrap();
    let ids = rows
        .items
        .iter()
        .map(|row| row.turn_id.as_deref().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ids,
        BTreeSet::from([
            "offset-new",
            "running",
            "unknown",
            "undated",
            "invalid-date",
            "numeric-date",
            "date-only",
            "invalid-calendar",
            "precise-new",
            "equal-cutoff",
        ])
    );
    let (timeline_count, remaining_turn): (i64, String) = store
        .conn
        .query_row("SELECT count(*),turn_id FROM timeline_items", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(timeline_count, 1);
    assert_eq!(remaining_turn, "offset-new");
    assert_eq!(store.cost_inputs().unwrap().len(), 10);
}

#[test]
fn retention_rolls_back_cutoff_and_metadata_when_a_delete_fails() {
    let dir = tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("rollback.db")).unwrap();
    let mut old = completed_snapshot("old", "2020-01-01T00:00:00Z");
    old.timeline.push(timeline(Some("old"), "old-item"));
    commit_snapshot(&mut store, &old, 100, false);
    let revision = store.activity_revision().unwrap();
    store.conn.execute_batch("CREATE TEMP TRIGGER fail_retention BEFORE DELETE ON turns BEGIN SELECT RAISE(ABORT,'artificial delete failure'); END;").unwrap();
    assert!(store.prune_retention("2026-08-28T00:00:00Z").is_err());
    assert_eq!(store.activity_revision().unwrap(), revision);
    assert_eq!(
        store
            .page_activity(&ActivityFilter::default())
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM timeline_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .conn
            .query_row(
                "SELECT retention_cutoff FROM activity_maintenance",
                [],
                |row| row.get::<_, Option<String>>(0)
            )
            .unwrap(),
        None
    );
    assert!(store.prune_retention("invalid-cutoff").is_err());
    assert_eq!(
        store.checkpoint("artificial-retention-rollout").unwrap(),
        100
    );
}

#[test]
fn otel_retention_revision_changes_when_the_largest_rowid_is_unchanged() {
    let dir = tempdir().unwrap();
    let store = Store::open(&dir.path().join("otel.db")).unwrap();
    store
        .insert_otel_metadata(super::tests::otel_event("old", None))
        .unwrap();
    store
        .insert_otel_metadata(super::tests::otel_event("new", None))
        .unwrap();
    store.conn.execute("UPDATE otel_events SET received_at=CASE WHEN id='old' THEN '2026-08-28T00:30:00+01:00' ELSE '2026-08-27T23:30:00-01:00' END", []).unwrap();
    let revision = store.activity_revision().unwrap();
    assert_eq!(store.prune_retention("2026-08-28T00:00:00Z").unwrap(), 1);
    assert_ne!(store.activity_revision().unwrap(), revision);
    assert_eq!(
        store
            .conn
            .query_row("SELECT max(rowid) FROM otel_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn activity_facets_cover_the_whole_view_and_share_its_revision() {
    let dir = tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("facets.db")).unwrap();
    let empty = store.activity_facets(None).unwrap();
    assert!(empty.models.is_empty());
    assert!(empty.efforts.is_empty());
    for index in 0..52 {
        let mut task = completed_snapshot(&format!("turn-{index:02}"), "2026-08-29T00:00:00Z");
        task.turns[0].model = Some(format!("task-model-{index:02}"));
        task.turns[0].effort = Some(if index == 0 { "only-oldest" } else { "high" }.into());
        task.model_calls[0].model = Some(format!("call-model-{index:02}"));
        task.model_calls[0].effort = None;
        commit_snapshot(&mut store, &task, index + 1, false);
    }
    let page = store.page_activity(&ActivityFilter::default()).unwrap();
    assert_eq!(page.items.len(), 50);
    assert!(
        !page
            .items
            .iter()
            .any(|row| row.effort.as_deref() == Some("only-oldest"))
    );
    store
        .insert_otel_metadata(super::tests::otel_event("otel-only-model", None))
        .unwrap();
    let turns = store.activity_facets(Some("turns")).unwrap();
    assert_eq!(turns.models.len(), 52);
    assert!(turns.models.contains(&"task-model-00".into()));
    assert_eq!(turns.efforts, vec!["high", "only-oldest"]);
    assert_eq!(turns.revision, store.activity_revision().unwrap());
    let calls = store.activity_facets(Some("modelCalls")).unwrap();
    assert_eq!(calls.models.len(), 53);
    assert!(calls.models.contains(&"call-model-00".into()));
    assert!(calls.models.contains(&"gpt-test".into()));
    assert_eq!(calls.efforts, turns.efforts);
    assert_eq!(calls.revision, turns.revision);
}
