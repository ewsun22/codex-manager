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
        "get_otel_config",
        "update_settings",
        "rescan",
        "probe_codex",
        "check_for_update",
        "install_pending_update",
        "get_codex_account",
        "start_codex_login",
        "list_auth_profiles",
        "import_auth_profile",
        "activate_auth_profile",
        "delete_auth_profile",
        "restore_auth_profile",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("生成 Tauri 权限清单失败");
}
