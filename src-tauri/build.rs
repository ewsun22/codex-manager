use chrono::{TimeZone, Utc};
use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=CODEX_MANAGER_BUILD_TIME");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=CODEX_MANAGER_COMMIT_SHA");
    let build_time = env::var("CODEX_MANAGER_BUILD_TIME")
        .ok()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .or_else(|| {
            env::var("SOURCE_DATE_EPOCH")
                .ok()
                .and_then(|value| value.parse::<i64>().ok())
                .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
                .map(|value| value.to_rfc3339())
        })
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    println!("cargo:rustc-env=CODEX_MANAGER_BUILD_TIME={build_time}");
    if let Some(commit) = env::var("CODEX_MANAGER_COMMIT_SHA")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| {
            (7..=40).contains(&value.len())
                && value.chars().all(|character| character.is_ascii_hexdigit())
        })
    {
        println!(
            "cargo:rustc-env=CODEX_MANAGER_COMMIT_SHA={}",
            &commit[..commit.len().min(12)]
        );
    }
    const COMMANDS: &[&str] = &[
        "bootstrap",
        "get_dashboard",
        "list_activity",
        "get_activity_facets",
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
        "get_codex_config_snapshot",
        "save_codex_config_profile",
        "delete_codex_config_profile",
        "preview_codex_config_profile",
        "apply_codex_config_profile",
        "restore_codex_config",
        "list_proxy_auth_profiles",
        "import_proxy_auth_profile",
        "set_proxy_auth_profile_enabled",
        "delete_proxy_auth_profile",
        "restore_proxy_auth_profile",
        "list_codex_providers",
        "save_codex_provider",
        "delete_codex_provider",
        "get_codex_gateway_status",
        "update_codex_gateway_port",
        "check_latest_cliproxy_core",
        "install_latest_cliproxy_core",
        "start_codex_gateway",
        "stop_codex_gateway",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("生成 Tauri 权限清单失败");
}
