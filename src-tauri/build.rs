fn main() {
    const COMMANDS: &[&str] = &[
        "bootstrap",
        "get_dashboard",
        "list_activity",
        "list_projects",
        "discover_projects",
        "get_agents_chain",
        "open_agents_file",
        "create_agents_file",
        "save_agents_file",
        "list_agents_revisions",
        "restore_agents_revision",
        "list_sources",
        "list_pricing_rules",
        "get_settings",
        "update_settings",
        "rescan",
        "probe_codex",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("生成 Tauri 权限清单失败");
}
