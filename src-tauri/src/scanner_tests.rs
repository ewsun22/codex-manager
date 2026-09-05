//! Scanner integration and optional benchmarks use only artificial temporary
//! Codex homes. Credential adapters are constructed, never invoked.

use super::*;

fn fixture_state(root: &Path) -> AppState {
    let home = root.join("codex-home");
    fs::create_dir_all(home.join("sessions")).unwrap();
    let home = fs::canonicalize(home).unwrap();
    let database_path = root.join("manager.db");
    let store = Store::open(&database_path).unwrap();
    let mut settings = store.settings().unwrap();
    settings.codex_homes = vec![home.to_string_lossy().into_owned()];
    store.save_settings(&settings).unwrap();
    AppState {
        store: Mutex::new(store),
        database_path,
        data_dir: root.join("data"),
        scan_gate: Mutex::new(()),
        scan_scheduler: Mutex::new(scanner::ScanScheduler::default()),
        retention_maintenance: Mutex::new(None),
        otel: Mutex::new(None),
        otel_error: Mutex::new(None),
        watcher: Mutex::new(None),
        capability: Mutex::new(None),
        scan_warning: Mutex::new(None),
        project_warning: Mutex::new(None),
        update_gate: tokio::sync::Mutex::new(()),
        pending_update: Mutex::new(None),
        account_gate: tokio::sync::Mutex::new(()),
        credential_mutation_gate: tokio::sync::Mutex::new(()),
        auth_stage_gate: tokio::sync::Mutex::new(()),
        login: Mutex::new(account::LoginRuntime::default()),
        auth_executable: Mutex::new(None),
        auth_profiles: std::sync::Arc::new(auth_profiles::AuthProfileStore::load()),
        proxy_auth_profiles: std::sync::Arc::new(proxy_auth::ProxyAuthStore::load()),
        auth_profile_revisions: Mutex::new(VecDeque::new()),
        codex_config_gate: Mutex::new(()),
        gateway_transition_gate: tokio::sync::Mutex::new(()),
        gateway: Mutex::new(None),
        gateway_error: Mutex::new(None),
        gateway_installing: Mutex::new(false),
    }
}

fn event(kind: &str, ordinal: i64, payload: serde_json::Value) -> String {
    serde_json::json!({"type":kind,"ordinal":ordinal,"timestamp":Utc::now().to_rfc3339(),"payload":payload}).to_string() + "\n"
}

fn initialize_rollout(path: &Path) {
    let content = event(
        "session_meta",
        0,
        serde_json::json!({"session_id":"artificial-benchmark","model_provider":"openai"}),
    ) + &event(
        "event_msg",
        1,
        serde_json::json!({"type":"task_started","turn_id":"artificial-turn"}),
    ) + &event(
        "turn_context",
        2,
        serde_json::json!({"turn_id":"artificial-turn","model":"gpt-5.4","effort":"medium"}),
    );
    fs::write(path, content).unwrap();
}

fn append_usage(path: &Path, cumulative: i64) {
    let content = event(
        "event_msg",
        cumulative + 2,
        serde_json::json!({"type":"token_count","info":{"total_token_usage":{
            "input_tokens":cumulative,"cached_input_tokens":0,"cache_write_input_tokens":0,
            "output_tokens":0,"reasoning_output_tokens":0,"total_tokens":cumulative,
        }}}),
    );
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(content.as_bytes())
        .unwrap();
}

fn finish_reconciliation(state: &AppState) {
    for _ in 0..1_000 {
        if !scan_scheduled(state, false, None).unwrap().pending {
            return;
        }
    }
    panic!("artificial reconciliation did not finish");
}

#[test]
fn a_changed_file_does_not_rescan_unchanged_history() {
    let directory = tempfile::tempdir().unwrap();
    let state = fixture_state(directory.path());
    let sessions = directory.path().join("codex-home/sessions");
    for index in 0..128 {
        fs::write(sessions.join(format!("{index}.jsonl")), b"").unwrap();
    }
    let current = fs::canonicalize(&sessions).unwrap().join("live.jsonl");
    initialize_rollout(&current);
    finish_reconciliation(&state);
    append_usage(&current, 10);
    let inbox = Mutex::new(scanner::ChangeInbox::default());
    inbox.lock().unwrap().record_file(current.clone());
    let stats = scan_scheduled(&state, false, Some(&inbox)).unwrap();
    assert_eq!(stats.files_checked, 1);
    assert_eq!(stats.discovered_entries, 0);
    assert_eq!(stats.changed_files, 1);
    assert!(!stats.pending);
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .checkpoint(current.to_str().unwrap())
            .unwrap(),
        fs::metadata(&current).unwrap().len() as i64
    );
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .summary()
            .unwrap()
            .3
            .total_tokens,
        10
    );
}

#[test]
fn file_batch_continuation_keeps_the_checkpoint_and_does_not_retry_partial_eof() {
    let directory = tempfile::tempdir().unwrap();
    let state = fixture_state(directory.path());
    let current = fs::canonicalize(directory.path().join("codex-home/sessions"))
        .unwrap()
        .join("large.jsonl");
    initialize_rollout(&current);
    let mut file = fs::OpenOptions::new().append(true).open(&current).unwrap();
    for ordinal in 3..MAX_SCAN_EVENTS_PER_FILE_BATCH as i64 + 1 {
        file.write_all(
            event(
                "event_msg",
                ordinal,
                serde_json::json!({"type":"artificial-progress"}),
            )
            .as_bytes(),
        )
        .unwrap();
    }
    file.write_all(b"{\"unfinished\":").unwrap();
    drop(file);
    let first = scan_file_unlocked(&state, &current, &mut ScanBudget::new()).unwrap();
    assert!(first.changed);
    assert!(first.needs_continuation);
    let before = state
        .store
        .lock()
        .unwrap()
        .checkpoint(current.to_str().unwrap())
        .unwrap();
    let second = scan_file_unlocked(&state, &current, &mut ScanBudget::new()).unwrap();
    assert!(second.changed);
    assert!(!second.needs_continuation);
    assert!(
        state
            .store
            .lock()
            .unwrap()
            .checkpoint(current.to_str().unwrap())
            .unwrap()
            > before
    );
    let partial = scan_file_unlocked(&state, &current, &mut ScanBudget::new()).unwrap();
    assert!(!partial.changed);
    assert!(!partial.needs_continuation);
}

#[cfg(unix)]
#[test]
fn changed_paths_still_require_safe_fs_containment() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let state = fixture_state(directory.path());
    finish_reconciliation(&state);
    let outside = directory.path().join("outside.jsonl");
    initialize_rollout(&outside);
    let escaped = fs::canonicalize(directory.path().join("codex-home/sessions"))
        .unwrap()
        .join("link.jsonl");
    symlink(&outside, &escaped).unwrap();
    let inbox = Mutex::new(scanner::ChangeInbox::default());
    inbox.lock().unwrap().record_file(escaped);
    let stats = scan_scheduled(&state, false, Some(&inbox)).unwrap();
    assert_eq!(stats.changed_files, 0);
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .page_activity(&ActivityFilter::default())
            .unwrap()
            .total,
        0
    );
    assert_eq!(fs::read_to_string(&outside).unwrap().lines().count(), 3);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkSample {
    checkpoint_latency_ms: f64,
    round_duration_ms: f64,
    discovered_entries: usize,
    files_checked: usize,
}

/// The pre-S5 discovery/sort/check loop, retained only for this optional
/// benchmark. Uses the same safe opener, parser, Store and synthetic fixture as
/// the current implementation; all counts are observed, not extrapolated.
fn legacy_scan_for_benchmark(state: &AppState, target: &Path) -> BenchmarkSample {
    let started = Instant::now();
    maintain_retention(state, false).unwrap();
    let homes = get_settings_inner(state).unwrap().codex_homes;
    let mut files = Vec::new();
    let mut visited = 0;
    for home in homes {
        for (source, directory) in ["sessions", "archived_sessions"].into_iter().enumerate() {
            let root = Path::new(&home).join(directory);
            if !fs::symlink_metadata(&root)
                .is_ok_and(|entry| entry.is_dir() && !entry.file_type().is_symlink())
            {
                continue;
            }
            let walker = walkdir::WalkDir::new(root)
                .follow_links(false)
                .max_depth(16)
                .sort_by(|left, right| right.file_name().cmp(left.file_name()))
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || !entry.file_type().is_dir()
                        || !entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| name.starts_with('.'))
                });
            for entry in walker {
                visited += 1;
                assert!(
                    visited <= 250_000 && started.elapsed() < Duration::from_secs(5),
                    "legacy discovery exceeded its original safety budget"
                );
                let entry = entry.unwrap();
                if entry.file_type().is_file()
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|value| value == "jsonl")
                {
                    files.push((
                        source,
                        entry.metadata().unwrap().modified().unwrap(),
                        entry.into_path(),
                    ));
                }
            }
        }
    }
    files.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let mut budget = ScanBudget::new();
    let mut checked = 0;
    let mut checkpoint_latency_ms = None;
    for (_, _, path) in files {
        assert!(
            !budget.exhausted(),
            "legacy processing exceeded its original safety budget"
        );
        scan_file_unlocked(state, &path, &mut budget).unwrap();
        checked += 1;
        if path == target {
            checkpoint_latency_ms = Some(started.elapsed().as_secs_f64() * 1_000.0);
        }
    }
    BenchmarkSample {
        checkpoint_latency_ms: checkpoint_latency_ms.unwrap(),
        round_duration_ms: started.elapsed().as_secs_f64() * 1_000.0,
        discovered_entries: visited,
        files_checked: checked,
    }
}

#[test]
#[ignore = "manual synthetic filesystem benchmark: 1,000 / 10,000 / 50,000 files"]
fn benchmark_changed_file_scanning_against_legacy_full_scans() {
    let mut results = Vec::new();
    for count in [1_000, 10_000, 50_000] {
        let directory = tempfile::tempdir().unwrap();
        let state = fixture_state(directory.path());
        let sessions = fs::canonicalize(directory.path().join("codex-home/sessions")).unwrap();
        for index in 1..count {
            fs::write(sessions.join(format!("history-{index:05}.jsonl")), b"").unwrap();
        }
        let current = sessions.join("live.jsonl");
        initialize_rollout(&current);
        finish_reconciliation(&state);
        let mut legacy = Vec::new();
        let mut current_samples = Vec::new();
        for iteration in 0..5 {
            append_usage(&current, iteration * 2 + 1);
            legacy.push(legacy_scan_for_benchmark(&state, &current));
            append_usage(&current, iteration * 2 + 2);
            let inbox = Mutex::new(scanner::ChangeInbox::default());
            let started = Instant::now();
            inbox.lock().unwrap().record_file(current.clone());
            let stats = scan_scheduled(&state, false, Some(&inbox)).unwrap();
            let latency = started.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(stats.files_checked, 1);
            assert_eq!(stats.discovered_entries, 0);
            assert_eq!(
                state
                    .store
                    .lock()
                    .unwrap()
                    .checkpoint(current.to_str().unwrap())
                    .unwrap(),
                fs::metadata(&current).unwrap().len() as i64
            );
            current_samples.push(BenchmarkSample {
                checkpoint_latency_ms: latency,
                round_duration_ms: latency,
                discovered_entries: stats.discovered_entries,
                files_checked: stats.files_checked,
            });
        }
        results
            .push(serde_json::json!({"fileCount":count,"legacy":legacy,"current":current_samples}));
    }
    let output = serde_json::json!({
        "os":std::env::consts::OS,"arch":std::env::consts::ARCH,"samplesPerCase":5,
        "scope":"warm filesystem; synthetic temporary home; safe_fs + SQLite + actual JSONL checkpoint; excludes notification debounce and UI refresh",
        "results":results,
    });
    let encoded = serde_json::to_string_pretty(&output).unwrap();
    if let Ok(path) = std::env::var("CODEX_MANAGER_SCANNER_BENCH_OUTPUT") {
        fs::write(path, &encoded).unwrap();
    }
    println!("{encoded}");
}
