//! Bounded Codex account integration.
//!
//! The official Codex CLI/App Server owns login, refresh and account validation.
//! This module exposes a bounded account snapshot and an internal isolated
//! validator for explicitly imported Keychain-backed profiles. Raw JSON-RPC
//! responses, tokens and authorization URLs are never returned to the WebView.

use crate::{auth_profiles, platform};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::Child,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_APP_SERVER_LINE_BYTES: usize = 256 * 1024;
const MAX_APP_SERVER_MESSAGES: usize = 128;
const MAX_RATE_LIMIT_BUCKETS: usize = 16;
const LOGIN_SUPERVISOR_INTERVAL: Duration = Duration::from_millis(250);
const MAX_AUTH_FILE_BYTES: u64 = 128 * 1024;
const ACCOUNT_CACHE_KEYCHAIN_SERVICE: &str = "cc.codex.manager.account-snapshot.v1";
const ACCOUNT_CACHE_KEYCHAIN_ACCOUNT: &str = "active";
const MAX_ACCOUNT_CACHE_BYTES: usize = 64 * 1024;
const ACCOUNT_CACHE_FRESH_HOURS: i64 = 6;

pub struct ValidatedAuthMaterial {
    pub bytes: Zeroizing<Vec<u8>>,
    pub identity_fingerprint: String,
}

#[derive(Clone, Copy)]
struct ExternalAuthMaterial<'a> {
    access_token: &'a str,
    account_id: &'a str,
}

#[derive(Serialize)]
struct ExternalLoginRequest<'a> {
    method: &'static str,
    id: i64,
    params: ExternalLoginParams<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalLoginParams<'a> {
    #[serde(rename = "type")]
    login_type: &'static str,
    access_token: &'a str,
    chatgpt_account_id: &'a str,
    chatgpt_plan_type: Option<&'static str>,
}

#[derive(Default)]
pub struct LoginRuntime {
    process: Option<LoginProcess>,
}

struct LoginProcess {
    active: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    supervisor: Option<thread::JoinHandle<()>>,
}

impl LoginRuntime {
    pub fn in_progress(&mut self) -> bool {
        let Some(process) = self.process.as_ref() else {
            return false;
        };
        if process.active.load(Ordering::Acquire) {
            return true;
        }
        self.finish_supervisor();
        false
    }

    fn finish_supervisor(&mut self) {
        if let Some(mut process) = self.process.take()
            && let Some(supervisor) = process.supervisor.take()
        {
            let _ = supervisor.join();
        }
    }
}

impl Drop for LoginRuntime {
    fn drop(&mut self) {
        if let Some(process) = self.process.as_ref() {
            process.cancel.store(true, Ordering::Release);
        }
        self.finish_supervisor();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStartResult {
    started: bool,
    login_in_progress: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    pub state: String,
    pub authenticated: bool,
    pub auth_method: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub requires_openai_auth: Option<bool>,
    pub login_in_progress: bool,
    pub rate_limits_available: bool,
    pub rate_limits: Vec<RateLimitBucket>,
    pub available_reset_credits: Option<i64>,
    pub checked_at: String,
    /// `live`, `cache`, `stale`, or `unavailable`. Never derived from an
    /// App Server response; this is local provenance only.
    pub source: String,
    pub stale: bool,
    pub cached_at: Option<String>,
    pub message: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitBucket {
    limit_id: String,
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary: Option<RateLimitWindow>,
    secondary: Option<RateLimitWindow>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    used_percent: f64,
    window_duration_mins: Option<i64>,
    resets_at: Option<String>,
}

pub fn start_login<L, F>(
    runtime: &mut LoginRuntime,
    executable: Option<&Path>,
    acquire_credential_lock: F,
) -> Result<LoginStartResult, String>
where
    L: Send + 'static,
    F: FnOnce() -> Result<L, String>,
{
    if runtime.in_progress() {
        return Ok(LoginStartResult {
            started: false,
            login_in_progress: true,
        });
    }
    let executable =
        executable.ok_or_else(|| "未找到通过签名验证的官方 Codex CLI。".to_string())?;
    let credential_lock = acquire_credential_lock()?;
    let child = platform::spawn_codex_login(executable)
        .map_err(|_| "无法启动官方 Codex 登录流程，请确认 Codex CLI 可用。".to_string())?;
    let active = Arc::new(AtomicBool::new(true));
    let cancel = Arc::new(AtomicBool::new(false));
    let supervisor_active = Arc::clone(&active);
    let supervisor_cancel = Arc::clone(&cancel);
    let supervisor = thread::spawn(move || {
        let _credential_lock = credential_lock;
        supervise_login_process(child, supervisor_active, supervisor_cancel, LOGIN_TIMEOUT)
    });
    runtime.process = Some(LoginProcess {
        active,
        cancel,
        supervisor: Some(supervisor),
    });
    Ok(LoginStartResult {
        started: true,
        login_in_progress: true,
    })
}

pub fn read_account(
    executable: Option<&Path>,
    login_in_progress: bool,
    refresh: bool,
) -> AccountSnapshot {
    let cache = system_account_cache();
    read_account_with_cache(executable, login_in_progress, refresh, cache.as_ref())
}

/// Read the sanitized Keychain snapshot without locating or launching Codex.
/// This keeps the subscription page available when the trusted CLI is
/// temporarily missing or cannot be revalidated.
pub fn read_cached_account(login_in_progress: bool) -> Option<AccountSnapshot> {
    let cache = system_account_cache();
    cache
        .read()
        .ok()
        .flatten()
        .and_then(parse_cached_snapshot)
        .map(|snapshot| cached_snapshot(&snapshot, false, login_in_progress))
}

/// Cache-first account read. The cache contains only a sanitized DTO and is
/// intentionally independent from Codex credentials. On unsupported systems
/// cache operations fail closed, while the trusted live read remains usable.
fn read_account_with_cache(
    executable: Option<&Path>,
    login_in_progress: bool,
    refresh: bool,
    cache: &dyn AccountSnapshotCache,
) -> AccountSnapshot {
    read_account_with_cache_and_live(
        executable,
        login_in_progress,
        refresh,
        cache,
        read_live_account,
    )
}

fn read_account_with_cache_and_live<F>(
    executable: Option<&Path>,
    login_in_progress: bool,
    refresh: bool,
    cache: &dyn AccountSnapshotCache,
    live_read: F,
) -> AccountSnapshot
where
    F: FnOnce(Option<&Path>, bool) -> Result<AccountSnapshot, String>,
{
    let cached = cache.read().ok().flatten().and_then(parse_cached_snapshot);
    if !refresh {
        if let Some(snapshot) = cached.as_ref() {
            return cached_snapshot(snapshot, false, login_in_progress);
        }
    }

    match live_read(executable, login_in_progress) {
        Ok(mut snapshot) => {
            if snapshot.state != "unavailable" {
                let cached_at = Utc::now().to_rfc3339();
                snapshot.cached_at = Some(cached_at);
                if let Ok(serialized) = serde_json::to_vec(&snapshot)
                    && serialized.len() <= MAX_ACCOUNT_CACHE_BYTES
                    && cache.write(&serialized).is_err()
                {
                    // The live response is still trustworthy, but never
                    // claim a persisted cache timestamp when Keychain write
                    // failed or is unsupported.
                    snapshot.cached_at = None;
                }
            }
            snapshot
        }
        Err(_) if cached.is_some() => {
            cached_snapshot(cached.as_ref().expect("checked"), true, login_in_progress)
        }
        Err(_) => unavailable_snapshot(
            login_in_progress,
            "无法通过官方 Codex App Server 读取账户状态；未读取或回退解析 auth.json。",
        ),
    }
}

fn read_live_account(
    executable: Option<&Path>,
    login_in_progress: bool,
) -> Result<AccountSnapshot, String> {
    let executable = executable.ok_or_else(|| {
        "未找到通过 OpenAI Developer ID 验证的 Codex 可执行文件；OAuth 功能已安全停用。".to_string()
    })?;
    let (account_response, rate_limits_response) =
        app_server_snapshot(executable, None, None, false, true, false)?;
    Ok(parse_snapshot(
        &account_response,
        rate_limits_response.as_ref(),
        login_in_progress,
    ))
}

fn unavailable_snapshot(login_in_progress: bool, message: &str) -> AccountSnapshot {
    AccountSnapshot {
        state: "unavailable".into(),
        authenticated: false,
        auth_method: None,
        email: None,
        plan_type: None,
        requires_openai_auth: None,
        login_in_progress,
        rate_limits_available: false,
        rate_limits: Vec::new(),
        available_reset_credits: None,
        checked_at: Utc::now().to_rfc3339(),
        source: "unavailable".into(),
        stale: false,
        cached_at: None,
        message: message.into(),
    }
}

trait AccountSnapshotCache: Send + Sync {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ()>;
    fn write(&self, value: &[u8]) -> Result<(), ()>;
}

#[cfg(target_os = "macos")]
struct MacAccountSnapshotCache;

#[cfg(target_os = "macos")]
impl AccountSnapshotCache for MacAccountSnapshotCache {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
        match security_framework::passwords::get_generic_password(
            ACCOUNT_CACHE_KEYCHAIN_SERVICE,
            ACCOUNT_CACHE_KEYCHAIN_ACCOUNT,
        ) {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(()),
        }
    }

    fn write(&self, value: &[u8]) -> Result<(), ()> {
        security_framework::passwords::set_generic_password(
            ACCOUNT_CACHE_KEYCHAIN_SERVICE,
            ACCOUNT_CACHE_KEYCHAIN_ACCOUNT,
            value,
        )
        .map_err(|_| ())
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedAccountSnapshotCache;

#[cfg(not(target_os = "macos"))]
impl AccountSnapshotCache for UnsupportedAccountSnapshotCache {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
        Err(())
    }

    fn write(&self, _: &[u8]) -> Result<(), ()> {
        Err(())
    }
}

fn system_account_cache() -> Box<dyn AccountSnapshotCache> {
    #[cfg(target_os = "macos")]
    {
        Box::new(MacAccountSnapshotCache)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedAccountSnapshotCache)
    }
}

fn parse_cached_snapshot(bytes: Zeroizing<Vec<u8>>) -> Option<AccountSnapshot> {
    if bytes.len() > MAX_ACCOUNT_CACHE_BYTES {
        return None;
    }
    let snapshot = serde_json::from_slice::<AccountSnapshot>(&bytes).ok()?;
    validate_cached_snapshot(snapshot)
}

fn validate_cached_snapshot(mut snapshot: AccountSnapshot) -> Option<AccountSnapshot> {
    if !matches!(snapshot.state.as_str(), "authenticated" | "signed-out")
        || snapshot.rate_limits.len() > MAX_RATE_LIMIT_BUCKETS
        || DateTime::parse_from_rfc3339(&snapshot.checked_at).is_err()
        || snapshot
            .cached_at
            .as_deref()
            .is_none_or(|value| DateTime::parse_from_rfc3339(value).is_err())
    {
        return None;
    }
    snapshot.auth_method = snapshot
        .auth_method
        .as_deref()
        .and_then(|value| sanitize_identifier(value, 64));
    snapshot.email = snapshot
        .email
        .as_deref()
        .and_then(|value| sanitize_display(value, 320));
    snapshot.plan_type = snapshot
        .plan_type
        .as_deref()
        .and_then(|value| sanitize_identifier(value, 64));
    if snapshot.authenticated && snapshot.auth_method.is_none() {
        return None;
    }
    if snapshot.message.chars().count() > 512 || snapshot.message.chars().any(char::is_control) {
        return None;
    }
    for bucket in &snapshot.rate_limits {
        if sanitize_identifier(&bucket.limit_id, 80).is_none()
            || bucket
                .limit_name
                .as_deref()
                .is_some_and(|value| sanitize_display(value, 100).is_none())
            || bucket
                .plan_type
                .as_deref()
                .is_some_and(|value| sanitize_identifier(value, 64).is_none())
            || !valid_window(bucket.primary.as_ref())
            || !valid_window(bucket.secondary.as_ref())
        {
            return None;
        }
    }
    snapshot.source = "cache".into();
    snapshot.stale = false;
    Some(snapshot)
}

fn valid_window(window: Option<&RateLimitWindow>) -> bool {
    window.is_none_or(|window| {
        window.used_percent.is_finite()
            && (0.0..=10_000.0).contains(&window.used_percent)
            && window
                .window_duration_mins
                .is_none_or(|duration| (1..=525_600).contains(&duration))
            && window
                .resets_at
                .as_deref()
                .is_none_or(|value| DateTime::parse_from_rfc3339(value).is_ok())
    })
}

fn cached_snapshot(
    snapshot: &AccountSnapshot,
    refresh_failed: bool,
    login_in_progress: bool,
) -> AccountSnapshot {
    let mut snapshot = snapshot.clone();
    let expired = account_cache_is_expired(&snapshot);
    let stale = refresh_failed || expired;
    snapshot.source = if stale { "stale" } else { "cache" }.into();
    snapshot.stale = stale;
    snapshot.login_in_progress = login_in_progress;
    snapshot.message = if refresh_failed {
        "实时读取失败；显示上次确认的本地账户与额度快照。"
    } else if expired {
        "本地账户与额度快照已超过 6 小时；请使用“刷新状态”重新确认。"
    } else {
        "显示本地保存的上次确认账户与额度快照；使用“刷新状态”获取实时数据。"
    }
    .into();
    snapshot
}

fn account_cache_is_expired(snapshot: &AccountSnapshot) -> bool {
    let timestamp = DateTime::parse_from_rfc3339(
        snapshot
            .cached_at
            .as_deref()
            .unwrap_or(&snapshot.checked_at),
    )
    .ok();
    let Some(timestamp) = timestamp else {
        return true;
    };
    let age = Utc::now().signed_duration_since(timestamp.with_timezone(&Utc));
    age < TimeDelta::zero() || age > TimeDelta::hours(ACCOUNT_CACHE_FRESH_HOURS)
}

fn supervise_login_process(
    mut child: Child,
    active: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(LOGIN_SUPERVISOR_INTERVAL);
    }
    active.store(false, Ordering::Release);
}

fn app_server_snapshot(
    executable: &Path,
    codex_home: Option<&Path>,
    external_auth: Option<ExternalAuthMaterial<'_>>,
    refresh_token: bool,
    include_rate_limits: bool,
    require_file_credentials_store: bool,
) -> Result<(Value, Option<Value>), String> {
    let mut child = platform::spawn_app_server_with_home(executable, codex_home)
        .map_err(|_| "无法启动 Codex App Server。".to_string())?;
    child
        .resume()
        .map_err(|_| "无法恢复已验签 Codex App Server。".to_string())?;
    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| "Codex App Server stdin 不可用。".to_string())?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| "Codex App Server stdout 不可用。".to_string())?;
    let (sender, receiver) = mpsc::sync_channel(16);
    let (reader_done_sender, reader_done_receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        for _ in 0..MAX_APP_SERVER_MESSAGES {
            let result = read_bounded_json_line(&mut stdout);
            let terminal = result.as_ref().is_err_and(|_| true)
                || result.as_ref().is_ok_and(|value| value.is_none());
            if sender.send(result).is_err() || terminal {
                let _ = reader_done_sender.send(());
                return;
            }
        }
        let _ = sender.send(Err("Codex App Server 响应数量超过安全上限。".into()));
        let _ = reader_done_sender.send(());
    });

    let initialize = json!({
        "method": "initialize",
        "id": 0,
        "params": {
            "clientInfo": {
                "name": "codex_manager",
                "title": "Codex Manager",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": external_auth.is_some()
            }
        }
    });
    let initialized = json!({ "method": "initialized", "params": {} });
    let mut requests = vec![json!({
        "method": "account/read",
        "id": 1,
        "params": { "refreshToken": refresh_token }
    })];
    if include_rate_limits {
        requests.push(json!({ "method": "account/rateLimits/read", "id": 2 }));
    }
    if require_file_credentials_store {
        requests.push(json!({
            "method": "config/read",
            "id": 3,
            "params": { "cwd": null, "includeLayers": false }
        }));
    }
    let result = (|| -> Result<(Value, Option<Value>), String> {
        write_json_request(&mut stdin, &initialize)?;
        write_json_request(&mut stdin, &initialized)?;
        if let Some(external_auth) = external_auth {
            write_external_login_request(&mut stdin, external_auth)?;
            stdin
                .flush()
                .map_err(|_| "无法提交 Codex App Server 外部认证请求。".to_string())?;
            wait_for_external_login(&receiver, Instant::now() + APP_SERVER_TIMEOUT)?;
        }
        for request in requests {
            write_json_request(&mut stdin, &request)?;
        }
        stdin
            .flush()
            .map_err(|_| "无法提交 Codex App Server 请求。".to_string())?;

        let deadline = Instant::now() + APP_SERVER_TIMEOUT;
        let mut account = None;
        let mut rate_limits = None;
        let mut config = None;
        while Instant::now() < deadline
            && (account.is_none()
                || (include_rate_limits && rate_limits.is_none())
                || (require_file_credentials_store && config.is_none()))
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match receiver.recv_timeout(remaining) {
                Ok(Ok(Some(message))) => match message.get("id").and_then(Value::as_i64) {
                    Some(1) => account = Some(message),
                    Some(2) => rate_limits = Some(message),
                    Some(3) => config = Some(message),
                    _ => {}
                },
                Ok(Ok(None)) | Ok(Err(_)) | Err(_) => break,
            }
        }

        let account = account.ok_or_else(|| "Codex App Server 未及时返回账户状态。".to_string())?;
        if require_file_credentials_store
            && config
                .as_ref()
                .and_then(|response| response.get("result"))
                .and_then(|result| result.get("config"))
                .and_then(|config| config.get("cli_auth_credentials_store"))
                .and_then(Value::as_str)
                != Some("file")
        {
            return Err("Codex 当前生效的认证存储不是 file 模式。".into());
        }
        Ok((account, rate_limits))
    })();

    child.stop();
    drop(stdin);
    drop(receiver);
    if reader_done_receiver
        .recv_timeout(Duration::from_secs(1))
        .is_ok()
    {
        let _ = reader.join();
    }
    result
}

fn write_json_request<W: Write, T: Serialize>(writer: &mut W, request: &T) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, request)
        .map_err(|_| "无法编码 Codex App Server 请求。".to_string())?;
    writer
        .write_all(b"\n")
        .map_err(|_| "无法写入 Codex App Server 请求。".to_string())
}

fn write_external_login_request<W: Write>(
    writer: &mut W,
    auth: ExternalAuthMaterial<'_>,
) -> Result<(), String> {
    let request = ExternalLoginRequest {
        method: "account/login/start",
        id: 4,
        params: ExternalLoginParams {
            login_type: "chatgptAuthTokens",
            access_token: auth.access_token,
            chatgpt_account_id: auth.account_id,
            chatgpt_plan_type: None,
        },
    };
    let mut encoded = Zeroizing::new(Vec::new());
    serde_json::to_writer(&mut *encoded, &request)
        .map_err(|_| "无法编码 Codex App Server 外部认证请求。".to_string())?;
    encoded.push(b'\n');
    writer
        .write_all(&encoded)
        .map_err(|_| "无法写入 Codex App Server 外部认证请求。".to_string())
}

fn wait_for_external_login(
    receiver: &mpsc::Receiver<Result<Option<Value>, String>>,
    deadline: Instant,
) -> Result<(), String> {
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(Ok(Some(message))) if message.get("id").and_then(Value::as_i64) == Some(4) => {
                if message.get("error").is_some() || message.get("result").is_none() {
                    return Err("官方 Codex 拒绝了该外部认证材料。".into());
                }
                return Ok(());
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) | Ok(Err(_)) | Err(_) => break,
        }
    }
    Err("官方 Codex 未及时完成外部认证。".into())
}

#[cfg(target_os = "macos")]
pub fn validate_auth_material(
    executable: &Path,
    candidate: &[u8],
) -> Result<ValidatedAuthMaterial, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if candidate.len() as u64 > MAX_AUTH_FILE_BYTES {
        return Err("认证文件超过 128 KiB 安全上限。".into());
    }
    auth_profiles::validate_structure(candidate).map_err(|error| error.to_string())?;
    let external_auth = extract_external_auth(candidate)?;
    let directory = tempfile::Builder::new()
        .prefix("codex-manager-auth-validate-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|_| "无法创建认证文件隔离验证目录。".to_string())?;
    let config_path = directory.path().join("config.toml");
    let mut config_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&config_path)
        .map_err(|_| "无法创建隔离认证配置。".to_string())?;
    config_file
        .write_all(b"cli_auth_credentials_store = \"file\"\n")
        .and_then(|_| config_file.sync_all())
        .map_err(|_| "无法写入隔离认证配置。".to_string())?;
    drop(config_file);
    let (account_response, rate_limits_response) = app_server_snapshot(
        executable,
        Some(directory.path()),
        Some(external_auth),
        false,
        true,
        true,
    )
    .map_err(|_| "官方 Codex 无法验证该认证文件。".to_string())?;
    require_authenticated_rate_limits(rate_limits_response.as_ref())?;
    let identity_fingerprint = verified_identity_fingerprint(&account_response, candidate)?;
    Ok(ValidatedAuthMaterial {
        bytes: Zeroizing::new(candidate.to_vec()),
        identity_fingerprint,
    })
}

pub fn read_active_identity(executable: &Path, active_auth: &[u8]) -> Result<String, String> {
    auth_profiles::validate_structure(active_auth)
        .map_err(|_| "当前 file 模式认证文件结构无效。".to_string())?;
    let (account_response, _) = app_server_snapshot(executable, None, None, false, false, true)
        .map_err(|_| "官方 Codex 无法验证当前活动账户。".to_string())?;
    verified_identity_fingerprint(&account_response, active_auth)
}

#[derive(Deserialize)]
struct AuthIdentity<'a> {
    #[serde(borrow)]
    tokens: AuthIdentityTokens<'a>,
}

#[derive(Deserialize)]
struct AuthIdentityTokens<'a> {
    #[serde(borrow)]
    access_token: Option<&'a str>,
    #[serde(borrow)]
    account_id: Option<&'a str>,
    #[serde(borrow)]
    chatgpt_account_id: Option<&'a str>,
}

fn extract_external_auth(auth_material: &[u8]) -> Result<ExternalAuthMaterial<'_>, String> {
    let auth: AuthIdentity<'_> = serde_json::from_slice(auth_material)
        .map_err(|_| "认证文件缺少可用的 ChatGPT token。".to_string())?;
    let access_token = auth
        .tokens
        .access_token
        .and_then(|value| validated_secret_component(value, 16 * 1024))
        .ok_or_else(|| "认证文件缺少可用的 access token。".to_string())?;
    let account_id = auth
        .tokens
        .account_id
        .or(auth.tokens.chatgpt_account_id)
        .and_then(|value| validated_secret_component(value, 320))
        .ok_or_else(|| "认证文件缺少工作区身份。".to_string())?;
    Ok(ExternalAuthMaterial {
        access_token,
        account_id,
    })
}

fn verified_identity_fingerprint(
    account_response: &Value,
    auth_material: &[u8],
) -> Result<String, String> {
    let account = account_response
        .get("result")
        .and_then(|value| value.get("account"))
        .filter(|value| !value.is_null())
        .ok_or_else(|| "该文件未包含可用的 ChatGPT OAuth 账户。".to_string())?;
    if account.get("type").and_then(Value::as_str) != Some("chatgpt") {
        return Err("仅支持 ChatGPT OAuth 认证文件。".into());
    }
    let email = account
        .get("email")
        .and_then(Value::as_str)
        .and_then(|value| sanitize_display(value, 320))
        .ok_or_else(|| "官方 Codex 未返回可识别的账户身份。".to_string())?;
    let auth: AuthIdentity<'_> = serde_json::from_slice(auth_material)
        .map_err(|_| "经验证的认证文件缺少工作区身份。".to_string())?;
    let account_id = auth
        .tokens
        .account_id
        .or(auth.tokens.chatgpt_account_id)
        .and_then(|value| validated_secret_component(value, 320))
        .ok_or_else(|| "经验证的认证文件缺少工作区身份。".to_string())?;
    let mut digest = Sha256::new();
    digest.update(b"codex-manager-auth-profile-v1\0");
    digest.update(email.trim().to_ascii_lowercase().as_bytes());
    digest.update(b"\0");
    digest.update(account_id.as_bytes());
    Ok(hex::encode(digest.finalize()))
}

fn require_authenticated_rate_limits(response: Option<&Value>) -> Result<(), String> {
    if response
        .and_then(|response| response.get("result"))
        .is_some()
    {
        Ok(())
    } else {
        Err("官方 Codex 无法用该认证文件读取账户额度。".into())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn validate_auth_material(
    _executable: &Path,
    _candidate: &[u8],
) -> Result<ValidatedAuthMaterial, String> {
    Err("认证档案当前仅支持 macOS。".into())
}

fn read_bounded_json_line<R: BufRead>(reader: &mut R) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    let read = reader
        .take((MAX_APP_SERVER_LINE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "读取 Codex App Server 响应失败。".to_string())?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_APP_SERVER_LINE_BYTES {
        return Err("Codex App Server 单条响应超过安全上限。".into());
    }
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Err("Codex App Server 返回空响应。".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "Codex App Server 返回无法解析的响应。".to_string())
}

fn parse_snapshot(
    account_response: &Value,
    rate_limits_response: Option<&Value>,
    login_in_progress: bool,
) -> AccountSnapshot {
    let checked_at = Utc::now().to_rfc3339();
    let Some(result) = account_response.get("result") else {
        return AccountSnapshot {
            state: "unavailable".into(),
            authenticated: false,
            auth_method: None,
            email: None,
            plan_type: None,
            requires_openai_auth: None,
            login_in_progress,
            rate_limits_available: false,
            rate_limits: Vec::new(),
            available_reset_credits: None,
            checked_at,
            source: "live".into(),
            stale: false,
            cached_at: None,
            message: "Codex App Server 未返回可识别的账户状态。".into(),
        };
    };
    let requires_openai_auth = result.get("requiresOpenaiAuth").and_then(Value::as_bool);
    let account = result.get("account").filter(|value| !value.is_null());
    let auth_method = account
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .and_then(|value| sanitize_identifier(value, 64));
    let authenticated = account.is_some();
    let state = if authenticated {
        "authenticated"
    } else if requires_openai_auth == Some(true) {
        "signed-out"
    } else {
        "unavailable"
    };
    let email = account
        .and_then(|value| value.get("email"))
        .and_then(Value::as_str)
        .and_then(|value| sanitize_display(value, 320));
    let plan_type = account
        .and_then(|value| value.get("planType"))
        .and_then(Value::as_str)
        .and_then(|value| sanitize_identifier(value, 64));
    let (rate_limits_available, rate_limits, available_reset_credits) =
        parse_rate_limits(rate_limits_response);
    let message = match (authenticated, auth_method.as_deref()) {
        (true, Some("chatgpt")) => {
            "账户与额度来自官方 Codex App Server；当前活动凭据由 Codex 的配置存储持有。"
        }
        (true, Some("apiKey")) => "当前使用 API Key 登录；Codex Manager 不读取或显示该密钥。",
        (true, _) => "已检测到 Codex 认证；活动凭据内容不会返回界面或写入数据库。",
        (false, _) if login_in_progress => "官方 Codex 登录流程正在等待浏览器完成授权。",
        (false, _) => "当前未检测到可用的 Codex 登录。",
    }
    .to_string();
    AccountSnapshot {
        state: state.into(),
        authenticated,
        auth_method,
        email,
        plan_type,
        requires_openai_auth,
        login_in_progress,
        rate_limits_available,
        rate_limits,
        available_reset_credits,
        checked_at,
        source: "live".into(),
        stale: false,
        cached_at: None,
        message,
    }
}

fn parse_rate_limits(response: Option<&Value>) -> (bool, Vec<RateLimitBucket>, Option<i64>) {
    let Some(result) = response.and_then(|value| value.get("result")) else {
        return (false, Vec::new(), None);
    };
    let mut buckets = Vec::new();
    if let Some(values) = result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        let mut entries = values.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(right.0));
        for (fallback_id, value) in entries.into_iter().take(MAX_RATE_LIMIT_BUCKETS) {
            if let Some(bucket) = parse_rate_limit_bucket(value, fallback_id) {
                buckets.push(bucket);
            }
        }
    } else if let Some(value) = result.get("rateLimits")
        && let Some(bucket) = parse_rate_limit_bucket(value, "codex")
    {
        buckets.push(bucket);
    }
    let available_reset_credits = result
        .get("rateLimitResetCredits")
        .and_then(|value| value.get("availableCount"))
        .and_then(Value::as_i64)
        .filter(|value| (0..=100_000).contains(value));
    (true, buckets, available_reset_credits)
}

fn parse_rate_limit_bucket(value: &Value, fallback_id: &str) -> Option<RateLimitBucket> {
    let limit_id = value
        .get("limitId")
        .and_then(Value::as_str)
        .and_then(|value| sanitize_identifier(value, 80))
        .or_else(|| sanitize_identifier(fallback_id, 80))?;
    Some(RateLimitBucket {
        limit_id,
        limit_name: value
            .get("limitName")
            .and_then(Value::as_str)
            .and_then(|value| sanitize_display(value, 100)),
        plan_type: value
            .get("planType")
            .and_then(Value::as_str)
            .and_then(|value| sanitize_identifier(value, 64)),
        primary: value.get("primary").and_then(parse_rate_limit_window),
        secondary: value.get("secondary").and_then(parse_rate_limit_window),
    })
}

fn parse_rate_limit_window(value: &Value) -> Option<RateLimitWindow> {
    if value.is_null() {
        return None;
    }
    let used_percent = value.get("usedPercent")?.as_f64()?;
    if !used_percent.is_finite() || !(0.0..=10_000.0).contains(&used_percent) {
        return None;
    }
    let window_duration_mins = value
        .get("windowDurationMins")
        .and_then(Value::as_i64)
        .filter(|duration| (1..=525_600).contains(duration));
    let resets_at = value
        .get("resetsAt")
        .and_then(Value::as_i64)
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|date| date.to_rfc3339());
    Some(RateLimitWindow {
        used_percent,
        window_duration_mins,
        resets_at,
    })
}

fn sanitize_identifier(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > max_chars
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
    {
        return None;
    }
    Some(value.to_string())
}

fn sanitize_display(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value.to_string())
}

fn validated_secret_component(value: &str, max_chars: usize) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return None;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[cfg(unix)]
    use std::{fs, os::unix::fs::PermissionsExt};

    #[derive(Default)]
    struct MemoryAccountCache {
        value: Mutex<Option<Vec<u8>>>,
    }

    impl MemoryAccountCache {
        fn with(snapshot: &AccountSnapshot) -> Self {
            Self {
                value: Mutex::new(Some(serde_json::to_vec(snapshot).unwrap())),
            }
        }

        fn snapshot(&self) -> AccountSnapshot {
            let value = self.value.lock().unwrap().clone().unwrap();
            parse_cached_snapshot(Zeroizing::new(value)).unwrap()
        }
    }

    impl AccountSnapshotCache for MemoryAccountCache {
        fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
            Ok(self.value.lock().unwrap().clone().map(Zeroizing::new))
        }

        fn write(&self, value: &[u8]) -> Result<(), ()> {
            *self.value.lock().unwrap() = Some(value.to_vec());
            Ok(())
        }
    }

    fn cached_fixture(email: &str) -> AccountSnapshot {
        let now = Utc::now().to_rfc3339();
        AccountSnapshot {
            state: "authenticated".into(),
            authenticated: true,
            auth_method: Some("chatgpt".into()),
            email: Some(email.into()),
            plan_type: Some("pro".into()),
            requires_openai_auth: Some(true),
            login_in_progress: false,
            rate_limits_available: true,
            rate_limits: Vec::new(),
            available_reset_credits: Some(1),
            checked_at: now.clone(),
            source: "live".into(),
            stale: false,
            cached_at: Some(now),
            message: "账户已确认。".into(),
        }
    }

    #[test]
    fn cache_hit_does_not_trigger_live_account_read() {
        let cache = MemoryAccountCache::with(&cached_fixture("cached@example.test"));
        let calls = AtomicUsize::new(0);
        let snapshot = read_account_with_cache_and_live(None, false, false, &cache, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("must not be called".into())
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot.source, "cache");
        assert!(!snapshot.stale);
        assert_eq!(snapshot.email.as_deref(), Some("cached@example.test"));
    }

    #[test]
    fn cache_hit_preserves_active_login_polling_state() {
        let cache = MemoryAccountCache::with(&cached_fixture("cached@example.test"));
        let snapshot = read_account_with_cache_and_live(None, true, false, &cache, |_, _| {
            Err("must not be called".into())
        });
        assert!(snapshot.login_in_progress);
        assert_eq!(snapshot.source, "cache");
    }

    #[test]
    fn forced_refresh_replaces_account_cache() {
        let cache = MemoryAccountCache::with(&cached_fixture("old@example.test"));
        let calls = AtomicUsize::new(0);
        let snapshot = read_account_with_cache_and_live(None, false, true, &cache, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(cached_fixture("fresh@example.test"))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(snapshot.source, "live");
        assert!(!snapshot.stale);
        assert_eq!(
            cache.snapshot().email.as_deref(),
            Some("fresh@example.test")
        );
    }

    #[test]
    fn forced_refresh_failure_returns_stale_cache_without_error_body() {
        let cache = MemoryAccountCache::with(&cached_fixture("cached@example.test"));
        let snapshot = read_account_with_cache_and_live(None, false, true, &cache, |_, _| {
            Err("upstream token=secret should not escape".into())
        });
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(snapshot.source, "stale");
        assert!(snapshot.stale);
        assert!(snapshot.cached_at.is_some());
        assert!(!serialized.contains("token=secret"));
    }

    #[test]
    fn expired_cache_is_explicitly_marked_stale_without_live_read() {
        let mut fixture = cached_fixture("cached@example.test");
        fixture.checked_at = "2025-08-30T00:00:00Z".into();
        fixture.cached_at = Some("2025-08-30T00:00:00Z".into());
        let cache = MemoryAccountCache::with(&fixture);
        let calls = AtomicUsize::new(0);
        let snapshot = read_account_with_cache_and_live(None, false, false, &cache, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("must not be called".into())
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(snapshot.source, "stale");
        assert!(snapshot.stale);
    }

    #[test]
    fn account_snapshot_only_exposes_allowlisted_identity_fields() {
        let account = json!({
            "id": 1,
            "result": {
                "account": {
                    "type": "chatgpt",
                    "email": "person@example.test",
                    "planType": "pro",
                    "accessToken": "never-expose"
                },
                "requiresOpenaiAuth": true
            }
        });
        let snapshot = parse_snapshot(&account, None, false);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(snapshot.state, "authenticated");
        assert_eq!(snapshot.auth_method.as_deref(), Some("chatgpt"));
        assert_eq!(snapshot.email.as_deref(), Some("person@example.test"));
        assert!(!serialized.contains("never-expose"));
        assert!(!serialized.contains("accessToken"));
    }

    #[test]
    fn rate_limits_are_bounded_and_normalized() {
        let response = json!({
            "result": {
                "rateLimitsByLimitId": {
                    "codex": {
                        "limitId": "codex",
                        "limitName": "Codex",
                        "primary": { "usedPercent": 25, "windowDurationMins": 300, "resetsAt": 1730947200 },
                        "secondary": null
                    }
                },
                "rateLimitResetCredits": { "availableCount": 2 }
            }
        });
        let (available, buckets, credits) = parse_rate_limits(Some(&response));
        assert!(available);
        assert_eq!(credits, Some(2));
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].limit_id, "codex");
        assert_eq!(buckets[0].primary.as_ref().unwrap().used_percent, 25.0);
    }

    #[test]
    fn imported_auth_requires_a_successful_non_refreshing_authenticated_probe() {
        assert!(require_authenticated_rate_limits(Some(&json!({ "result": {} }))).is_ok());
        assert!(
            require_authenticated_rate_limits(Some(&json!({
                "error": { "code": -32000, "message": "redacted" }
            })))
            .is_err()
        );
        assert!(require_authenticated_rate_limits(None).is_err());
    }

    #[test]
    fn oversized_app_server_lines_are_rejected_before_json_parsing() {
        let data = vec![b'x'; MAX_APP_SERVER_LINE_BYTES + 1];
        let mut reader = Cursor::new(data);
        assert!(read_bounded_json_line(&mut reader).is_err());
    }

    #[test]
    fn untrusted_identifiers_and_control_text_are_rejected() {
        assert_eq!(
            sanitize_identifier("chatgpt", 64).as_deref(),
            Some("chatgpt")
        );
        assert!(sanitize_identifier("../secret", 64).is_none());
        assert!(sanitize_display("hello\nworld", 64).is_none());
    }

    #[test]
    fn profile_identity_distinguishes_workspaces_for_the_same_email() {
        let response = json!({
            "result": {
                "account": {
                    "type": "chatgpt",
                    "email": "person@example.test",
                    "planType": "pro"
                }
            }
        });
        let first = br#"{"tokens":{"account_id":"workspace-a"}}"#;
        let second = br#"{"tokens":{"account_id":"workspace-b"}}"#;
        assert_ne!(
            verified_identity_fingerprint(&response, first).unwrap(),
            verified_identity_fingerprint(&response, second).unwrap()
        );
        assert!(verified_identity_fingerprint(&response, br#"{"tokens":{}}"#).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn app_server_process_round_trip_reads_account_and_quota_responses() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex");
        fs::write(
            &executable,
            r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 2
head -n 4 >/dev/null
printf '%s\n' '{"id":1,"result":{"account":{"type":"chatgpt","email":"fixture@example.test","planType":"pro"},"requiresOpenaiAuth":true}}'
printf '%s\n' '{"id":2,"result":{"rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":20}}}}}'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();

        let (account, rate_limits) =
            app_server_snapshot(&executable, None, None, false, true, false).unwrap();

        assert_eq!(account.get("id").and_then(Value::as_i64), Some(1));
        assert_eq!(
            rate_limits
                .as_ref()
                .and_then(|value| value.get("id"))
                .and_then(Value::as_i64),
            Some(2)
        );
    }

    #[cfg(unix)]
    #[test]
    fn imported_auth_uses_external_access_token_without_persisted_auth_file() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex");
        fs::write(
            &executable,
            r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 2
IFS= read -r initialize
case "$initialize" in
  *'"experimentalApi":true'*) ;;
  *) exit 3 ;;
esac
head -n 2 >/dev/null
printf '%s\n' '{"id":4,"result":{"type":"chatgptAuthTokens"}}'
head -n 3 >/dev/null
printf '%s\n' '{"id":1,"result":{"account":{"type":"chatgpt","email":"fixture@example.test","planType":"pro"}}}'
printf '%s\n' '{"id":2,"result":{"rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":20}}}}}'
printf '%s\n' '{"id":3,"result":{"config":{"cli_auth_credentials_store":"file"},"origins":{}}}'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let candidate = br#"{"auth_mode":"chatgpt","tokens":{"access_token":"fixture-access-token","refresh_token":"fixture-refresh-token","account_id":"workspace-a"}}"#;
        let external_auth = extract_external_auth(candidate).unwrap();

        let (account, rate_limits) = app_server_snapshot(
            &executable,
            Some(directory.path()),
            Some(external_auth),
            false,
            true,
            true,
        )
        .unwrap();

        assert_eq!(account.get("id").and_then(Value::as_i64), Some(1));
        assert!(require_authenticated_rate_limits(rate_limits.as_ref()).is_ok());
        assert!(!directory.path().join("auth.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn active_identity_requires_effective_file_credentials_store() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex");
        fs::write(
            &executable,
            r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 2
head -n 4 >/dev/null
printf '%s\n' '{"id":1,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}'
printf '%s\n' '{"id":3,"result":{"config":{"cli_auth_credentials_store":"file"},"origins":{}}}'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions.clone()).unwrap();

        assert!(app_server_snapshot(&executable, None, None, false, false, true).is_ok());

        fs::write(
            &executable,
            r#"#!/bin/sh
[ "$1" = "app-server" ] || exit 2
head -n 4 >/dev/null
printf '%s\n' '{"id":1,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}'
printf '%s\n' '{"id":3,"result":{"config":{"cli_auth_credentials_store":"auto"},"origins":{}}}'
"#,
        )
        .unwrap();
        fs::set_permissions(&executable, permissions).unwrap();

        assert!(app_server_snapshot(&executable, None, None, false, false, true).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the installed signed ChatGPT Codex bundle"]
    fn installed_app_server_reports_isolated_file_store() {
        use std::os::unix::fs::OpenOptionsExt;

        let executable = platform::trusted_codex_auth_candidate().unwrap();
        let home = tempfile::tempdir().unwrap();
        let mut config = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(home.path().join("config.toml"))
            .unwrap();
        config
            .write_all(b"cli_auth_credentials_store = \"file\"\n")
            .unwrap();
        config.sync_all().unwrap();

        assert!(
            app_server_snapshot(
                executable.path(),
                Some(home.path()),
                None,
                false,
                false,
                true,
            )
            .is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_supervisor_reaps_process_without_status_polling() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fake-codex-login");
        fs::write(&executable, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();
        let child = platform::spawn_codex_login(&executable).unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let cancel = Arc::new(AtomicBool::new(false));
        let started = Instant::now();

        supervise_login_process(
            child,
            Arc::clone(&active),
            cancel,
            Duration::from_millis(100),
        );

        assert!(!active.load(Ordering::Acquire));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
