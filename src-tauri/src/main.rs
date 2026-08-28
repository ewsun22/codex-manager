#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod account;
mod agents;
mod auth_profiles;
mod auth_switch;
mod otel;
mod platform;
mod pricing;
mod projects;
mod safe_fs;

use anyhow::{Context, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use codex_core::{ResumeState, RolloutNormalizer, TokenUsage};
use codex_storage::{
    ActivityFilter, ActivityRow, CommitIngest, ProjectRow, RevisionDraft, RevisionRow, SettingsRow,
    Store,
};
use directories_next::{BaseDirs, ProjectDirs};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};
use uuid::Uuid;

const MAX_JSONL_LINE_BYTES: usize = 4 * 1024 * 1024;
const MAX_ROLLOUT_FILES: usize = 200_000;
const MAX_SCAN_ENTRIES: usize = 250_000;
const MAX_SCAN_BYTES_PER_ROUND: u64 = 256 * 1024 * 1024;
const MAX_SCAN_EVENTS_PER_ROUND: usize = 200_000;
const MAX_SCAN_BYTES_PER_FILE_BATCH: u64 = 32 * 1024 * 1024;
const MAX_SCAN_EVENTS_PER_FILE_BATCH: usize = 20_000;
const MAX_SCAN_DISCOVERY_DURATION: Duration = Duration::from_secs(5);
const MAX_SCAN_DURATION: Duration = Duration::from_secs(20);
const WATCH_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(350);
const REVISION_LIMIT: i64 = 20;
const MAX_SCHEMA_PROBE_BYTES: u64 = 64 * 1024 * 1024;
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_UPDATE_NOTES_CHARS: usize = 4_000;

struct AppState {
    store: Mutex<Store>,
    database_path: PathBuf,
    data_dir: PathBuf,
    scan_gate: Mutex<()>,
    otel: Mutex<Option<otel::ServerHandle>>,
    otel_error: Mutex<Option<String>>,
    watcher: Mutex<Option<WatchRuntime>>,
    capability: Mutex<Option<Capability>>,
    scan_warning: Mutex<Option<String>>,
    project_warning: Mutex<Option<String>>,
    update_gate: tokio::sync::Mutex<()>,
    pending_update: Mutex<Option<Update>>,
    account_gate: tokio::sync::Mutex<()>,
    credential_mutation_gate: tokio::sync::Mutex<()>,
    auth_stage_gate: tokio::sync::Mutex<()>,
    login: Mutex<account::LoginRuntime>,
    auth_executable: Mutex<Option<platform::TrustedCodexAuthExecutable>>,
    auth_profiles: std::sync::Arc<auth_profiles::AuthProfileStore>,
    auth_profile_revisions: Mutex<VecDeque<AuthProfileRevision>>,
}

#[derive(Clone)]
struct AuthProfileRevision {
    token: String,
    stamp: safe_fs::FileStamp,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthProfilesSnapshot {
    supported: bool,
    activation_available: bool,
    active_profile_id: Option<String>,
    active_revision: Option<String>,
    profiles: Vec<auth_profiles::AuthProfile>,
    deleted_retention_days: u8,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthProfileOperationResult {
    changed: bool,
    message: String,
}

enum WatchSignal {
    Reconcile,
    Stop,
}

struct WatchRuntime {
    _watcher: RecommendedWatcher,
    signal: mpsc::SyncSender<WatchSignal>,
}

impl Drop for WatchRuntime {
    fn drop(&mut self) {
        let _ = self.signal.try_send(WatchSignal::Stop);
    }
}

struct ScanBudget {
    started: Instant,
    bytes: u64,
    events: usize,
    warning: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum RolloutSource {
    ActiveSessions,
    ArchivedSessions,
}

impl RolloutSource {
    fn directory(self) -> &'static str {
        match self {
            Self::ActiveSessions => "sessions",
            Self::ArchivedSessions => "archived_sessions",
        }
    }
}

#[derive(Debug)]
struct RolloutCandidate {
    path: PathBuf,
    source: RolloutSource,
    modified_at: SystemTime,
}

fn sort_rollout_candidates(candidates: &mut Vec<RolloutCandidate>) {
    candidates.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| right.modified_at.cmp(&left.modified_at))
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.dedup_by(|left, right| left.path == right.path);
}

impl ScanBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            bytes: 0,
            events: 0,
            warning: None,
        }
    }

    fn exhausted(&self) -> bool {
        self.started.elapsed() >= MAX_SCAN_DURATION
            || self.bytes >= MAX_SCAN_BYTES_PER_ROUND
            || self.events >= MAX_SCAN_EVENTS_PER_ROUND
    }

    fn remaining_bytes(&self) -> u64 {
        MAX_SCAN_BYTES_PER_ROUND.saturating_sub(self.bytes)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metric<T: Serialize> {
    value: Option<T>,
    source: &'static str,
    confidence: &'static str,
}

impl<T: Serialize> Metric<T> {
    fn new(value: Option<T>, source: &'static str, confidence: &'static str) -> Self {
        Self {
            value,
            source,
            confidence,
        }
    }

    fn unavailable() -> Self {
        Self::new(None, "unavailable", "unavailable")
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Activity {
    id: String,
    occurred_at: String,
    project_name: Option<String>,
    project_path: Option<String>,
    session_id: String,
    turn_id: Option<String>,
    model: Metric<String>,
    effort: Metric<String>,
    provider: Metric<String>,
    endpoint: Metric<String>,
    input_tokens: Metric<i64>,
    cached_input_tokens: Metric<i64>,
    cache_write_input_tokens: Metric<i64>,
    output_tokens: Metric<i64>,
    reasoning_output_tokens: Metric<i64>,
    total_tokens: Metric<i64>,
    cache_hit_ratio: Metric<f64>,
    equivalent_cost_usd: Metric<f64>,
    duration_ms: Metric<i64>,
    first_visible_output_ms: Metric<i64>,
    response_bytes: Metric<i64>,
    result: String,
    status_code: Option<i64>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ActivityQuery {
    cursor: Option<String>,
    limit: Option<i64>,
    project_path: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    result: Option<String>,
    search: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityPage {
    items: Vec<Activity>,
    next_cursor: Option<String>,
    total: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    range_label: String,
    records: i64,
    successful: i64,
    failed: i64,
    input_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    cache_write_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_output_tokens: Option<i64>,
    equivalent_cost_usd: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OtelConfig {
    endpoint: String,
    certificate_path: String,
    header_name: &'static str,
    config_snippet: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    canonical_path: String,
    source: String,
    exists: bool,
    is_git: bool,
    worktree: bool,
    last_seen_at: Option<String>,
    agents_file_count: i64,
    has_agents_file: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentsChain {
    project: Project,
    selected_cwd: String,
    files: Vec<agents::AgentFile>,
    effective_paths: Vec<String>,
    warnings: Vec<String>,
    max_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSnapshot {
    #[serde(flatten)]
    summary: agents::AgentFile,
    content: String,
    newline: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveInput {
    path: String,
    authorized_root: String,
    content: String,
    expected_sha256: String,
    expected_mtime_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateInput {
    project_path: String,
    authorized_root: String,
    content: String,
    file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveResult {
    snapshot: AgentSnapshot,
    revision_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Revision {
    id: String,
    path: String,
    created_at: String,
    before_sha256: String,
    after_sha256: String,
    byte_length: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceHealth {
    id: String,
    kind: String,
    label: String,
    state: String,
    last_observed_at: Option<String>,
    lag_ms: Option<i64>,
    codex_version: Option<String>,
    schema_sha256: Option<String>,
    unparsed_events: i64,
    message: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    codex_homes: Vec<String>,
    authorized_roots: Vec<String>,
    retention_days: i64,
    metadata_only: bool,
    telemetry_enabled: bool,
    price_catalog_version: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capability {
    executable_path: String,
    version: Option<String>,
    schema_sha256: Option<String>,
    checked_at: String,
    available: bool,
    message: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateStatus {
    current_version: String,
    available: bool,
    version: Option<String>,
    date: Option<String>,
    notes: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInstallResult {
    accepted: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    summary: Summary,
    activity: ActivityPage,
    projects: Vec<Project>,
    sources: Vec<SourceHealth>,
    pricing_rules: Vec<pricing::PublicRule>,
    settings: Settings,
    capability: Option<Capability>,
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    Ok(Bootstrap {
        summary: dashboard_inner(&state)?,
        activity: list_activity_inner(
            &state,
            ActivityQuery {
                limit: Some(50),
                ..Default::default()
            },
        )?,
        projects: list_projects_inner(&state)?,
        sources: list_sources_inner(&state)?,
        pricing_rules: pricing::public_rules(),
        settings: get_settings_inner(&state)?,
        capability: state.capability.lock().map_err(lock_err)?.clone(),
    })
}

#[tauri::command]
fn get_dashboard(state: State<'_, AppState>) -> Result<Summary, String> {
    dashboard_inner(&state)
}

#[tauri::command]
fn list_activity(state: State<'_, AppState>, query: ActivityQuery) -> Result<ActivityPage, String> {
    list_activity_inner(&state, query)
}

#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    list_projects_inner(&state)
}

#[tauri::command]
fn discover_projects(state: State<'_, AppState>) -> Result<Vec<Project>, String> {
    discover_projects_inner(&state)?;
    list_projects_inner(&state)
}

#[tauri::command]
fn get_agents_chain(
    state: State<'_, AppState>,
    project_path: String,
    selected_cwd: String,
) -> Result<AgentsChain, String> {
    agents_chain_inner(&state, &project_path, &selected_cwd)
}

#[tauri::command]
fn open_agents_file(state: State<'_, AppState>, path: String) -> Result<AgentSnapshot, String> {
    open_agents_inner(&state, &path)
}

#[tauri::command]
fn create_agents_file(
    state: State<'_, AppState>,
    input: CreateInput,
) -> Result<SaveResult, String> {
    create_agents_inner(&state, input)
}

#[tauri::command]
fn save_agents_file(state: State<'_, AppState>, input: SaveInput) -> Result<SaveResult, String> {
    save_agents_inner(&state, input)
}

#[tauri::command]
fn list_agents_revisions(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<Revision>, String> {
    read_location(&state, Path::new(&path))?;
    let rows = state
        .store
        .lock()
        .map_err(lock_err)?
        .revisions(&path)
        .map_err(display_error)?;
    Ok(rows.into_iter().map(revision_public).collect())
}

#[tauri::command]
fn restore_agents_revision(
    state: State<'_, AppState>,
    revision_id: String,
    expected_sha256: Option<String>,
) -> Result<SaveResult, String> {
    restore_agents_inner(
        &state,
        &revision_id,
        expected_sha256
            .as_deref()
            .ok_or_else(|| "恢复必须携带当前文件 SHA-256。".to_string())?,
    )
}

#[tauri::command]
fn list_sources(state: State<'_, AppState>) -> Result<Vec<SourceHealth>, String> {
    list_sources_inner(&state)
}

#[tauri::command]
fn list_pricing_rules() -> Vec<pricing::PublicRule> {
    pricing::public_rules()
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    get_settings_inner(&state)
}

#[tauri::command]
fn get_otel_config(state: State<'_, AppState>) -> Result<OtelConfig, String> {
    let guard = state.otel.lock().map_err(lock_err)?;
    let server = guard
        .as_ref()
        .ok_or_else(|| "OTel receiver 尚未启用；请先保存设置。".to_string())?;
    Ok(OtelConfig {
        endpoint: server.endpoint().to_string(),
        certificate_path: server.certificate_path().to_string_lossy().to_string(),
        header_name: "x-codex-manager-token",
        config_snippet: server.config_snippet(),
    })
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    update_settings_inner(&app, &state, settings)
}

#[tauri::command]
fn rescan(state: State<'_, AppState>) -> Result<Vec<SourceHealth>, String> {
    scan_all(&state)?;
    discover_projects_inner(&state)?;
    list_sources_inner(&state)
}

#[tauri::command]
fn probe_codex(state: State<'_, AppState>) -> Result<Capability, String> {
    let capability = probe_codex_inner(&state)?;
    *state.capability.lock().map_err(lock_err)? = Some(capability.clone());
    Ok(capability)
}

#[tauri::command]
async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppUpdateStatus, String> {
    let _guard = state.update_gate.lock().await;
    *state.pending_update.lock().map_err(lock_err)? = None;

    let current_version = app.package_info().version.to_string();
    let update = app
        .updater_builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|_| "更新服务配置无效，请检查安装包来源。".to_string())?
        .check()
        .await
        .map_err(|_| "暂时无法连接 GitHub 更新服务，请稍后重试。".to_string())?;

    let Some(mut update) = update else {
        return Ok(AppUpdateStatus {
            current_version,
            available: false,
            version: None,
            date: None,
            notes: None,
        });
    };
    update.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);

    let status = AppUpdateStatus {
        current_version,
        available: true,
        version: Some(update.version.clone()),
        date: update.date.and_then(|value| {
            chrono::DateTime::<Utc>::from_timestamp(value.unix_timestamp(), 0)
                .map(|date| date.to_rfc3339())
        }),
        notes: sanitize_update_notes(update.body.clone()),
    };
    *state.pending_update.lock().map_err(lock_err)? = Some(update);
    Ok(status)
}

#[tauri::command]
async fn install_pending_update(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_version: String,
) -> Result<UpdateInstallResult, String> {
    let _guard = state.update_gate.lock().await;
    let update = state
        .pending_update
        .lock()
        .map_err(lock_err)?
        .take()
        .ok_or_else(|| "待安装更新已失效，请重新检查更新。".to_string())?;

    if update.version != expected_version {
        return Err("更新版本已变化，请重新检查后再安装。".to_string());
    }

    let accepted = app
        .dialog()
        .message(format!(
            "将下载并安装 Codex Manager {}。安装完成后应用会自动重启。",
            update.version
        ))
        .title("安装 Codex Manager 更新")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "下载并安装".into(),
            "取消".into(),
        ))
        .blocking_show();

    if !accepted {
        return Ok(UpdateInstallResult { accepted: false });
    }

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|_| "更新包下载、签名验证或安装失败；未应用该更新，请重新检查。".to_string())?;
    app.restart()
}

#[tauri::command]
async fn get_codex_account(state: State<'_, AppState>) -> Result<account::AccountSnapshot, String> {
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后刷新账户状态。".to_string())?;
    let _account_guard = state
        .account_gate
        .try_lock()
        .map_err(|_| "Codex 账户状态正在刷新，请稍后重试。".to_string())?;
    let login_in_progress = state.login.lock().map_err(lock_err)?.in_progress();
    let executable = trusted_auth_executable(&state).await?;
    tauri::async_runtime::spawn_blocking(move || {
        account::read_account(executable.as_deref(), login_in_progress)
    })
    .await
    .map_err(|_| "Codex 账户状态读取任务异常结束。".to_string())
}

#[tauri::command]
async fn start_codex_login(
    state: State<'_, AppState>,
) -> Result<account::LoginStartResult, String> {
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，暂时不能启动登录。".to_string())?;
    let _account_guard = state
        .account_gate
        .try_lock()
        .map_err(|_| "Codex 账户状态正在刷新，请稍后重试。".to_string())?;
    let executable = trusted_auth_executable(&state).await?;
    let mut runtime = state.login.lock().map_err(lock_err)?;
    account::start_login(&mut runtime, executable.as_deref(), || {
        auth_switch::acquire_process_lock(&state.data_dir)
    })
}

async fn trusted_auth_executable(state: &State<'_, AppState>) -> Result<Option<PathBuf>, String> {
    let _stage_guard = state.auth_stage_gate.lock().await;
    let cached_path = {
        let candidate = state.auth_executable.lock().map_err(lock_err)?;
        candidate
            .as_ref()
            .map(|candidate| candidate.path().to_path_buf())
    };
    if let Some(path) = cached_path {
        let path_to_verify = path.clone();
        let trusted = tauri::async_runtime::spawn_blocking(move || {
            platform::trusted_codex_auth_path(&path_to_verify)
        })
        .await
        .map_err(|_| "Codex OAuth 可执行文件复验任务异常结束。".to_string())?;
        if trusted {
            return Ok(Some(path));
        }
        *state.auth_executable.lock().map_err(lock_err)? = None;
    }

    let candidate = tauri::async_runtime::spawn_blocking(platform::trusted_codex_auth_candidate)
        .await
        .map_err(|_| "Codex OAuth 可执行文件验证任务异常结束。".to_string())?;
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let path = candidate.path().to_path_buf();
    *state.auth_executable.lock().map_err(lock_err)? = Some(candidate);
    Ok(Some(path))
}

struct AuthProfileListWork {
    list: auth_profiles::AuthProfileList,
    active_stamp: Option<safe_fs::FileStamp>,
    activation_available: bool,
    message: String,
}

fn auth_profiles_supported() -> bool {
    cfg!(target_os = "macos")
}

fn require_auth_profiles_supported() -> Result<(), String> {
    if auth_profiles_supported() {
        Ok(())
    } else {
        Err("认证档案当前仅支持 macOS Keychain。".into())
    }
}

#[tauri::command]
async fn list_auth_profiles(state: State<'_, AppState>) -> Result<AuthProfilesSnapshot, String> {
    if !auth_profiles_supported() {
        return Ok(AuthProfilesSnapshot {
            supported: false,
            activation_available: false,
            active_profile_id: None,
            active_revision: None,
            profiles: Vec::new(),
            deleted_retention_days: auth_profiles::SOFT_DELETE_RETENTION_DAYS,
            message: "认证档案当前仅支持 macOS Keychain。".into(),
        });
    }
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后刷新。".to_string())?;
    let _account_guard = state
        .account_gate
        .try_lock()
        .map_err(|_| "Codex 账户状态正在刷新，请稍后重试。".to_string())?;
    let executable = trusted_auth_executable(&state).await?;
    let store = std::sync::Arc::clone(&state.auth_profiles);
    let data_dir = state.data_dir.clone();
    let work = tauri::async_runtime::spawn_blocking(move || {
        let _process_lock = auth_switch::acquire_process_lock(&data_dir)?;
        let _ = store
            .purge_expired(Utc::now())
            .map_err(|error| error.to_string())?;
        let mut list = store.list(true).map_err(|error| error.to_string())?;
        let Ok(home) = auth_switch::codex_auth_home() else {
            return Ok::<_, String>(AuthProfileListWork {
                list,
                active_stamp: None,
                activation_available: false,
                message: "Codex file 模式认证目录不可用；可管理档案，但不能切换当前账户。".into(),
            });
        };
        let Some(executable) = executable.as_deref() else {
            return Ok(AuthProfileListWork {
                list,
                active_stamp: None,
                activation_available: false,
                message: "未找到经签名验证的官方 Codex，认证档案切换已停用。".into(),
            });
        };
        let Ok((active, identity)) = read_verified_active(executable, &home) else {
            return Ok(AuthProfileListWork {
                list,
                active_stamp: None,
                activation_available: false,
                message: "官方 Codex 暂时无法复核当前 file 模式账户；切换和删除已安全停用。".into(),
            });
        };
        list = store
            .reconcile_active_identity(Some(&identity))
            .map_err(|error| error.to_string())?;
        Ok(AuthProfileListWork {
            list,
            active_stamp: Some(active.stamp),
            activation_available: true,
            message:
                "档案秘密保存在应用专用 macOS Keychain；切换会原子替换共享的 file 模式 auth.json。"
                    .into(),
        })
    })
    .await
    .map_err(|_| "认证档案读取任务异常结束。".to_string())??;
    auth_profiles_snapshot(&state, work)
}

#[tauri::command]
async fn import_auth_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
) -> Result<AuthProfileOperationResult, String> {
    require_auth_profiles_supported()?;
    let Some(path) = pick_auth_file(&app).await? else {
        return Ok(AuthProfileOperationResult {
            changed: false,
            message: "已取消导入。".into(),
        });
    };
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后重试。".to_string())?;
    let _account_guard = state
        .account_gate
        .try_lock()
        .map_err(|_| "Codex 账户状态正在刷新，请稍后重试。".to_string())?;
    if state.login.lock().map_err(lock_err)?.in_progress() {
        return Err("官方登录进行中，暂时不能导入认证档案。".into());
    }
    let executable = trusted_auth_executable(&state)
        .await?
        .ok_or_else(|| "未找到经签名验证的官方 Codex，不能验证认证文件。".to_string())?;
    let store = std::sync::Arc::clone(&state.auth_profiles);
    let data_dir = state.data_dir.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let _process_lock = auth_switch::acquire_process_lock(&data_dir)?;
        let source = auth_switch::read_native_import(&path)?;
        auth_profiles::validate_structure(&source).map_err(|error| error.to_string())?;
        let validated = account::validate_auth_material(&executable, &source)?;
        store
            .import_validated(
                &validated.bytes,
                Some(&label),
                &validated.identity_fingerprint,
            )
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| "认证档案导入任务异常结束。".to_string())??;
    let message = match outcome.action.as_str() {
        "created" => "认证档案已安全导入 Keychain。",
        "restored" => "同一账户的认证档案已恢复并更新。",
        _ => "同一账户的认证档案已更新。",
    };
    Ok(AuthProfileOperationResult {
        changed: true,
        message: message.into(),
    })
}

#[tauri::command]
async fn activate_auth_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
    active_revision: String,
) -> Result<AuthProfileOperationResult, String> {
    require_auth_profiles_supported()?;
    if !confirm_native(
        &app,
        "切换 Codex 当前账户",
        "此操作会更新 Codex CLI 与 IDE 扩展共享的 file 模式认证缓存。请先结束正在运行的 Codex 任务。",
        "确认切换",
    )
    .await?
    {
        return Ok(AuthProfileOperationResult {
            changed: false,
            message: "已取消账户切换。".into(),
        });
    }
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后重试。".to_string())?;
    let _account_guard = state.account_gate.lock().await;
    if state.login.lock().map_err(lock_err)?.in_progress() {
        return Err("官方登录进行中，暂时不能切换认证档案。".into());
    }
    let expected = resolve_auth_profile_revision(&state, &active_revision)?;
    let executable = trusted_auth_executable(&state)
        .await?
        .ok_or_else(|| "未找到经签名验证的官方 Codex，不能切换认证档案。".to_string())?;
    let store = std::sync::Arc::clone(&state.auth_profiles);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        activate_auth_profile_inner(&store, &data_dir, &executable, &profile_id, &expected)
    })
    .await
    .map_err(|_| "认证档案切换任务异常结束。".to_string())??;
    state
        .auth_profile_revisions
        .lock()
        .map_err(lock_err)?
        .clear();
    Ok(AuthProfileOperationResult {
        changed: true,
        message: "已切换 Codex 当前认证档案；新启动的 CLI/IDE 会话将使用该账户。".into(),
    })
}

#[tauri::command]
async fn delete_auth_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<AuthProfileOperationResult, String> {
    require_auth_profiles_supported()?;
    if !confirm_native(
        &app,
        "删除认证档案",
        "档案将移入应用专用 Keychain 回收站并保留 30 天；当前生效或唯一可用档案不能删除。",
        "移入回收站",
    )
    .await?
    {
        return Ok(AuthProfileOperationResult {
            changed: false,
            message: "已取消删除。".into(),
        });
    }
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后重试。".to_string())?;
    let _account_guard = state.account_gate.lock().await;
    if state.login.lock().map_err(lock_err)?.in_progress() {
        return Err("官方登录进行中，暂时不能删除认证档案。".into());
    }
    let executable = trusted_auth_executable(&state)
        .await?
        .ok_or_else(|| "未找到经签名验证的官方 Codex，不能安全删除认证档案。".to_string())?;
    let store = std::sync::Arc::clone(&state.auth_profiles);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _process_lock = auth_switch::acquire_process_lock(&data_dir)?;
        let home = auth_switch::codex_auth_home()?;
        let (current, live_identity) = read_verified_active(&executable, &home)?;
        store
            .reconcile_active_identity(Some(&live_identity))
            .map_err(|error| error.to_string())?;
        let (latest, latest_live_identity) = read_verified_active(&executable, &home)?;
        if latest.stamp != current.stamp {
            store
                .reconcile_active_identity(Some(&latest_live_identity))
                .map_err(|error| error.to_string())?;
        }
        store
            .soft_delete(&profile_id)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "认证档案删除任务异常结束。".to_string())??;
    Ok(AuthProfileOperationResult {
        changed: true,
        message: "认证档案已移入回收站，可在 30 天内恢复。".into(),
    })
}

#[tauri::command]
async fn restore_auth_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<AuthProfileOperationResult, String> {
    require_auth_profiles_supported()?;
    let _credential_guard = state
        .credential_mutation_gate
        .try_lock()
        .map_err(|_| "认证档案正在修改，请稍后重试。".to_string())?;
    if state.login.lock().map_err(lock_err)?.in_progress() {
        return Err("官方登录进行中，暂时不能恢复认证档案。".into());
    }
    let store = std::sync::Arc::clone(&state.auth_profiles);
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _process_lock = auth_switch::acquire_process_lock(&data_dir)?;
        store
            .restore(&profile_id)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "认证档案恢复任务异常结束。".to_string())??;
    Ok(AuthProfileOperationResult {
        changed: true,
        message: "认证档案已从回收站恢复。".into(),
    })
}

fn activate_auth_profile_inner(
    store: &auth_profiles::AuthProfileStore,
    data_dir: &Path,
    executable: &Path,
    profile_id: &str,
    expected: &safe_fs::FileStamp,
) -> Result<(), String> {
    let _process_lock = auth_switch::acquire_process_lock(data_dir)?;
    let home = auth_switch::codex_auth_home()?;
    let (current, live_identity) = read_verified_active(executable, &home)?;
    if current.stamp.sha256 != expected.sha256
        || current.stamp.mtime_ms != expected.mtime_ms
        || current.stamp.identity != expected.identity
    {
        return Err("活动认证文件已被 Codex 或其他进程修改；请刷新后重试。".into());
    }
    let old_stamp = current.stamp.clone();
    let old_bytes = current.bytes;
    let current_profiles = store
        .reconcile_active_identity(Some(&live_identity))
        .map_err(|error| error.to_string())?;
    if current_profiles.active_profile_id.as_deref() == Some(profile_id) {
        return Ok(());
    }
    if let Some(current_id) = current_profiles.active_profile_id.as_deref() {
        store
            .update_secret(current_id, &old_bytes, &live_identity)
            .map_err(|error| error.to_string())?;
    }

    let target = store
        .read_target_secret(profile_id)
        .map_err(|error| error.to_string())?;
    auth_profiles::validate_structure(&target).map_err(|error| error.to_string())?;
    let validated_target = account::validate_auth_material(executable, &target)?;
    if !store
        .profile_matches_verified_identity(profile_id, &validated_target.identity_fingerprint)
        .map_err(|error| error.to_string())?
    {
        return Err("目标认证档案未通过账户身份复核。".into());
    }
    store
        .update_secret(
            profile_id,
            &validated_target.bytes,
            &validated_target.identity_fingerprint,
        )
        .map_err(|error| error.to_string())?;
    let written = auth_switch::replace_active(&home, &old_stamp, &validated_target.bytes)?;
    let post = read_verified_active(executable, &home);
    if post.as_ref().map(|(_, identity)| identity.as_str())
        != Ok(validated_target.identity_fingerprint.as_str())
    {
        rollback_auth_switch(
            executable,
            &home,
            &written,
            &validated_target.identity_fingerprint,
            &old_bytes,
        )?;
        return Err("切换后的官方 Codex 账户验证失败，已恢复原认证文件。".into());
    }
    let (post_auth, post_identity) = post.expect("已在上方验证切换后账户");
    if let Err(error) = store.update_secret(profile_id, &post_auth.bytes, &post_identity) {
        rollback_auth_switch(
            executable,
            &home,
            &written,
            &validated_target.identity_fingerprint,
            &old_bytes,
        )?;
        return Err(error.to_string());
    }
    if let Err(error) = store.set_active_after_live_switch(profile_id) {
        rollback_auth_switch(
            executable,
            &home,
            &written,
            &validated_target.identity_fingerprint,
            &old_bytes,
        )?;
        return Err(error.to_string());
    }
    Ok(())
}

fn rollback_auth_switch(
    executable: &Path,
    home: &Path,
    written: &safe_fs::FileStamp,
    target_identity: &str,
    old_bytes: &[u8],
) -> Result<(), String> {
    let (current, current_identity) = read_verified_active(executable, home)?;
    let rollback_stamp = if current.stamp.sha256 == written.sha256
        && current.stamp.identity == written.identity
    {
        current.stamp
    } else {
        if current_identity != target_identity {
            return Err("活动认证文件已被外部进程改为另一账户；为避免覆盖，未自动回滚。".into());
        }
        current.stamp
    };
    auth_switch::replace_active(home, &rollback_stamp, old_bytes)?;
    Ok(())
}

fn read_verified_active(
    executable: &Path,
    home: &Path,
) -> Result<(safe_fs::SecretReadResult, String), String> {
    for _ in 0..3 {
        let before = auth_switch::read_active(home)?;
        let identity = account::read_active_identity(executable, &before.bytes)?;
        let after = auth_switch::read_active(home)?;
        if before.stamp == after.stamp {
            return Ok((after, identity));
        }
    }
    Err("Codex 活动认证文件在复核期间持续变化；请结束其他 Codex 任务后重试。".into())
}

fn auth_profiles_snapshot(
    state: &State<'_, AppState>,
    work: AuthProfileListWork,
) -> Result<AuthProfilesSnapshot, String> {
    let active_revision = match work.active_stamp {
        Some(stamp) => Some(remember_auth_profile_revision(state, stamp)?),
        None => None,
    };
    Ok(AuthProfilesSnapshot {
        supported: cfg!(target_os = "macos"),
        activation_available: work.activation_available,
        active_profile_id: work.list.active_profile_id,
        active_revision,
        profiles: work.list.profiles,
        deleted_retention_days: auth_profiles::SOFT_DELETE_RETENTION_DAYS,
        message: work.message,
    })
}

fn remember_auth_profile_revision(
    state: &State<'_, AppState>,
    stamp: safe_fs::FileStamp,
) -> Result<String, String> {
    let token = Uuid::new_v4().to_string();
    let mut revisions = state.auth_profile_revisions.lock().map_err(lock_err)?;
    revisions.push_back(AuthProfileRevision {
        token: token.clone(),
        stamp,
    });
    while revisions.len() > 16 {
        revisions.pop_front();
    }
    Ok(token)
}

fn resolve_auth_profile_revision(
    state: &State<'_, AppState>,
    token: &str,
) -> Result<safe_fs::FileStamp, String> {
    Uuid::parse_str(token).map_err(|_| "认证文件版本 token 无效。".to_string())?;
    state
        .auth_profile_revisions
        .lock()
        .map_err(lock_err)?
        .iter()
        .find(|revision| revision.token == token)
        .map(|revision| revision.stamp.clone())
        .ok_or_else(|| "认证文件版本已过期；请刷新档案列表。".to_string())
}

async fn pick_auth_file(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Codex 认证文件", &["json"])
        .pick_file(move |selected| {
            let _ = sender.send(selected);
        });
    let selected = receiver
        .await
        .map_err(|_| "认证文件选择器异常结束。".to_string())?;
    selected
        .map(|path| {
            path.into_path()
                .map_err(|_| "仅支持导入本机认证文件。".to_string())
        })
        .transpose()
}

async fn confirm_native(
    app: &AppHandle,
    title: &str,
    message: &str,
    confirm_label: &str,
) -> Result<bool, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            confirm_label.into(),
            "取消".into(),
        ))
        .show(move |accepted| {
            let _ = sender.send(accepted);
        });
    receiver
        .await
        .map_err(|_| "系统确认对话框异常结束。".to_string())
}

fn sanitize_update_notes(notes: Option<String>) -> Option<String> {
    let notes = notes?.trim().to_string();
    if notes.is_empty() {
        return None;
    }
    let mut chars = notes.chars();
    let mut sanitized = chars
        .by_ref()
        .take(MAX_UPDATE_NOTES_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        sanitized.push_str("\n…");
    }
    Some(sanitized)
}

fn dashboard_inner(state: &AppState) -> Result<Summary, String> {
    let all = state
        .store
        .lock()
        .map_err(lock_err)?
        .page_activity(&ActivityFilter {
            limit: Some(1),
            ..Default::default()
        })
        .map_err(display_error)?;
    let successful = activity_total_for_result(state, "success")?;
    let failed = activity_total_for_result(state, "failure")?;
    let cost_inputs = state
        .store
        .lock()
        .map_err(lock_err)?
        .cost_inputs()
        .map_err(display_error)?;
    let has_cost_inputs = !cost_inputs.is_empty();
    let mut usage = Some(TokenUsage::default());
    let mut total_cost = 0.0;
    let mut all_priced = true;
    for input in cost_inputs {
        let Some(call_total) = input.input_tokens.checked_add(input.output_tokens) else {
            usage = None;
            all_priced = false;
            continue;
        };
        let call_usage = TokenUsage {
            input_tokens: input.input_tokens,
            cached_input_tokens: input.cached_input_tokens,
            cache_write_input_tokens: input.cache_write_input_tokens,
            output_tokens: input.output_tokens,
            reasoning_output_tokens: input.reasoning_output_tokens,
            total_tokens: call_total,
        };
        if !call_usage.is_valid() {
            usage = None;
            all_priced = false;
            continue;
        }
        usage = usage.and_then(|current| current.checked_add(&call_usage));
        if let Some(estimate) = pricing::estimate(input.model.as_deref(), &call_usage) {
            total_cost += estimate.total_usd;
        } else {
            all_priced = false;
        }
    }
    Ok(Summary {
        range_label: "全部已采集模型交互".into(),
        records: all.total,
        successful,
        failed,
        input_tokens: usage.as_ref().map(|value| value.input_tokens),
        cached_input_tokens: usage.as_ref().map(|value| value.cached_input_tokens),
        cache_write_input_tokens: usage.as_ref().map(|value| value.cache_write_input_tokens),
        output_tokens: usage.as_ref().map(|value| value.output_tokens),
        reasoning_output_tokens: usage.as_ref().map(|value| value.reasoning_output_tokens),
        equivalent_cost_usd: (has_cost_inputs && all_priced && usage.is_some())
            .then_some(total_cost),
    })
}

fn activity_total_for_result(state: &AppState, result: &str) -> Result<i64, String> {
    state
        .store
        .lock()
        .map_err(lock_err)?
        .page_activity(&ActivityFilter {
            limit: Some(1),
            result: Some(result.into()),
            ..Default::default()
        })
        .map(|page| page.total)
        .map_err(display_error)
}

fn list_activity_inner(state: &AppState, query: ActivityQuery) -> Result<ActivityPage, String> {
    let result = match query.result.as_deref() {
        Some("success") => Some("success".to_string()),
        Some("failure") => Some("failure".to_string()),
        Some("running") => Some("running".to_string()),
        Some("unknown") => Some("unknown".to_string()),
        _ => None,
    };
    let page = state
        .store
        .lock()
        .map_err(lock_err)?
        .page_activity(&ActivityFilter {
            cursor: query.cursor,
            limit: query.limit,
            project_path: non_empty(query.project_path),
            model: non_empty(query.model),
            effort: non_empty(query.effort),
            result,
            search: non_empty(query.search),
        })
        .map_err(display_error)?;
    Ok(ActivityPage {
        items: page.items.into_iter().map(activity_public).collect(),
        next_cursor: page.next_cursor,
        total: page.total,
    })
}

fn activity_public(row: ActivityRow) -> Activity {
    let is_otel = row.source_kind == "otel";
    let metric_source = if is_otel { "server" } else { "rollout" };
    let metric = |value, available| {
        if available {
            Metric::new(Some(value), metric_source, "high")
        } else {
            Metric::unavailable()
        }
    };
    let input = metric(row.usage.input_tokens, row.usage_available.input_tokens);
    let cached = metric(
        row.usage.cached_input_tokens,
        row.usage_available.cached_input_tokens,
    );
    let cache_write = metric(
        row.usage.cache_write_input_tokens,
        row.usage_available.cache_write_input_tokens,
    );
    let output = metric(row.usage.output_tokens, row.usage_available.output_tokens);
    let reasoning = metric(
        row.usage.reasoning_output_tokens,
        row.usage_available.reasoning_output_tokens,
    );
    let total = metric(row.usage.total_tokens, row.usage_available.total_tokens);
    let cache_ratio = if row.usage_available.input_tokens
        && row.usage_available.cached_input_tokens
        && row.usage.input_tokens > 0
    {
        Metric::new(
            Some(row.usage.cached_input_tokens as f64 / row.usage.input_tokens as f64),
            "derived",
            "medium",
        )
    } else {
        Metric::unavailable()
    };
    let cost = if row.usage_available.input_tokens
        && row.usage_available.cached_input_tokens
        && row.usage_available.cache_write_input_tokens
        && row.usage_available.output_tokens
    {
        pricing::estimate(row.model.as_deref(), &row.usage)
            .map(|estimate| Metric::new(Some(estimate.total_usd), "configured", "medium"))
            .unwrap_or_else(Metric::unavailable)
    } else {
        Metric::unavailable()
    };
    let project_name = row
        .project_path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned);
    Activity {
        id: row.id,
        occurred_at: row.occurred_at.unwrap_or_default(),
        project_name,
        project_path: row.project_path,
        session_id: row.session_id.unwrap_or_else(|| "unavailable".into()),
        turn_id: row.turn_id,
        model: observed_string(row.model, metric_source),
        effort: if is_otel {
            Metric::unavailable()
        } else {
            observed_string(row.effort, "rollout")
        },
        provider: observed_string(row.provider, metric_source),
        endpoint: row.endpoint.map_or_else(Metric::unavailable, |value| {
            Metric::new(Some(value), "server", "high")
        }),
        input_tokens: input,
        cached_input_tokens: cached,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: reasoning,
        total_tokens: total,
        cache_hit_ratio: cache_ratio,
        equivalent_cost_usd: cost,
        duration_ms: row.duration_ms.map_or_else(Metric::unavailable, |value| {
            Metric::new(
                Some(value),
                metric_source,
                if is_otel { "medium" } else { "low" },
            )
        }),
        first_visible_output_ms: row
            .first_visible_output_ms
            .map_or_else(Metric::unavailable, |value| {
                Metric::new(Some(value), "rollout", "medium")
            }),
        response_bytes: row
            .response_bytes
            .map_or_else(Metric::unavailable, |value| {
                Metric::new(Some(value), "server", "medium")
            }),
        result: match row.result.as_str() {
            "completed" => "success",
            "failed" | "aborted" => "failure",
            "running" => "running",
            _ => "unknown",
        }
        .into(),
        status_code: row.status_code,
    }
}

fn observed_string(value: Option<String>, source: &'static str) -> Metric<String> {
    value.map_or_else(Metric::unavailable, |value| {
        Metric::new(Some(value), source, "high")
    })
}

fn get_settings_inner(state: &AppState) -> Result<Settings, String> {
    let row = state
        .store
        .lock()
        .map_err(lock_err)?
        .settings()
        .map_err(display_error)?;
    Ok(settings_public(row))
}

fn settings_public(row: SettingsRow) -> Settings {
    Settings {
        codex_homes: row.codex_homes,
        authorized_roots: row.authorized_roots,
        retention_days: row.retention_days,
        metadata_only: true,
        telemetry_enabled: row.telemetry_enabled,
        price_catalog_version: pricing::catalog().catalog_version.clone(),
    }
}

fn update_settings_inner(
    app: &AppHandle,
    state: &AppState,
    requested: Settings,
) -> Result<Settings, String> {
    if !requested.metadata_only {
        return Err("metadataOnly 必须保持 true；当前版本不支持持久化消息正文。".into());
    }
    let previous = state
        .store
        .lock()
        .map_err(lock_err)?
        .settings()
        .map_err(display_error)?;
    let next = SettingsRow {
        codex_homes: canonical_directories(&requested.codex_homes)?,
        authorized_roots: canonical_directories(&requested.authorized_roots)?,
        retention_days: requested.retention_days.clamp(1, 3650),
        telemetry_enabled: requested.telemetry_enabled,
        price_catalog_version: pricing::catalog().catalog_version.clone(),
    };
    if next.telemetry_enabled {
        start_otel(state)?;
    }
    if let Err(error) = state.store.lock().map_err(lock_err)?.save_settings(&next) {
        if !previous.telemetry_enabled {
            stop_otel(state)?;
        }
        return Err(display_error(error));
    }
    if !next.telemetry_enabled {
        stop_otel(state)?;
    }
    if let Err(error) = restart_watcher(app) {
        state
            .store
            .lock()
            .map_err(lock_err)?
            .save_settings(&previous)
            .map_err(display_error)?;
        if previous.telemetry_enabled {
            start_otel(state)?;
        } else {
            stop_otel(state)?;
        }
        let _ = restart_watcher(app);
        return Err(format!("文件监听器更新失败，设置已回滚：{error}"));
    }
    prune_retention(state, next.retention_days)?;
    Ok(settings_public(next))
}

fn start_otel(state: &AppState) -> Result<(), String> {
    let mut guard = state.otel.lock().map_err(lock_err)?;
    if guard.is_some() {
        return Ok(());
    }
    match otel::start(state.database_path.clone(), state.data_dir.clone()) {
        Ok(server) => {
            *guard = Some(server);
            *state.otel_error.lock().map_err(lock_err)? = None;
            Ok(())
        }
        Err(error) => {
            *state.otel_error.lock().map_err(lock_err)? = Some(error.clone());
            Err(error)
        }
    }
}

fn stop_otel(state: &AppState) -> Result<(), String> {
    if let Some(server) = state.otel.lock().map_err(lock_err)?.take() {
        server.stop();
    }
    *state.otel_error.lock().map_err(lock_err)? = None;
    Ok(())
}

fn prune_retention(state: &AppState, retention_days: i64) -> Result<(), String> {
    let older_than =
        (Utc::now() - ChronoDuration::days(retention_days.clamp(1, 3650))).to_rfc3339();
    state
        .store
        .lock()
        .map_err(lock_err)?
        .prune_retention(&older_than)
        .map(|_| ())
        .map_err(display_error)
}

fn scan_all(state: &AppState) -> Result<(), String> {
    let _gate = state.scan_gate.lock().map_err(lock_err)?;
    let homes = get_settings_inner(state)?.codex_homes;
    let mut files = Vec::new();
    let mut visited = 0_usize;
    let mut warning = None;
    let mut budget = ScanBudget::new();
    for home in homes {
        for source in [
            RolloutSource::ActiveSessions,
            RolloutSource::ArchivedSessions,
        ] {
            let root = Path::new(&home).join(source.directory());
            if !fs::symlink_metadata(&root)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            {
                continue;
            }
            let walker = walkdir::WalkDir::new(root)
                .follow_links(false)
                .max_depth(16)
                .sort_by(|left, right| right.file_name().cmp(left.file_name()))
                .into_iter()
                .filter_entry(is_safe_walk_entry);
            for entry in walker {
                visited = visited.saturating_add(1);
                if visited > MAX_SCAN_ENTRIES
                    || budget.started.elapsed() >= MAX_SCAN_DISCOVERY_DURATION
                {
                    warning = Some(format!(
                        "rollout 扫描已达遍历/时间预算（visited={visited}）；本轮结果为 partial，下次 reconciliation 会继续。"
                    ));
                    break;
                }
                let Ok(entry) = entry else {
                    continue;
                };
                if files.len() >= MAX_ROLLOUT_FILES {
                    warning = Some(format!(
                        "rollout 文件超过 {MAX_ROLLOUT_FILES} 个安全上限；本轮结果为 partial，请缩小 Codex home 范围。"
                    ));
                    break;
                }
                if entry.file_type().is_file()
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
                {
                    files.push(RolloutCandidate {
                        path: entry.path().to_path_buf(),
                        source,
                        modified_at: entry
                            .metadata()
                            .ok()
                            .and_then(|metadata| metadata.modified().ok())
                            .unwrap_or(SystemTime::UNIX_EPOCH),
                    });
                }
            }
            if warning.is_some() {
                break;
            }
        }
        if warning.is_some() {
            break;
        }
    }
    sort_rollout_candidates(&mut files);
    for candidate in files {
        if budget.exhausted() {
            warning.get_or_insert_with(|| {
                format!(
                    "rollout 扫描已达本轮资源预算（bytes={} events={}）；已提交的 checkpoint 保持有效。",
                    budget.bytes, budget.events
                )
            });
            break;
        }
        if let Err(error) = scan_file_unlocked(state, &candidate.path, &mut budget) {
            let path = candidate.path.to_string_lossy().to_string();
            let message = safe_error_label(&error);
            state
                .store
                .lock()
                .map_err(lock_err)?
                .mark_ingest_error(&path, &message)
                .map_err(display_error)?;
        }
    }
    if warning.is_none() {
        warning = budget.warning.take();
    }
    *state.scan_warning.lock().map_err(lock_err)? = warning;
    Ok(())
}

fn scan_file_unlocked(state: &AppState, path: &Path, round: &mut ScanBudget) -> Result<(), String> {
    let (root, relative, canonical) = rollout_location(state, path)?;
    let opened = safe_fs::open_regular_beneath(&root, &relative)?;
    let canonical_string = canonical.to_string_lossy().to_string();
    let identity = opened.identity;
    let size = i64::try_from(opened.size_bytes).map_err(display_error)?;
    let plan = state
        .store
        .lock()
        .map_err(lock_err)?
        .plan_ingest(&canonical_string, Some(&identity), size)
        .map_err(display_error)?;
    if !plan.rebuild_required && plan.checkpoint.byte_offset == size {
        return Ok(());
    }

    let start_offset = if plan.rebuild_required {
        0
    } else {
        plan.checkpoint.byte_offset.max(0)
    };
    let resume = if plan.rebuild_required {
        ResumeState::default()
    } else {
        plan.checkpoint.resume_state.clone().unwrap_or_default()
    };
    let base_unparsed = if plan.rebuild_required {
        0
    } else {
        plan.checkpoint.unparsed_events.max(0)
    };
    let mut file = opened.file;
    file.seek(SeekFrom::Start(start_offset as u64))
        .map_err(display_error)?;
    let mut reader = BufReader::new(file);
    let mut normalizer = RolloutNormalizer::from_resume_state(resume);
    let mut next_offset = start_offset;
    let mut reader_unparsed = 0_i64;
    let mut saw_complete = false;
    let mut file_bytes = 0_u64;
    let mut file_events = 0_usize;
    loop {
        if round.exhausted()
            || file_bytes >= MAX_SCAN_BYTES_PER_FILE_BATCH
            || file_events >= MAX_SCAN_EVENTS_PER_FILE_BATCH
        {
            break;
        }
        let remaining = MAX_SCAN_BYTES_PER_FILE_BATCH
            .saturating_sub(file_bytes)
            .min(round.remaining_bytes());
        if remaining <= MAX_JSONL_LINE_BYTES as u64 {
            break;
        }
        match read_bounded_line(
            &mut reader,
            MAX_JSONL_LINE_BYTES,
            usize::try_from(remaining).unwrap_or(usize::MAX),
        )? {
            BoundedLine::Eof => break,
            BoundedLine::Partial { consumed } => {
                file_bytes = file_bytes.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                break;
            }
            BoundedLine::Oversize { consumed } => {
                next_offset = next_offset.saturating_add(usize_to_i64_saturating(consumed));
                reader_unparsed = reader_unparsed.saturating_add(1);
                saw_complete = true;
                file_events = file_events.saturating_add(1);
                file_bytes = file_bytes.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
            }
            BoundedLine::Complete { bytes, consumed } => {
                let line_offset = next_offset as u64;
                next_offset = next_offset.saturating_add(usize_to_i64_saturating(consumed));
                saw_complete = true;
                file_events = file_events.saturating_add(1);
                file_bytes = file_bytes.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                match std::str::from_utf8(&bytes) {
                    Ok(line) => normalizer.parse_line(line, Some(line_offset)),
                    Err(_) => reader_unparsed = reader_unparsed.saturating_add(1),
                }
            }
            BoundedLine::BudgetExhausted {
                consumed,
                oversized,
            } => {
                file_bytes = file_bytes.saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                // A mid-line offset is not checkpoint-safe: a later scan could
                // interpret an attacker-chosen suffix as a standalone event.
                // Leave the durable checkpoint at the preceding newline. The
                // per-file and per-round byte caps still bound retry work.
                if oversized {
                    round.warning.get_or_insert_with(|| {
                        "rollout 含超过单行上限且未结束的记录；为避免从恶意后缀中间继读，checkpoint 保留在上一个换行处。"
                            .to_string()
                    });
                }
                break;
            }
        }
    }
    if file_bytes >= MAX_SCAN_BYTES_PER_FILE_BATCH || file_events >= MAX_SCAN_EVENTS_PER_FILE_BATCH
    {
        round.warning.get_or_insert_with(|| {
            format!(
                "rollout 单文件批次达到安全预算（bytes={file_bytes} events={file_events}）；结果为 partial，后续 reconciliation 会继续。"
            )
        });
    }
    round.bytes = round.bytes.saturating_add(file_bytes);
    round.events = round.events.saturating_add(file_events);
    if !saw_complete && !plan.rebuild_required {
        return Ok(());
    }
    let snapshot = normalizer.incremental_snapshot();
    let parse_unparsed = snapshot.unhandled_event_counts.values().copied().fold(
        usize_to_i64_saturating(snapshot.diagnostics.len()),
        |total, value| total.saturating_add(u64_to_i64_saturating(value)),
    );
    let resume_state = normalizer.resume_state();
    state
        .store
        .lock()
        .map_err(lock_err)?
        .commit_ingest(CommitIngest {
            path: &canonical_string,
            source_kind: "rollout",
            next_offset,
            file_identity: Some(&identity),
            snapshot: &snapshot,
            resume_state: &resume_state,
            rebuild: plan.rebuild_required,
            unparsed_events: base_unparsed
                .saturating_add(reader_unparsed)
                .saturating_add(parse_unparsed),
        })
        .map_err(display_error)
}

enum BoundedLine {
    Eof,
    Partial { consumed: usize },
    Complete { bytes: Vec<u8>, consumed: usize },
    Oversize { consumed: usize },
    BudgetExhausted { consumed: usize, oversized: bool },
}

fn usize_to_i64_saturating(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn u64_to_i64_saturating(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
    max_consumed: usize,
) -> Result<BoundedLine, String> {
    let mut bytes = Vec::new();
    let mut consumed = 0_usize;
    let mut oversized = false;
    loop {
        if consumed >= max_consumed {
            return Ok(BoundedLine::BudgetExhausted {
                consumed,
                oversized,
            });
        }
        let available = reader.fill_buf().map_err(display_error)?;
        if available.is_empty() {
            return if consumed == 0 {
                Ok(BoundedLine::Eof)
            } else {
                Ok(BoundedLine::Partial { consumed })
            };
        }
        let allowed = available.len().min(max_consumed.saturating_sub(consumed));
        let window = &available[..allowed];
        if let Some(index) = window.iter().position(|byte| *byte == b'\n') {
            let amount = index + 1;
            if !oversized {
                if bytes.len().saturating_add(index) > max_bytes {
                    oversized = true;
                } else {
                    bytes.extend_from_slice(&available[..index]);
                    if bytes.last() == Some(&b'\r') {
                        bytes.pop();
                    }
                }
            }
            reader.consume(amount);
            consumed = consumed.saturating_add(amount);
            return if oversized {
                Ok(BoundedLine::Oversize { consumed })
            } else {
                Ok(BoundedLine::Complete { bytes, consumed })
            };
        }
        let amount = window.len();
        if !oversized {
            if bytes.len().saturating_add(amount) > max_bytes {
                oversized = true;
                bytes.clear();
            } else {
                bytes.extend_from_slice(window);
            }
        }
        reader.consume(amount);
        consumed = consumed.saturating_add(amount);
    }
}

fn rollout_location(state: &AppState, path: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let settings = get_settings_inner(state)?;
    for home in settings.codex_homes {
        for directory in ["sessions", "archived_sessions"] {
            let root = Path::new(&home).join(directory);
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            if relative.as_os_str().is_empty()
                || relative
                    .components()
                    .any(|component| !matches!(component, std::path::Component::Normal(_)))
            {
                return Err("采集源相对路径不安全。".into());
            }
            let relative = relative.to_path_buf();
            let canonical = root.join(&relative);
            return Ok((root, relative, canonical));
        }
    }
    Err("采集源不在配置的 Codex home 会话目录内。".into())
}

fn is_safe_walk_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return true;
    }
    !entry.file_type().is_symlink()
        && !entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.') && name != ".")
}

fn restart_watcher(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let previous = state.watcher.lock().map_err(lock_err)?.take();
    drop(previous);
    let settings = get_settings_inner(&state)?;
    // A single coalescing slot bounds memory under filesystem event floods.
    // A periodic reconciliation remains the source of truth for dropped wakes.
    let (signal, receiver) = mpsc::sync_channel::<WatchSignal>(1);
    let callback_signal = signal.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let _ = result;
        let _ = callback_signal.try_send(WatchSignal::Reconcile);
    })
    .map_err(display_error)?;
    let mut watched = 0_usize;
    for home in settings.codex_homes {
        for directory in ["sessions", "archived_sessions"] {
            let root = Path::new(&home).join(directory);
            if root.is_dir() {
                watcher
                    .watch(&root, RecursiveMode::Recursive)
                    .map_err(display_error)?;
                watched += 1;
            }
        }
    }
    if watched == 0 {
        return Ok(());
    }
    let worker_app = app.clone();
    thread::Builder::new()
        .name("codex-manager-rollout".into())
        .spawn(move || watcher_loop(worker_app, receiver))
        .map_err(display_error)?;
    *state.watcher.lock().map_err(lock_err)? = Some(WatchRuntime {
        _watcher: watcher,
        signal,
    });
    Ok(())
}

fn watcher_loop(app: AppHandle, receiver: mpsc::Receiver<WatchSignal>) {
    loop {
        let periodic = match receiver.recv_timeout(WATCH_RECONCILE_INTERVAL) {
            Ok(WatchSignal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Ok(WatchSignal::Reconcile) => false,
            Err(mpsc::RecvTimeoutError::Timeout) => true,
        };
        if !periodic {
            let deadline = Instant::now() + WATCH_DEBOUNCE;
            while Instant::now() < deadline {
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(WatchSignal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Ok(WatchSignal::Reconcile) => {}
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                }
            }
        }
        if let Some(state) = app.try_state::<AppState>() {
            if scan_all(&state).is_ok() {
                if periodic {
                    let _ = discover_projects_inner(&state);
                }
                let _ = app.emit("local-data-refreshed", ());
            }
        } else {
            return;
        }
    }
}

fn discover_projects_inner(state: &AppState) -> Result<(), String> {
    let (observed, settings) = {
        let store = state.store.lock().map_err(lock_err)?;
        (
            store.observed_cwds().map_err(display_error)?,
            store.settings().map_err(display_error)?,
        )
    };
    let discovery = projects::discover(&observed, &settings.authorized_roots);
    *state.project_warning.lock().map_err(lock_err)? =
        (!discovery.warnings.is_empty()).then(|| discovery.warnings.join(" "));
    let now = Utc::now().to_rfc3339();
    let store = state.store.lock().map_err(lock_err)?;
    for project in discovery.projects {
        store
            .upsert_project(&ProjectRow {
                canonical_path: project.canonical_path.to_string_lossy().to_string(),
                name: project.name,
                source: project.source.clone(),
                exists: true,
                is_git: project.is_git,
                worktree: project.worktree,
                last_seen_at: (project.source == "observed").then(|| now.clone()),
            })
            .map_err(display_error)?;
    }
    Ok(())
}

fn list_projects_inner(state: &AppState) -> Result<Vec<Project>, String> {
    let (rows, settings) = {
        let store = state.store.lock().map_err(lock_err)?;
        (
            store.projects().map_err(display_error)?,
            store.settings().map_err(display_error)?,
        )
    };
    let config_home = settings.codex_homes.first().map(PathBuf::from);
    let doc_config = agents::load_doc_config(config_home.as_deref());
    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.exists = canonical_directory(Path::new(&row.canonical_path)).is_ok();
            let has_agents_file =
                row.exists && agents::has_root_agents_file(Path::new(&row.canonical_path));
            let count = if row.exists {
                agents::discover_all(Path::new(&row.canonical_path), &doc_config)
                    .map(|result| result.files.len() as i64)
                    .unwrap_or(0)
            } else {
                0
            };
            project_public(row, count, has_agents_file)
        })
        .collect())
}

fn project_public(row: ProjectRow, agents_file_count: i64, has_agents_file: bool) -> Project {
    Project {
        id: path_id(&row.canonical_path),
        name: row.name,
        canonical_path: row.canonical_path,
        source: row.source,
        exists: row.exists,
        is_git: row.is_git,
        worktree: row.worktree,
        last_seen_at: row.last_seen_at,
        agents_file_count,
        has_agents_file,
    }
}

fn agents_chain_inner(
    state: &AppState,
    project_path: &str,
    selected_cwd: &str,
) -> Result<AgentsChain, String> {
    let project_row = known_project(state, project_path)?;
    let settings = get_settings_inner(state)?;
    let roots = settings
        .authorized_roots
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let codex_home = settings.codex_homes.first().map(PathBuf::from);
    let resolution = agents::resolve_chain(
        Path::new(&project_row.canonical_path),
        Path::new(selected_cwd),
        codex_home.as_deref(),
        &roots,
    )?;
    let discovery = agents::discover_all(
        Path::new(&project_row.canonical_path),
        &agents::load_doc_config(codex_home.as_deref()),
    )?;
    let file_count = discovery.files.len() as i64;
    let has_agents_file = agents::has_root_agents_file(Path::new(&project_row.canonical_path));
    let mut warnings = resolution.warnings;
    if let Some(warning) = discovery.warning {
        warnings.push(warning);
    }
    Ok(AgentsChain {
        project: project_public(project_row, file_count, has_agents_file),
        selected_cwd: fs::canonicalize(selected_cwd)
            .map_err(display_error)?
            .to_string_lossy()
            .to_string(),
        files: resolution.files,
        effective_paths: resolution.effective_paths,
        warnings,
        max_bytes: resolution.max_bytes,
    })
}

fn known_project(state: &AppState, path: &str) -> Result<ProjectRow, String> {
    let canonical = canonical_directory(Path::new(path))?;
    state
        .store
        .lock()
        .map_err(lock_err)?
        .projects()
        .map_err(display_error)?
        .into_iter()
        .find(|project| Path::new(&project.canonical_path) == canonical)
        .ok_or_else(|| "项目尚未被 Codex Manager 发现；请先刷新项目列表。".into())
}

struct ReadLocation {
    root: PathBuf,
    relative_parent: PathBuf,
    name: String,
    kind: String,
    relative_path: String,
}

fn read_location(state: &AppState, target: &Path) -> Result<ReadLocation, String> {
    let settings = get_settings_inner(state)?;
    let target_parent = target
        .parent()
        .ok_or_else(|| "AGENTS 文件缺少父目录。".to_string())?;
    let canonical_parent = fs::canonicalize(target_parent).map_err(display_error)?;
    let mut candidates = settings
        .codex_homes
        .iter()
        .map(|path| (PathBuf::from(path), true))
        .chain(
            state
                .store
                .lock()
                .map_err(lock_err)?
                .projects()
                .map_err(display_error)?
                .into_iter()
                .map(|project| (PathBuf::from(project.canonical_path), false)),
        )
        .filter(|(root, _)| canonical_parent.starts_with(root))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(root, _)| std::cmp::Reverse(root.components().count()));
    let (root, global) = candidates
        .into_iter()
        .next()
        .ok_or_else(|| "该文件不属于已知 Codex home 或已发现项目。".to_string())?;
    let config_home = settings.codex_homes.first().map(PathBuf::from);
    let config = agents::load_doc_config(config_home.as_deref());
    let mut allowed = agents::allowed_names(&config);
    if global {
        allowed = vec!["AGENTS.override.md".into(), "AGENTS.md".into()];
    }
    let (root, relative_parent, name) = safe_fs::validate_relative_target(&root, target, &allowed)?;
    let relative_path = relative_parent.join(&name).to_string_lossy().to_string();
    let kind = if global {
        "global"
    } else if !matches!(name.as_str(), "AGENTS.override.md" | "AGENTS.md") {
        "fallback"
    } else if relative_parent.as_os_str().is_empty() {
        "project"
    } else {
        "nested"
    }
    .into();
    Ok(ReadLocation {
        root,
        relative_parent,
        name,
        kind,
        relative_path,
    })
}

fn open_agents_inner(state: &AppState, path: &str) -> Result<AgentSnapshot, String> {
    let location = read_location(state, Path::new(path))?;
    let read = safe_fs::read_authorized(
        &location.root,
        &location.relative_parent,
        &location.name,
        agents::HARD_AGENTS_READ_LIMIT,
    )?;
    snapshot_from_read(state, location, read)
}

fn snapshot_from_read(
    state: &AppState,
    location: ReadLocation,
    read: safe_fs::ReadResult,
) -> Result<AgentSnapshot, String> {
    let settings = get_settings_inner(state)?;
    let writable = read.writable
        && settings
            .authorized_roots
            .iter()
            .map(PathBuf::from)
            .any(|root| read.path.starts_with(root));
    let newline = safe_fs::newline_style(&read.bytes).to_string();
    let content = String::from_utf8(read.bytes)
        .map_err(|_| "AGENTS 文件不是有效 UTF-8；为避免破坏原文件，本版本拒绝编辑。".to_string())?;
    Ok(AgentSnapshot {
        summary: agents::AgentFile {
            path: read.path.to_string_lossy().to_string(),
            relative_path: location.relative_path,
            kind: location.kind,
            precedence: 0,
            effective: false,
            overridden: false,
            sha256: read.stamp.sha256,
            mtime_ms: read.stamp.mtime_ms,
            size_bytes: read.stamp.size_bytes,
            writable,
        },
        content,
        newline,
    })
}

fn save_agents_inner(state: &AppState, input: SaveInput) -> Result<SaveResult, String> {
    let root = authorized_root(state, &input.authorized_root)?;
    let target = Path::new(&input.path);
    let config_home = get_settings_inner(state)?
        .codex_homes
        .first()
        .map(PathBuf::from);
    let allowed = agents::allowed_names(&agents::load_doc_config(config_home.as_deref()));
    let (root, relative_parent, name) = safe_fs::validate_relative_target(&root, target, &allowed)?;
    let current = safe_fs::read_authorized(
        &root,
        &relative_parent,
        &name,
        agents::HARD_AGENTS_READ_LIMIT,
    )?;
    if current.stamp.sha256 != input.expected_sha256
        || current.stamp.mtime_ms != input.expected_mtime_ms
    {
        return Err("文件已被外部修改；请重新加载后再保存。".into());
    }
    let bytes = safe_fs::preserve_newlines(&input.content, safe_fs::newline_style(&current.bytes));
    write_with_revision(
        state,
        root,
        relative_parent,
        name,
        safe_fs::WriteMode::Replace,
        Some(safe_fs::WriteExpectation {
            sha256: &input.expected_sha256,
            mtime_ms: input.expected_mtime_ms,
        }),
        bytes,
    )
}

fn create_agents_inner(state: &AppState, input: CreateInput) -> Result<SaveResult, String> {
    if input.file_name != "AGENTS.md" {
        return Err("初版只允许显式创建项目根目录的 AGENTS.md。".into());
    }
    let project = known_project(state, &input.project_path)?;
    let root = authorized_root(state, &input.authorized_root)?;
    let project_path = canonical_directory(Path::new(&project.canonical_path))?;
    if !project_path.starts_with(&root) {
        return Err("项目不在所选授权根目录内。".into());
    }
    let target = project_path.join("AGENTS.md");
    let (root, relative_parent, name) =
        safe_fs::validate_relative_target(&root, &target, &["AGENTS.md".to_string()])?;
    let bytes = safe_fs::preserve_newlines(&input.content, "lf");
    write_with_revision(
        state,
        root,
        relative_parent,
        name,
        safe_fs::WriteMode::Create,
        None,
        bytes,
    )
}

fn write_with_revision(
    state: &AppState,
    root: PathBuf,
    relative_parent: PathBuf,
    name: String,
    mode: safe_fs::WriteMode,
    expectation: Option<safe_fs::WriteExpectation<'_>>,
    bytes: Vec<u8>,
) -> Result<SaveResult, String> {
    let revision_id = Uuid::new_v4().to_string();
    let revision_for_prepare = revision_id.clone();
    let target = root.join(&relative_parent).join(&name);
    let target_string = target.to_string_lossy().to_string();
    let created_at = Utc::now().to_rfc3339();
    let result = safe_fs::write_authorized(
        &root,
        &relative_parent,
        &name,
        mode,
        expectation,
        &bytes,
        agents::HARD_AGENTS_READ_LIMIT,
        |before, after| {
            let before_sha = safe_fs::sha256(before);
            let after_sha = safe_fs::sha256(after);
            state
                .store
                .lock()
                .map_err(lock_err)?
                .prepare_revision(RevisionDraft {
                    id: &revision_for_prepare,
                    path: &target_string,
                    created_at: &created_at,
                    before_sha256: &before_sha,
                    after_sha256: &after_sha,
                    byte_length: after.len() as i64,
                    before_content: before,
                    after_content: after,
                })
                .map_err(display_error)
        },
    );
    let (read, ()) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = state
                .store
                .lock()
                .map_err(lock_err)?
                .discard_revision(&revision_id);
            return Err(error);
        }
    };
    let committed = state
        .store
        .lock()
        .map_err(lock_err)?
        .commit_revision(&revision_id)
        .map_err(|error| {
            format!("文件已原子保存，但 revision 尚未确认；重启后会按文件 hash 恢复：{error}")
        })?;
    if !committed {
        return Err("文件已原子保存，但 revision 状态异常；请勿重复覆盖并重启应用。".into());
    }
    state
        .store
        .lock()
        .map_err(lock_err)?
        .trim_revisions(&target_string, REVISION_LIMIT)
        .map_err(display_error)?;
    let location = read_location(state, &target)?;
    Ok(SaveResult {
        snapshot: snapshot_from_read(state, location, read)?,
        revision_id,
    })
}

fn restore_agents_inner(
    state: &AppState,
    revision_id: &str,
    expected_sha256: &str,
) -> Result<SaveResult, String> {
    let revision = state
        .store
        .lock()
        .map_err(lock_err)?
        .revision(revision_id)
        .map_err(display_error)?
        .ok_or_else(|| "revision 不存在或尚未确认。".to_string())?;
    let root = authorized_root_for_path(state, Path::new(&revision.path))?;
    let config_home = get_settings_inner(state)?
        .codex_homes
        .first()
        .map(PathBuf::from);
    let allowed = agents::allowed_names(&agents::load_doc_config(config_home.as_deref()));
    let (root, relative_parent, name) =
        safe_fs::validate_relative_target(&root, Path::new(&revision.path), &allowed)?;
    let current = safe_fs::read_authorized(
        &root,
        &relative_parent,
        &name,
        agents::HARD_AGENTS_READ_LIMIT,
    )?;
    if current.stamp.sha256 != expected_sha256 {
        return Err("文件已被外部修改；请重新加载后再恢复。".into());
    }
    let current_sha = current.stamp.sha256.clone();
    let current_mtime = current.stamp.mtime_ms;
    write_with_revision(
        state,
        root,
        relative_parent,
        name,
        safe_fs::WriteMode::Replace,
        Some(safe_fs::WriteExpectation {
            sha256: &current_sha,
            mtime_ms: current_mtime,
        }),
        revision.before_content,
    )
}

fn revision_public(row: RevisionRow) -> Revision {
    Revision {
        id: row.id,
        path: row.path,
        created_at: row.created_at,
        before_sha256: row.before_sha256,
        after_sha256: row.after_sha256,
        byte_length: row.byte_length,
    }
}

fn recover_pending_revisions(state: &AppState) -> Result<(), String> {
    let pending = state
        .store
        .lock()
        .map_err(lock_err)?
        .pending_revisions()
        .map_err(display_error)?;
    for revision in pending {
        let applied = read_location(state, Path::new(&revision.path))
            .and_then(|location| {
                safe_fs::read_authorized(
                    &location.root,
                    &location.relative_parent,
                    &location.name,
                    agents::HARD_AGENTS_READ_LIMIT,
                )
            })
            .is_ok_and(|read| read.stamp.sha256 == revision.after_sha256);
        let store = state.store.lock().map_err(lock_err)?;
        if applied {
            store.commit_revision(&revision.id).map_err(display_error)?;
        } else {
            store
                .discard_revision(&revision.id)
                .map_err(display_error)?;
        }
    }
    Ok(())
}

fn authorized_root(state: &AppState, requested: &str) -> Result<PathBuf, String> {
    let requested = canonical_directory(Path::new(requested))?;
    get_settings_inner(state)?
        .authorized_roots
        .into_iter()
        .map(PathBuf::from)
        .find(|root| root == &requested)
        .ok_or_else(|| "该根目录不在用户已保存的授权列表中。".into())
}

fn authorized_root_for_path(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "文件缺少父目录。".to_string())?;
    let parent = fs::canonicalize(parent).map_err(display_error)?;
    let mut roots = get_settings_inner(state)?
        .authorized_roots
        .into_iter()
        .map(PathBuf::from)
        .filter(|root| parent.starts_with(root))
        .collect::<Vec<_>>();
    roots.sort_by_key(|root| std::cmp::Reverse(root.components().count()));
    roots
        .into_iter()
        .next()
        .ok_or_else(|| "文件不在用户授权根目录内。".into())
}

fn list_sources_inner(state: &AppState) -> Result<Vec<SourceHealth>, String> {
    let settings = get_settings_inner(state)?;
    let all_rows = state
        .store
        .lock()
        .map_err(lock_err)?
        .source_health()
        .map_err(display_error)?;
    let rows = all_rows
        .into_iter()
        .filter(|row| {
            settings.codex_homes.iter().any(|home| {
                Path::new(&row.canonical_path).starts_with(Path::new(home).join("sessions"))
                    || Path::new(&row.canonical_path)
                        .starts_with(Path::new(home).join("archived_sessions"))
            })
        })
        .collect::<Vec<_>>();
    let last_observed = rows
        .iter()
        .map(|row| row.updated_at.as_str())
        .max()
        .map(str::to_owned);
    let lag_ms = last_observed
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| {
            Utc::now()
                .signed_duration_since(value.with_timezone(&Utc))
                .num_milliseconds()
                .max(0)
        });
    let unparsed = rows
        .iter()
        .map(|row| row.unparsed_events)
        .fold(0_i64, i64::saturating_add);
    let has_error = rows.iter().any(|row| row.last_error.is_some());
    let scan_warning = state.scan_warning.lock().map_err(lock_err)?.clone();
    let project_warning = state.project_warning.lock().map_err(lock_err)?.clone();
    let has_budget_warning = scan_warning.is_some() || project_warning.is_some();
    let rollout_state = if settings.codex_homes.is_empty() {
        "unavailable"
    } else if rows.is_empty() || has_error || has_budget_warning {
        "degraded"
    } else {
        "healthy"
    };
    let rollout_message = if settings.codex_homes.is_empty() {
        "尚未配置可读取的 Codex home。".to_string()
    } else if rows.is_empty() {
        "尚未观察到完整 JSONL 行；不会把缺失指标填成 0。".to_string()
    } else if has_error {
        "至少一个 rollout 源解析异常；详情已脱敏，其他源继续采集。".to_string()
    } else if has_budget_warning {
        [scan_warning, project_warning]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        "增量 checkpoint 正常；仅保存消息长度与使用元数据，不保存正文。".to_string()
    };
    let capability = state.capability.lock().map_err(lock_err)?.clone();
    let app_server = capability.as_ref().map_or(
        SourceHealth {
            id: "app-server-capability".into(),
            kind: "app-server".into(),
            label: "App Server capability".into(),
            state: "unavailable".into(),
            last_observed_at: None,
            lag_ms: None,
            codex_version: None,
            schema_sha256: None,
            unparsed_events: 0,
            message: Some("尚未运行本地 CLI schema 探测；不会附着 Desktop 私有 stdio。".into()),
        },
        |capability| SourceHealth {
            id: "app-server-capability".into(),
            kind: "app-server".into(),
            label: "App Server capability".into(),
            state: if capability.available {
                "degraded"
            } else {
                "unavailable"
            }
            .into(),
            last_observed_at: Some(capability.checked_at.clone()),
            lag_ms: None,
            codex_version: capability.version.clone(),
            schema_sha256: capability.schema_sha256.clone(),
            unparsed_events: i64::from(!capability.available),
            message: capability.message.clone(),
        },
    );
    let otel_guard = state.otel.lock().map_err(lock_err)?;
    let otel_running = otel_guard.as_ref().map(|server| server.port());
    let otel_error = state.otel_error.lock().map_err(lock_err)?.clone();
    let otel_last = state
        .store
        .lock()
        .map_err(lock_err)?
        .otel_last_received()
        .map_err(display_error)?;
    let otel_source = SourceHealth {
        id: "otel-loopback".into(),
        kind: "otel".into(),
        label: "本地 OTLP/HTTP receiver".into(),
        state: if !settings.telemetry_enabled {
            "disabled"
        } else if otel_running.is_some() {
            "healthy"
        } else {
            "degraded"
        }
        .into(),
        last_observed_at: otel_last,
        lag_ms: None,
        codex_version: None,
        schema_sha256: None,
        unparsed_events: i64::from(otel_error.is_some()),
        message: Some(if !settings.telemetry_enabled {
            "未启用，不监听端口，也不会修改 Codex 配置。".into()
        } else if let Some(port) = otel_running {
            format!(
                "仅监听经 TLS 保护的 https://localhost:{port}，并要求专用 header；需用户显式查看配置片段后自行配置 Codex exporter。"
            )
        } else {
            otel_error.unwrap_or_else(|| "接收器未能启动。".into())
        }),
    };
    Ok(vec![
        SourceHealth {
            id: "rollout-local".into(),
            kind: "rollout".into(),
            label: "本地 rollout JSONL".into(),
            state: rollout_state.into(),
            last_observed_at: last_observed,
            lag_ms,
            codex_version: None,
            schema_sha256: None,
            unparsed_events: unparsed,
            message: Some(rollout_message),
        },
        app_server,
        otel_source,
    ])
}

fn probe_codex_inner(state: &AppState) -> Result<Capability, String> {
    let checked_at = Utc::now().to_rfc3339();
    let candidates = platform::codex_candidates();
    if candidates.is_empty() {
        return Ok(Capability {
            executable_path: "codex".into(),
            version: None,
            schema_sha256: None,
            checked_at,
            available: false,
            message: Some("未找到受信任的 Codex CLI 可执行文件。".into()),
        });
    }
    let mut last_failure = None;
    for executable in candidates {
        let version = platform::run_capture(&executable, ["--version"], Duration::from_secs(5))
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                let text = if output.stdout.is_empty() {
                    output.stderr
                } else {
                    output.stdout
                };
                String::from_utf8(text).ok()
            })
            .map(|value| value.trim().chars().take(256).collect::<String>())
            .filter(|value| !value.is_empty());
        let probe_parent = state.data_dir.join("schema-probe");
        fs::create_dir_all(&probe_parent).map_err(display_error)?;
        let directory = tempfile::Builder::new()
            .prefix("probe-")
            .tempdir_in(&probe_parent)
            .map_err(display_error)?;
        let output = directory.path().join("schema");
        fs::create_dir_all(&output).map_err(display_error)?;
        let args = vec![
            "app-server".to_string(),
            "generate-json-schema".to_string(),
            "--out".to_string(),
            output.to_string_lossy().to_string(),
        ];
        match platform::run_quiet_status(&executable, args, Duration::from_secs(20)) {
            Ok(true) => {
                let schema_sha256 = hash_tree(&output)?;
                return Ok(Capability {
                    executable_path: executable.to_string_lossy().to_string(),
                    version,
                    schema_sha256: Some(schema_sha256),
                    checked_at,
                    available: true,
                    message: Some(
                        "已生成与该 CLI 对应的 schema 指纹；仅做能力探测，未连接 Desktop 内部 stdio。"
                            .into(),
                    ),
                });
            }
            Ok(false) => last_failure = Some((executable, version, "schema 生成命令失败".into())),
            Err(error) => last_failure = Some((executable, version, safe_error_label(&error))),
        }
    }
    let (executable, version, reason) = last_failure.expect("non-empty candidates");
    Ok(Capability {
        executable_path: executable.to_string_lossy().to_string(),
        version,
        schema_sha256: None,
        checked_at,
        available: false,
        message: Some(format!(
            "Codex CLI 可执行，但 capability probe 未通过：{reason}"
        )),
    })
}

fn hash_tree(root: &Path) -> Result<String, String> {
    let mut files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect::<Vec<_>>();
    if files.len() > 10_000 {
        return Err("schema probe 产物数量超过安全上限。".into());
    }
    if files.is_empty() {
        return Err("schema probe 未生成任何文件。".into());
    }
    files.sort();
    let mut digest = Sha256::new();
    let mut total_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    for file in files {
        let metadata = fs::symlink_metadata(&file).map_err(display_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("schema probe 生成了不受支持的文件类型。".into());
        }
        if metadata.len() > 16 * 1024 * 1024 {
            return Err("schema probe 单文件超过安全上限。".into());
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "schema probe 产物大小溢出。".to_string())?;
        if total_bytes > MAX_SCHEMA_PROBE_BYTES {
            return Err(format!(
                "schema probe 产物超过 {} bytes 安全上限。",
                MAX_SCHEMA_PROBE_BYTES
            ));
        }
        let relative = file
            .strip_prefix(root)
            .map_err(display_error)?
            .to_string_lossy()
            .replace('\\', "/");
        digest.update(relative.as_bytes());
        digest.update([0]);
        let mut input = File::open(&file).map_err(display_error)?;
        loop {
            let read = input.read(&mut buffer).map_err(display_error)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        digest.update([0]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn initialize_settings(store: &Store) -> Result<(), String> {
    let mut settings = store.settings().map_err(display_error)?;
    if settings.codex_homes.is_empty() {
        settings.codex_homes = default_codex_homes();
    }
    settings.price_catalog_version = pricing::catalog().catalog_version.clone();
    settings.retention_days = settings.retention_days.clamp(1, 3650);
    store.save_settings(&settings).map_err(display_error)
}

fn default_codex_homes() -> Vec<String> {
    BaseDirs::new()
        .map(|base| base.home_dir().join(".codex"))
        .filter(|path| path.is_dir())
        .and_then(|path| canonical_directory(&path).ok())
        .map(|path| vec![path.to_string_lossy().to_string()])
        .unwrap_or_default()
}

fn canonical_directories(values: &[String]) -> Result<Vec<String>, String> {
    if values.len() > 32 {
        return Err("目录列表最多允许 32 项。".into());
    }
    let mut canonical = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            continue;
        }
        canonical.insert(canonical_directory(Path::new(value))?);
    }
    Ok(canonical
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect())
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(display_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("目录无效或为符号链接：{}", path.display()));
    }
    fs::canonicalize(path).map_err(display_error)
}

fn path_id(path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_bytes());
    hex::encode(digest.finalize())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn safe_error_label(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("permission") || lower.contains("权限") {
        "读取权限不足".into()
    } else if lower.contains("utf-8") || lower.contains("json") {
        "事件格式无法解析".into()
    } else if lower.contains("too large") || lower.contains("上限") {
        "事件超过安全上限".into()
    } else {
        "采集源暂时不可用".into()
    }
}

fn lock_err<T>(_: std::sync::PoisonError<T>) -> String {
    "应用内部状态锁异常；请重启应用。".into()
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let directories = ProjectDirs::from("cc", "codex", "Codex Manager")
                .ok_or_else(|| anyhow!("无法确定平台数据目录"))?;
            let data_dir = directories.data_local_dir().to_path_buf();
            fs::create_dir_all(&data_dir).context("创建应用数据目录失败")?;
            let database_path = data_dir.join("codex-manager.db");
            let store = Store::open(&database_path)?;
            initialize_settings(&store).map_err(anyhow::Error::msg)?;
            let settings = store.settings()?;
            let state = AppState {
                store: Mutex::new(store),
                database_path,
                data_dir,
                scan_gate: Mutex::new(()),
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
                auth_profile_revisions: Mutex::new(VecDeque::new()),
            };
            app.manage(state);
            let handle = app.handle().clone();
            recover_pending_revisions(&app.state::<AppState>()).map_err(anyhow::Error::msg)?;
            restart_watcher(&handle).map_err(anyhow::Error::msg)?;
            let state = app.state::<AppState>();
            if settings.telemetry_enabled {
                if let Err(error) = start_otel(&state) {
                    *state
                        .otel_error
                        .lock()
                        .map_err(|_| anyhow!("OTel 状态锁异常"))? = Some(error);
                }
            }
            prune_retention(&state, settings.retention_days).map_err(anyhow::Error::msg)?;
            thread::spawn(move || {
                if let Some(state) = handle.try_state::<AppState>() {
                    if scan_all(&state).is_ok() {
                        let _ = discover_projects_inner(&state);
                        let _ = handle.emit("local-data-refreshed", ());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            get_dashboard,
            list_activity,
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
            get_otel_config,
            update_settings,
            rescan,
            probe_codex,
            check_for_update,
            install_pending_update,
            get_codex_account,
            start_codex_login,
            list_auth_profiles,
            import_auth_profile,
            activate_auth_profile,
            delete_auth_profile,
            restore_auth_profile,
        ])
        .run(tauri::generate_context!())
        .expect("Codex Manager 运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn update_notes_are_plain_text_trimmed_and_bounded() {
        assert_eq!(
            sanitize_update_notes(Some("  release notes  ".into())).as_deref(),
            Some("release notes")
        );
        assert_eq!(sanitize_update_notes(Some("  ".into())), None);
        let oversized = "x".repeat(MAX_UPDATE_NOTES_CHARS + 1);
        let sanitized = sanitize_update_notes(Some(oversized)).unwrap();
        assert_eq!(
            sanitized.chars().filter(|value| *value == 'x').count(),
            MAX_UPDATE_NOTES_CHARS
        );
        assert!(sanitized.ends_with("\n…"));
    }

    #[test]
    fn bounded_reader_preserves_partial_line_and_skips_oversize_line() {
        let mut partial = Cursor::new(b"{\"a\":1}\n{\"b\":2}".to_vec());
        assert!(matches!(
            read_bounded_line(&mut partial, 64, 128).unwrap(),
            BoundedLine::Complete { .. }
        ));
        assert!(matches!(
            read_bounded_line(&mut partial, 64, 128).unwrap(),
            BoundedLine::Partial { consumed: 7 }
        ));

        let mut budgeted = Cursor::new(vec![b'x'; 32]);
        assert!(matches!(
            read_bounded_line(&mut budgeted, 4, 8).unwrap(),
            BoundedLine::BudgetExhausted {
                consumed: 8,
                oversized: true
            }
        ));

        let mut oversize = Cursor::new(b"123456789\nok\n".to_vec());
        assert!(matches!(
            read_bounded_line(&mut oversize, 4, 128).unwrap(),
            BoundedLine::Oversize { consumed: 10 }
        ));
        match read_bounded_line(&mut oversize, 4, 128).unwrap() {
            BoundedLine::Complete { bytes, .. } => assert_eq!(bytes, b"ok"),
            _ => panic!("expected complete line"),
        }
    }

    #[test]
    fn sanitizes_collector_errors_before_health_storage() {
        assert_eq!(
            safe_error_label("/private/project: permission denied"),
            "读取权限不足"
        );
        assert_eq!(safe_error_label("secret-token-123"), "采集源暂时不可用");
    }

    fn rollout_candidate(path: &str, source: RolloutSource, seconds: u64) -> RolloutCandidate {
        RolloutCandidate {
            path: PathBuf::from(path),
            source,
            modified_at: SystemTime::UNIX_EPOCH + Duration::from_secs(seconds),
        }
    }

    #[test]
    fn rollout_candidates_prioritize_active_sessions_over_archives() {
        let mut candidates = vec![
            rollout_candidate(
                "/codex/archived_sessions/rollout-archive.jsonl",
                RolloutSource::ArchivedSessions,
                999,
            ),
            rollout_candidate(
                "/codex/sessions/2026/08/29/rollout-active.jsonl",
                RolloutSource::ActiveSessions,
                1,
            ),
        ];

        sort_rollout_candidates(&mut candidates);

        assert_eq!(candidates[0].source, RolloutSource::ActiveSessions);
        assert_eq!(
            candidates[0].path,
            PathBuf::from("/codex/sessions/2026/08/29/rollout-active.jsonl")
        );
    }

    #[test]
    fn rollout_candidates_sort_newest_first_with_stable_path_tiebreaker() {
        let mut candidates = vec![
            rollout_candidate(
                "/codex/sessions/rollout-z.jsonl",
                RolloutSource::ActiveSessions,
                10,
            ),
            rollout_candidate(
                "/codex/sessions/rollout-old.jsonl",
                RolloutSource::ActiveSessions,
                1,
            ),
            rollout_candidate(
                "/codex/sessions/rollout-new.jsonl",
                RolloutSource::ActiveSessions,
                20,
            ),
            rollout_candidate(
                "/codex/sessions/rollout-a.jsonl",
                RolloutSource::ActiveSessions,
                10,
            ),
        ];

        sort_rollout_candidates(&mut candidates);

        let paths = candidates
            .iter()
            .map(|candidate| candidate.path.as_path())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                Path::new("/codex/sessions/rollout-new.jsonl"),
                Path::new("/codex/sessions/rollout-a.jsonl"),
                Path::new("/codex/sessions/rollout-z.jsonl"),
                Path::new("/codex/sessions/rollout-old.jsonl"),
            ]
        );
    }

    #[test]
    fn scanner_resumes_after_partial_line_without_double_counting() {
        let directory = tempfile::tempdir().unwrap();
        let codex_home = directory.path().join("codex-home");
        let sessions = codex_home.join("sessions/2026/08/27");
        let project = directory.path().join("project");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&project).unwrap();
        let codex_home = fs::canonicalize(codex_home).unwrap();
        let sessions = codex_home.join("sessions/2026/08/27");
        let project = fs::canonicalize(project).unwrap();
        let database_path = directory.path().join("manager.db");
        let store = Store::open(&database_path).unwrap();
        store
            .save_settings(&SettingsRow {
                codex_homes: vec![codex_home.to_string_lossy().to_string()],
                authorized_roots: vec![project.to_string_lossy().to_string()],
                retention_days: 30,
                telemetry_enabled: false,
                price_catalog_version: pricing::catalog().catalog_version.clone(),
            })
            .unwrap();
        let state = AppState {
            store: Mutex::new(store),
            database_path,
            data_dir: directory.path().join("data"),
            scan_gate: Mutex::new(()),
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
            auth_profile_revisions: Mutex::new(VecDeque::new()),
        };
        let event = |ordinal: i64, kind: &str, payload: serde_json::Value| {
            serde_json::json!({
                "timestamp": format!("2026-08-27T00:00:{ordinal:02}Z"),
                "ordinal": ordinal,
                "type": kind,
                "payload": payload,
            })
            .to_string()
        };
        let usage = |total: i64| {
            serde_json::json!({
                "input_tokens": total,
                "cached_input_tokens": 0,
                "cache_write_input_tokens": 0,
                "output_tokens": 0,
                "reasoning_output_tokens": 0,
                "total_tokens": total,
            })
        };
        let complete = [
            event(
                0,
                "session_meta",
                serde_json::json!({"id":"session-a","cwd":project,"model_provider":"openai"}),
            ),
            event(
                1,
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"turn-a"}),
            ),
            event(
                2,
                "turn_context",
                serde_json::json!({"turn_id":"turn-a","model":"gpt-5.6-sol","effort":"high","cwd":project}),
            ),
            event(
                3,
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(10),"last_token_usage":usage(10)}}),
            ),
        ]
        .join("\n");
        let partial = event(
            4,
            "event_msg",
            serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(16),"last_token_usage":usage(6)}}),
        );
        let rollout = sessions.join("rollout.jsonl");
        fs::write(&rollout, format!("{complete}\n{partial}")).unwrap();

        scan_file_unlocked(&state, &rollout, &mut ScanBudget::new()).unwrap();
        let first = state
            .store
            .lock()
            .unwrap()
            .page_activity(&ActivityFilter::default())
            .unwrap();
        assert_eq!(first.total, 1);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&rollout)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        scan_file_unlocked(&state, &rollout, &mut ScanBudget::new()).unwrap();
        scan_file_unlocked(&state, &rollout, &mut ScanBudget::new()).unwrap();
        let resumed = state
            .store
            .lock()
            .unwrap()
            .page_activity(&ActivityFilter::default())
            .unwrap();
        assert_eq!(resumed.total, 2);
        assert_eq!(
            state
                .store
                .lock()
                .unwrap()
                .summary()
                .unwrap()
                .3
                .total_tokens,
            16
        );
    }

    #[test]
    fn agents_service_creates_saves_restores_and_rejects_stale_edits() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let codex_home = root.join("codex-home");
        let project = root.join("project");
        fs::create_dir_all(&codex_home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let database_path = root.join("manager.db");
        let store = Store::open(&database_path).unwrap();
        store
            .save_settings(&SettingsRow {
                codex_homes: vec![codex_home.to_string_lossy().to_string()],
                authorized_roots: vec![root.to_string_lossy().to_string()],
                retention_days: 30,
                telemetry_enabled: false,
                price_catalog_version: pricing::catalog().catalog_version.clone(),
            })
            .unwrap();
        store
            .upsert_project(&ProjectRow {
                canonical_path: project.to_string_lossy().to_string(),
                name: "project".into(),
                source: "manual".into(),
                exists: true,
                is_git: false,
                worktree: false,
                last_seen_at: None,
            })
            .unwrap();
        let state = AppState {
            store: Mutex::new(store),
            database_path,
            data_dir: root.join("data"),
            scan_gate: Mutex::new(()),
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
            auth_profile_revisions: Mutex::new(VecDeque::new()),
        };

        let created = create_agents_inner(
            &state,
            CreateInput {
                project_path: project.to_string_lossy().to_string(),
                authorized_root: root.to_string_lossy().to_string(),
                content: "one\n".into(),
                file_name: "AGENTS.md".into(),
            },
        )
        .unwrap();
        assert_eq!(created.snapshot.content, "one\n");
        let saved = save_agents_inner(
            &state,
            SaveInput {
                path: created.snapshot.summary.path,
                authorized_root: root.to_string_lossy().to_string(),
                content: "two\n".into(),
                expected_sha256: created.snapshot.summary.sha256,
                expected_mtime_ms: created.snapshot.summary.mtime_ms,
            },
        )
        .unwrap();
        assert_eq!(saved.snapshot.content, "two\n");

        let restored =
            restore_agents_inner(&state, &saved.revision_id, &saved.snapshot.summary.sha256)
                .unwrap();
        assert_eq!(restored.snapshot.content, "one\n");
        fs::write(&restored.snapshot.summary.path, "outside\n").unwrap();
        let stale = match save_agents_inner(
            &state,
            SaveInput {
                path: restored.snapshot.summary.path,
                authorized_root: root.to_string_lossy().to_string(),
                content: "three\n".into(),
                expected_sha256: restored.snapshot.summary.sha256,
                expected_mtime_ms: restored.snapshot.summary.mtime_ms,
            },
        ) {
            Ok(_) => panic!("stale save unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(stale.contains("外部修改"));
    }
}
