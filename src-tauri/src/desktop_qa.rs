//! Debug-feature-only real desktop harness. No production paths or credentials.
use super::*;
use tauri::{Listener, WebviewUrl, WebviewWindowBuilder};

fn fixture_state(root: &Path) -> anyhow::Result<AppState> {
    let home = root.join("codex-home");
    let project = root.join("project");
    let empty_project = root.join("empty-project");
    let data_dir = root.join("app-data");
    fs::create_dir_all(home.join("sessions"))?;
    fs::create_dir_all(project.join("nested"))?;
    fs::create_dir_all(&empty_project)?;
    fs::write(
        empty_project.join("package.json"),
        "{\"name\":\"desktop-qa-empty\",\"private\":true}\n",
    )?;
    fs::create_dir_all(&data_dir)?;
    fs::write(
        project.join("AGENTS.md"),
        "# Desktop QA\n\n人工验收文件。\n",
    )?;
    fs::write(project.join("nested/AGENTS.md"), "# Nested QA\n")?;
    let template = include_str!("../../tests/fixtures/rollout-basic.jsonl");
    for index in 0..60 {
        let timestamp = (Utc::now() - ChronoDuration::minutes(index)).to_rfc3339();
        let mut output = String::new();
        for line in template.lines() {
            let mut record: serde_json::Value = serde_json::from_str(line)?;
            record["timestamp"] = timestamp.clone().into();
            let payload = &mut record["payload"];
            if payload.get("cwd").is_some() {
                payload["cwd"] = project.to_string_lossy().as_ref().into();
            }
            if payload.get("session_id").is_some() {
                payload["session_id"] = format!("qa-session-{index}").into();
                payload["id"] = format!("qa-session-{index}").into();
            }
            if payload.get("turn_id").is_some() {
                payload["turn_id"] = format!("qa-turn-{index}").into();
            }
            if payload.get("model").is_some() {
                payload["model"] = if index == 59 {
                    "qa-rare-model"
                } else {
                    "qa-common-model"
                }
                .into();
            }
            for key in ["timestamp", "started_at", "completed_at"] {
                if payload.get(key).is_some() {
                    payload[key] = timestamp.clone().into();
                }
            }
            output.push_str(&serde_json::to_string(&record)?);
            output.push('\n');
        }
        fs::write(
            home.join("sessions").join(format!("qa-{index:03}.jsonl")),
            output,
        )?;
    }
    let database_path = data_dir.join("codex-manager.db");
    // Construct an artificial v13-shaped database. Its only downgraded objects
    // are those added by migration 14; never copy a real user database.
    {
        let store = Store::open(&database_path)?;
        let mut settings = store.settings()?;
        settings.codex_homes = vec![home.to_string_lossy().into_owned()];
        settings.authorized_roots = vec![
            project.to_string_lossy().into_owned(),
            empty_project.to_string_lossy().into_owned(),
        ];
        settings.telemetry_enabled = false;
        store.save_settings(&settings)?;
    }
    {
        let connection = rusqlite::Connection::open(&database_path)?;
        connection.execute_batch("DROP TABLE activity_maintenance; DROP INDEX idx_timeline_items_turn; DELETE FROM schema_migrations WHERE version=14;")?;
    }
    fs::write(
        root.join("migration-before.json"),
        "{\"schemaVersion\":13,\"source\":\"artificial fixture\"}\n",
    )?;
    let store = Store::open(&database_path)?;
    {
        let connection = rusqlite::Connection::open(&database_path)?;
        let version: i64 =
            connection.query_row("SELECT max(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        fs::write(
            root.join("migration-after.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"schemaVersion":version,"integrityCheck":integrity}),
            )?,
        )?;
    }
    Ok(AppState {
        store: Mutex::new(store),
        database_path,
        data_dir,
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
        auth_profiles: std::sync::Arc::new(
            auth_profiles::AuthProfileStore::disabled_for_desktop_qa(),
        ),
        proxy_auth_profiles: std::sync::Arc::new(
            proxy_auth::ProxyAuthStore::disabled_for_desktop_qa(),
        ),
        auth_profile_revisions: Mutex::new(VecDeque::new()),
        codex_config_gate: Mutex::new(()),
        gateway_transition_gate: tokio::sync::Mutex::new(()),
        gateway: Mutex::new(None),
        gateway_error: Mutex::new(None),
        gateway_installing: Mutex::new(false),
    })
}

pub fn run() {
    let temporary = tempfile::Builder::new()
        .prefix("codex-manager-desktop-qa-")
        .tempdir()
        .expect("create isolated QA root");
    let root = fs::canonicalize(temporary.path()).expect("canonical QA root");
    // Retain only our own fixture/evidence directory for inspection after close.
    let _ = temporary.keep();
    let state = fixture_state(&root).expect("prepare artificial QA data");
    scan_all(&state).expect("scan only artificial QA home");
    discover_projects_inner(&state).expect("discover only artificial QA project");
    let store_id = Uuid::new_v4();
    let autorun = std::env::var("CODEX_MANAGER_QA_AUTORUN").is_ok_and(|value| value == "1");
    let manifest = serde_json::json!({"root":root,"project":root.join("project"),"emptyProject":root.join("empty-project"),"database":state.database_path,"webviewStore":store_id.to_string(),"incognito":true,"autorun":autorun,"credentialBackend":"disabled","updaterPlugin":false});
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    println!("DESKTOP_QA_ROOT={}", root.display());
    let mut context = app_context();
    context.config_mut().identifier = "cc.codex.manager.desktop-qa".into();
    context.config_mut().app.windows.clear();
    let lifecycle_root = root.clone();
    tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            let report_root = root.clone();
            app.listen("desktop-qa-report", move |event| {
                if event.payload().len() <= 1024 * 1024
                    && serde_json::from_str::<serde_json::Value>(event.payload()).is_ok()
                {
                    let _ = fs::write(report_root.join("desktop-results.json"), event.payload());
                }
            });
            let control_root = root.clone();
            let handle = app.handle().clone();
            app.listen("desktop-qa-control", move |event| match event.payload() {
                "\"close\"" => {
                    if let Some(window) = handle.get_webview_window("main") {
                        let _ = window.close();
                    }
                }
                "\"external-edit\"" => {
                    let result = fs::write(
                        control_root.join("project/AGENTS.md"),
                        "# Desktop QA external edit\n",
                    )
                    .is_ok();
                    let _ = handle.emit("desktop-qa-external-written", result);
                }
                _ => {}
            });
            let metadata = format!(
                "window.__DESKTOP_QA = {};",
                serde_json::to_string(&manifest)?
            );
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("Codex Manager · 隔离桌面 QA")
                    .inner_size(1440.0, 960.0)
                    .incognito(true)
                    .data_store_identifier(*store_id.as_bytes())
                    .initialization_script(&metadata)
                    .initialization_script(if autorun {
                        include_str!("desktop_qa_driver.js")
                    } else {
                        ""
                    })
                    .build()?;
            window.show()?;
            Ok(())
        })
        // Exact safe-command allowlist. Account, updater, providers, config,
        // executable probing and settings mutation have no handlers at all.
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            get_dashboard,
            list_activity,
            get_activity_facets,
            list_projects,
            discover_projects,
            get_agents_chain,
            open_agents_file,
            create_agents_file,
            save_agents_file,
            list_agents_revisions,
            restore_agents_revision,
            list_sources,
            list_pricing_rules,
            get_settings,
            rescan,
        ])
        .on_window_event(move |_, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let _ = fs::write(
                    lifecycle_root.join("window-destroyed.json"),
                    "{\"destroyed\":true}\n",
                );
            }
        })
        .run(context)
        .expect("run isolated desktop QA");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artificial_desktop_state_migrates_and_keeps_rare_model_beyond_first_page() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let state = fixture_state(&root).unwrap();
        assert!(state.auth_profiles.list(false).is_err());
        assert!(state.proxy_auth_profiles.list(false).is_err());
        scan_all(&state).unwrap();
        discover_projects_inner(&state).unwrap();
        let projects = list_projects_inner(&state).unwrap();
        assert!(projects.iter().any(|project| project.canonical_path
            == root.join("empty-project").to_string_lossy()
            && !project.has_agents_file));
        let page = list_activity_inner(
            &state,
            ActivityQuery {
                view: Some("turns".into()),
                limit: Some(50),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.items.len(), 50);
        assert!(
            page.items
                .iter()
                .all(|item| item.model.value.as_deref() != Some("qa-rare-model"))
        );
        let store = state.store.lock().unwrap();
        let facets = store.activity_facets(Some("turns")).unwrap();
        assert!(facets.models.contains(&"qa-rare-model".to_string()));
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("migration-after.json")).unwrap()).unwrap();
        assert_eq!(result["schemaVersion"], 14);
        assert_eq!(result["integrityCheck"], "ok");
    }
}
