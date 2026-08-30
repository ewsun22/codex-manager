//! Lifecycle and supply-chain boundary for the optional CLIProxyAPI sidecar.
//!
//! The WebView sees provider metadata only. API keys and the installation-scoped
//! loopback bearer remain in the platform Keychain / server handle and are
//! never included in ordinary DTOs or log messages.

use crate::proxy_auth::{ProxyAuthError, ProxyAuthStore, RuntimeMaterialization};
use chrono::Utc;
use codex_storage::{CodexProviderRow, Store};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, ToSocketAddrs},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYCHAIN_SERVICE: &str = "cc.codex.manager.providers.v1";
const KEYCHAIN_PREFIX: &str = "provider:";
const KEYCHAIN_GATEWAY_TOKEN: &str = "gateway-client-token";
const MAX_FIELD_BYTES: usize = 256;
const CORE_REPOSITORY: &str = "router-for-me/CLIProxyAPI";
const CORE_RELEASE_URL: &str =
    "https://api.github.com/repos/router-for-me/CLIProxyAPI/releases/latest";
const CORE_DOWNLOAD_LIMIT: u64 = 128 * 1024 * 1024;
const CORE_EXTRACT_LIMIT: u64 = 512 * 1024 * 1024;
const CORE_HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
const CORE_STOP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSaveInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: String,
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub reasoning_effort: String,
    pub has_api_key: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub state: String,
    pub source: String,
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub endpoint: Option<String>,
    pub started_at: Option<String>,
    pub requests: Option<u64>,
    pub failed: Option<u64>,
    pub in_flight: Option<u32>,
    pub last_error: Option<String>,
    pub port: u16,
    pub installed: bool,
    pub core_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub installing: bool,
    pub process_id: Option<u32>,
    pub core_source: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
#[serde(rename_all = "camelCase")]
struct CoreMetadata {
    core_version: Option<String>,
    asset: Option<String>,
    digest: Option<String>,
    installed_at: Option<String>,
    latest_version: Option<String>,
    previous_core_dir: Option<String>,
    previous: Option<Box<CoreMetadata>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInstallJournal {
    backup_core_dir: Option<String>,
    failed_core_dir: String,
    previous: CoreMetadata,
    pending: CoreMetadata,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreRollbackJournal {
    backup_core_dir: String,
    failed_core_dir: String,
    restored: CoreMetadata,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Clone, Debug)]
struct CoreRelease {
    version: String,
    asset: GithubAsset,
    checksums: GithubAsset,
}

trait KeychainBackend: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, GatewayError>;
    fn write(&self, account: &str, value: &[u8]) -> Result<(), GatewayError>;
    fn delete(&self, account: &str) -> Result<(), GatewayError>;
}

#[cfg(target_os = "macos")]
struct MacKeychain;
#[cfg(target_os = "macos")]
impl KeychainBackend for MacKeychain {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, GatewayError> {
        match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(GatewayError::Keychain),
        }
    }
    fn write(&self, account: &str, value: &[u8]) -> Result<(), GatewayError> {
        security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, value)
            .map_err(|_| GatewayError::Keychain)
    }
    fn delete(&self, account: &str) -> Result<(), GatewayError> {
        match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(GatewayError::Keychain),
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedKeychain;
#[cfg(not(target_os = "macos"))]
impl KeychainBackend for UnsupportedKeychain {
    fn read(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>, GatewayError> {
        Err(GatewayError::Keychain)
    }
    fn write(&self, _: &str, _: &[u8]) -> Result<(), GatewayError> {
        Err(GatewayError::Keychain)
    }
    fn delete(&self, _: &str) -> Result<(), GatewayError> {
        Err(GatewayError::Keychain)
    }
}

#[derive(Debug, thiserror::Error)]
enum GatewayError {
    #[error("供应商配置不符合要求。")]
    InvalidProvider,
    #[error("供应商不存在。")]
    NotFound,
    #[error("新供应商必须提供 API Key。")]
    MissingApiKey,
    #[error("无法访问 macOS Keychain。")]
    Keychain,
}

fn mac_keychain() -> Arc<dyn KeychainBackend> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(MacKeychain)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnsupportedKeychain)
    }
}

fn provider_account(id: &str) -> String {
    format!("{KEYCHAIN_PREFIX}{id}")
}

pub fn list(store: &Store) -> Result<Vec<ProviderDto>, String> {
    list_with_keychain(store, mac_keychain()).map_err(public_error)
}

fn list_with_keychain(
    store: &Store,
    keychain: Arc<dyn KeychainBackend>,
) -> Result<Vec<ProviderDto>, GatewayError> {
    store
        .codex_providers()
        .map_err(|_| GatewayError::InvalidProvider)?
        .into_iter()
        .map(|row| {
            let has_api_key = keychain.read(&provider_account(&row.id))?.is_some();
            Ok(dto(row, has_api_key))
        })
        .collect()
}

pub fn save(store: &Store, input: ProviderSaveInput) -> Result<ProviderDto, String> {
    save_with_keychain(store, input, mac_keychain()).map_err(public_error)
}

fn save_with_keychain(
    store: &Store,
    input: ProviderSaveInput,
    keychain: Arc<dyn KeychainBackend>,
) -> Result<ProviderDto, GatewayError> {
    let normalized = normalize_input(input)?;
    let old = if let Some(id) = normalized.id.as_deref() {
        store
            .codex_provider(id)
            .map_err(|_| GatewayError::InvalidProvider)?
    } else {
        None
    };
    if normalized.id.is_some() && old.is_none() {
        return Err(GatewayError::NotFound);
    }
    let id = normalized.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let account = provider_account(&id);
    let key_update = normalized.api_key.filter(|value| !value.trim().is_empty());
    if old.is_none() && key_update.is_none() {
        return Err(GatewayError::MissingApiKey);
    }
    let previous_key = if key_update.is_some() {
        keychain.read(&account)?
    } else {
        None
    };
    if let Some(ref key) = key_update {
        keychain.write(&account, key.as_bytes())?;
    }
    let now = Utc::now().to_rfc3339();
    let row = CodexProviderRow {
        id,
        name: normalized.name,
        base_url: normalized.base_url,
        model: normalized.model,
        reasoning_effort: normalized.reasoning_effort,
        created_at: old
            .as_ref()
            .map(|v| v.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    if store.save_codex_provider(&row).is_err() {
        if key_update.is_some() {
            restore_secret(&*keychain, &account, previous_key);
        }
        return Err(GatewayError::InvalidProvider);
    }
    let has_api_key = key_update.is_some() || keychain.read(&account)?.is_some();
    Ok(dto(row, has_api_key))
}

pub fn delete(store: &Store, provider_id: &str) -> Result<Vec<ProviderDto>, String> {
    delete_with_keychain(store, provider_id, mac_keychain()).map_err(public_error)
}

fn delete_with_keychain(
    store: &Store,
    provider_id: &str,
    keychain: Arc<dyn KeychainBackend>,
) -> Result<Vec<ProviderDto>, GatewayError> {
    let provider = store
        .codex_provider(provider_id)
        .map_err(|_| GatewayError::InvalidProvider)?
        .ok_or(GatewayError::NotFound)?;
    let account = provider_account(&provider.id);
    let secret = keychain.read(&account)?;
    keychain.delete(&account)?;
    let deleted = store.delete_codex_provider(&provider.id).map_err(|_| ());
    complete_metadata_delete(&*keychain, &account, secret, deleted)?;
    list_with_keychain(store, keychain)
}

fn complete_metadata_delete(
    keychain: &dyn KeychainBackend,
    account: &str,
    secret: Option<Zeroizing<Vec<u8>>>,
    deleted: Result<bool, ()>,
) -> Result<(), GatewayError> {
    match deleted {
        Ok(true) => Ok(()),
        Ok(false) => {
            restore_secret(keychain, account, secret);
            Err(GatewayError::NotFound)
        }
        Err(()) => {
            restore_secret(keychain, account, secret);
            Err(GatewayError::InvalidProvider)
        }
    }
}

fn restore_secret(keychain: &dyn KeychainBackend, account: &str, old: Option<Zeroizing<Vec<u8>>>) {
    match old {
        Some(value) => {
            let _ = keychain.write(account, &value);
        }
        None => {
            let _ = keychain.delete(account);
        }
    }
}

struct NormalizedInput {
    id: Option<String>,
    name: String,
    base_url: String,
    model: String,
    reasoning_effort: String,
    api_key: Option<Zeroizing<String>>,
}
fn normalize_input(input: ProviderSaveInput) -> Result<NormalizedInput, GatewayError> {
    let id = input.id.filter(|value| !value.is_empty());
    if id
        .as_deref()
        .is_some_and(|value| Uuid::parse_str(value).is_err())
    {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(NormalizedInput {
        id,
        name: clean_field(&input.name)?,
        base_url: normalize_base_url(&input.base_url)?,
        model: clean_field(&input.model)?,
        reasoning_effort: clean_field(&input.reasoning_effort)?,
        api_key: input.api_key.map(Zeroizing::new),
    })
}
fn clean_field(value: &str) -> Result<String, GatewayError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_FIELD_BYTES || value.chars().any(char::is_control) {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(value.to_owned())
}
fn normalize_base_url(value: &str) -> Result<String, GatewayError> {
    let mut url = Url::parse(value.trim()).map_err(|_| GatewayError::InvalidProvider)?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(GatewayError::InvalidProvider);
    }
    let host = url.host_str().ok_or(GatewayError::InvalidProvider)?;
    let local = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1";
    if !(url.scheme() == "https" || (url.scheme() == "http" && local)) {
        return Err(GatewayError::InvalidProvider);
    }
    if url.port_or_known_default().is_none() {
        return Err(GatewayError::InvalidProvider);
    }
    if url.scheme() == "https" {
        resolve_safe_remote(host, url.port_or_known_default().unwrap())?;
    }
    let path = url.path().trim_end_matches('/');
    if !(path.is_empty() || path == "/v1") {
        return Err(GatewayError::InvalidProvider);
    }
    url.set_path("/v1");
    Ok(url.to_string().trim_end_matches('/').to_owned())
}
/// A deliberately conservative global-unicast allowlist. `IpAddr::is_global`
/// is not available on every supported Rust version, and permissive fallback
/// checks would let special-use or documentation networks become SSRF sinks.
fn unsafe_remote_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(value) => !is_public_v4(value),
        IpAddr::V6(value) => !is_public_v6(value),
    }
}
fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    if a == 0 || a == 10 || a == 127 || a >= 224 {
        return false;
    }
    if (a == 100 && (64..=127).contains(&b)) // shared CGNAT
        || (a == 169 && b == 254) // link-local
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19)) // benchmark
        || (a == 192 && b == 0) // IETF protocol assignments / documentation
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || (a == 192 && b == 31 && c == 196) // AS112
        || (a == 192 && b == 52 && c == 193) // AMT
        || (a == 192 && b == 88 && c == 99) // deprecated 6to4 relay
        || (a == 192 && b == 175 && c == 48)
    {
        return false;
    }
    true
}
fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    let segments = ip.segments();
    // Only globally routable 2000::/3 is permitted. This excludes ::/128,
    // loopback, IPv4-compatible, NAT64, ULA, link-local and multicast space.
    if (segments[0] & 0xe000) != 0x2000 {
        return false;
    }
    // Remaining special-purpose addresses nested inside 2000::/3.
    !((segments[0] == 0x2001
        && (segments[1] <= 0x01ff // IETF special-purpose 2001::/23
            || segments[1] == 0x0db8)) // documentation 2001:db8::/32
        || segments[0] == 0x2002 // deprecated 6to4, can embed non-public v4
        || segments[0] == 0x3ffe) // retired 6bone
}
fn resolve_safe_remote(host: &str, port: u16) -> Result<SocketAddr, GatewayError> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| GatewayError::InvalidProvider)?
        .collect::<Vec<_>>();
    let first = *addresses.first().ok_or(GatewayError::InvalidProvider)?;
    if addresses
        .iter()
        .any(|address| unsafe_remote_ip(address.ip()))
    {
        return Err(GatewayError::InvalidProvider);
    }
    Ok(first)
}
fn dto(row: CodexProviderRow, has_api_key: bool) -> ProviderDto {
    ProviderDto {
        id: row.id,
        name: row.name,
        base_url: row.base_url,
        model: row.model,
        reasoning_effort: row.reasoning_effort,
        has_api_key,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
fn public_error(error: GatewayError) -> String {
    error.to_string()
}

pub struct ServerHandle {
    port: u16,
    source: String,
    provider_id: Option<String>,
    provider_name: String,
    started_at: String,
    endpoint: String,
    token: Zeroizing<String>,
    sidecar: Option<SidecarProcess>,
    proxy_auth_store: Option<Arc<ProxyAuthStore>>,
    proxy_auth_runtime: Option<RuntimeMaterialization>,
}

struct SidecarProcess {
    child: Mutex<Child>,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
}
impl ServerHandle {
    pub fn status(&self) -> GatewayStatus {
        let Some(sidecar) = self.sidecar.as_ref() else {
            return status_from_core(
                &PathBuf::new(),
                self.port,
                CoreStatusFields {
                    state: "error",
                    last_error: Some("CLIProxyAPI 进程状态不可用。".into()),
                    ..CoreStatusFields::default()
                },
            );
        };
        let (alive, process_id) = sidecar
            .child
            .lock()
            .map(|mut child| match child.try_wait() {
                Ok(None) => (true, Some(child.id())),
                Ok(Some(_)) | Err(_) => (false, None),
            })
            .unwrap_or((false, None));
        status_from_core(
            &sidecar.data_dir,
            self.port,
            CoreStatusFields {
                state: if alive { "running" } else { "error" },
                source: Some(self.source.clone()),
                provider_id: self.provider_id.clone(),
                provider_name: Some(self.provider_name.clone()),
                endpoint: Some(self.endpoint.clone()),
                started_at: Some(self.started_at.clone()),
                last_error: if alive {
                    None
                } else {
                    Some("CLIProxyAPI 进程已退出。".into())
                },
                process_id,
                installing: false,
            },
        )
    }
    pub async fn stop(mut self) -> Result<(), String> {
        let sidecar = self
            .sidecar
            .take()
            .ok_or_else(|| "CLIProxyAPI 进程状态不可用。".to_string())?;
        stop_sidecar(sidecar).await?;
        self.checkpoint_proxy_auth()
    }

    fn checkpoint_proxy_auth(&mut self) -> Result<(), String> {
        let Some(materialization) = self.proxy_auth_runtime.take() else {
            return Ok(());
        };
        self.proxy_auth_store
            .as_ref()
            .ok_or_else(|| "CLIProxyAPI OAuth checkpoint 状态不完整。".to_string())?
            .checkpoint_materialized_runtime(&materialization)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(sidecar) = self.sidecar.take() {
            if let Ok(mut child) = sidecar.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(sidecar.runtime_dir);
        }
        let _ = self.checkpoint_proxy_auth();
    }
}

/// Fetches only the official GitHub Release manifest and persists the latest
/// version marker. A missing API digest is deliberately a hard failure.
pub async fn check_latest_core(data_dir: &Path) -> Result<(), String> {
    let release = fetch_latest_release().await?;
    let mut metadata = read_core_metadata(data_dir)?;
    metadata.latest_version = Some(release.version);
    write_core_metadata(data_dir, &metadata)
}

/// Installs an official prebuilt CLIProxyAPI release. CLIProxyAPI source code is
/// never downloaded, built or linked into the desktop application.
pub async fn install_latest_core(data_dir: &Path) -> Result<(), String> {
    let root = core_root(data_dir);
    ensure_private_dir(&root)?;
    recover_incomplete_core_rollback(&root)?;
    recover_incomplete_core_install(&root)?;
    let release = fetch_latest_release().await?;
    let stage = root.join(format!(".staging-{}", Uuid::new_v4()));
    ensure_private_dir(&stage)?;
    let outcome = install_release_into(&root, &stage, &release).await;
    let _ = fs::remove_dir_all(&stage);
    outcome
}

pub async fn start(
    store: &Store,
    provider_id: &str,
    port: u16,
    data_dir: &Path,
) -> Result<ServerHandle, String> {
    if !(1024..=65535).contains(&port) {
        return Err("本地反代端口必须在 1024 到 65535 之间。".into());
    }
    let metadata = read_core_metadata(data_dir)?;
    if metadata.core_version.is_none() {
        return Err("CLIProxyAPI 内核尚未安装；请先安装官方 Release。".into());
    }
    let row = store
        .codex_provider(provider_id)
        .map_err(|_| "读取供应商失败。".to_string())?
        .ok_or_else(|| "供应商不存在。".to_string())?;
    let secret = mac_keychain()
        .read(&provider_account(&row.id))
        .map_err(public_error)?
        .ok_or_else(|| "供应商缺少 API Key。".to_string())?;
    let upstream_key =
        String::from_utf8(secret.to_vec()).map_err(|_| "供应商 API Key 无效。".to_string())?;
    let token = gateway_token(mac_keychain())?;
    match start_sidecar(row, upstream_key, token, port, data_dir).await {
        Ok(handle) => {
            if let Err(error) = finalize_core_update(data_dir) {
                let _ = handle.stop().await;
                let rollback = rollback_core_update(data_dir);
                return Err(match rollback {
                    Ok(true) => format!("新 CLIProxyAPI 内核已停止并恢复上一版：{error}"),
                    Ok(false) => error,
                    Err(rollback_error) => format!("{error}；{rollback_error}"),
                });
            }
            Ok(handle)
        }
        Err(error) => match rollback_core_update(data_dir) {
            Ok(true) => Err(format!(
                "新 CLIProxyAPI 内核未通过健康检查，已恢复上一版：{error}"
            )),
            Ok(false) => Err(error),
            Err(rollback_error) => Err(format!("{error}；{rollback_error}")),
        },
    }
}

pub async fn start_oauth(
    port: u16,
    data_dir: &Path,
    proxy_auth_store: Arc<ProxyAuthStore>,
) -> Result<ServerHandle, String> {
    if !(1024..=65535).contains(&port) {
        return Err("本地反代端口必须在 1024 到 65535 之间。".into());
    }
    let metadata = read_core_metadata(data_dir)?;
    if metadata.core_version.is_none() {
        return Err("CLIProxyAPI 内核尚未安装；请先安装官方 Release。".into());
    }
    // Holding the port before touching recovery evidence proves that no
    // orphaned CLIProxyAPI process is still serving/writing the auth-dir.
    let port_guard = reserve_loopback_port(port)?;
    let auth_parent = proxy_auth_runtime_dir(data_dir);
    let materialization = match proxy_auth_store.materialize_enabled_for_runtime(&auth_parent) {
        Ok(value) => value,
        Err(ProxyAuthError::RecoveryRequired) => {
            proxy_auth_store
                .recover_pending_runtime(&auth_parent)
                .map_err(|error| error.to_string())?;
            proxy_auth_store
                .materialize_enabled_for_runtime(&auth_parent)
                .map_err(|error| error.to_string())?
        }
        Err(error) => return Err(error.to_string()),
    };
    let token = gateway_token(mac_keychain())?;
    match start_oauth_sidecar(
        token,
        port,
        data_dir,
        port_guard,
        proxy_auth_store,
        materialization,
    )
    .await
    {
        Ok(handle) => {
            if let Err(error) = finalize_core_update(data_dir) {
                let _ = handle.stop().await;
                let rollback = rollback_core_update(data_dir);
                return Err(match rollback {
                    Ok(true) => format!("新 CLIProxyAPI 内核已停止并恢复上一版：{error}"),
                    Ok(false) => error,
                    Err(rollback_error) => format!("{error}；{rollback_error}"),
                });
            }
            Ok(handle)
        }
        Err(error) => match rollback_core_update(data_dir) {
            Ok(true) => Err(format!(
                "新 CLIProxyAPI 内核未通过健康检查，已恢复上一版：{error}"
            )),
            Ok(false) => Err(error),
            Err(rollback_error) => Err(format!("{error}；{rollback_error}")),
        },
    }
}

fn core_root(data_dir: &Path) -> PathBuf {
    data_dir.join("cliproxyapi")
}

fn core_dir(data_dir: &Path) -> PathBuf {
    core_root(data_dir).join("core")
}

fn runtime_dir(data_dir: &Path) -> PathBuf {
    core_root(data_dir).join("runtime")
}

fn proxy_auth_runtime_dir(data_dir: &Path) -> PathBuf {
    core_root(data_dir).join("oauth-runtime")
}

fn cleanup_stale_runtime(data_dir: &Path) {
    let runtime = runtime_dir(data_dir);
    if fs::symlink_metadata(&runtime)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        let _ = fs::remove_dir_all(runtime);
    }
}

fn reserve_loopback_port(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .map_err(|_| "本地反代端口已被其他进程占用；未启动 CLIProxyAPI。".to_string())
}

fn metadata_path(data_dir: &Path) -> PathBuf {
    core_root(data_dir).join("core-metadata.json")
}

fn install_journal_path(root: &Path) -> PathBuf {
    root.join("core-install-journal.json")
}

fn rollback_journal_path(root: &Path) -> PathBuf {
    root.join("core-rollback-journal.json")
}

fn read_core_metadata(data_dir: &Path) -> Result<CoreMetadata, String> {
    let root = core_root(data_dir);
    recover_incomplete_core_rollback(&root)?;
    recover_incomplete_core_install(&root)?;
    read_core_metadata_from_root(&root)
}

fn read_core_metadata_from_root(root: &Path) -> Result<CoreMetadata, String> {
    let path = root.join("core-metadata.json");
    if !path.exists() {
        return Ok(CoreMetadata::default());
    }
    let contents = fs::read(&path).map_err(|_| "读取 CLIProxyAPI 内核元数据失败。".to_string())?;
    serde_json::from_slice(&contents)
        .map_err(|_| "CLIProxyAPI 内核元数据损坏；为安全起见未使用该内核。".to_string())
}

fn write_core_metadata(data_dir: &Path, metadata: &CoreMetadata) -> Result<(), String> {
    ensure_private_dir(&core_root(data_dir))?;
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|_| "写入 CLIProxyAPI 内核元数据失败。".to_string())?;
    atomic_private_write(&metadata_path(data_dir), &bytes)
}

async fn fetch_latest_release() -> Result<CoreRelease, String> {
    let client = Client::builder()
        .user_agent("Codex-Manager/CLIProxyAPI-core")
        .redirect(Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "创建 CLIProxyAPI Release 客户端失败。".to_string())?;
    let response = client
        .get(CORE_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "无法读取 CLIProxyAPI 官方 Release。".to_string())?;
    if !response.status().is_success() {
        return Err("无法读取 CLIProxyAPI 官方 Release。".into());
    }
    let release: GithubRelease = response
        .json()
        .await
        .map_err(|_| "CLIProxyAPI 官方 Release 数据无效。".to_string())?;
    select_release_asset(release)
}

fn select_release_asset(release: GithubRelease) -> Result<CoreRelease, String> {
    let (os, arch, extension) = platform_asset_parts()?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .filter(|value| {
            let parts = value.split('.').collect::<Vec<_>>();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .ok_or_else(|| "CLIProxyAPI 最新 Release tag 不是受支持的稳定语义版本。".to_string())?
        .to_owned();
    let expected_name = format!("CLIProxyAPI_{version}_{os}_{arch}.{extension}");
    let mut candidates = release
        .assets
        .iter()
        .filter(|asset| asset.name == expected_name)
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err("CLIProxyAPI 官方 Release 未提供唯一匹配的当前平台内核。".into());
    }
    let checksums = release
        .assets
        .iter()
        .find(|asset| asset.name == "checksums.txt")
        .cloned()
        .ok_or_else(|| "CLIProxyAPI 官方 Release 缺少 checksums.txt。".to_string())?;
    require_sha256_digest(&candidates[0].digest)?;
    require_sha256_digest(&checksums.digest)?;
    Ok(CoreRelease {
        version,
        asset: candidates.remove(0),
        checksums,
    })
}

fn platform_asset_parts() -> Result<(&'static str, &'static str, &'static str), String> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => return Err("当前平台不支持 CLIProxyAPI 预编译内核。".into()),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "amd64",
        _ => return Err("当前处理器架构不支持 CLIProxyAPI 预编译内核。".into()),
    };
    Ok((os, arch, if os == "windows" { "zip" } else { "tar.gz" }))
}

fn require_sha256_digest(value: &Option<String>) -> Result<String, String> {
    let value = value
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "CLIProxyAPI 官方 Release 缺少可验证的 SHA-256 digest。".to_string())?;
    Ok(value.to_ascii_lowercase())
}

async fn install_release_into(
    root: &Path,
    stage: &Path,
    release: &CoreRelease,
) -> Result<(), String> {
    let previous_metadata = read_core_metadata_from_root(root)?;
    if previous_metadata.previous_core_dir.is_some() {
        return Err("上一次 CLIProxyAPI 更新尚未通过启动验证；请先启动或恢复内核。".into());
    }
    let archive = stage.join(&release.asset.name);
    let checksums = stage.join("checksums.txt");
    download_verified(&release.asset, &archive).await?;
    download_verified(&release.checksums, &checksums).await?;
    let api_digest = require_sha256_digest(&release.asset.digest)?;
    let checksums_digest = checksum_for_asset(
        &fs::read_to_string(&checksums).map_err(|_| "读取 checksums.txt 失败。".to_string())?,
        &release.asset.name,
    )?;
    if checksums_digest != api_digest || sha256_file(&archive)? != api_digest {
        return Err("CLIProxyAPI 内核 SHA-256 校验失败；未安装该文件。".into());
    }
    let staged_core = stage.join("core");
    ensure_private_dir(&staged_core)?;
    extract_archive(&archive, &staged_core)?;
    let binary = find_unique_core_binary(&staged_core)?;
    set_executable(&binary)?;
    let core = root.join("core");
    let transaction = Uuid::new_v4();
    let backup = core
        .exists()
        .then(|| root.join(format!(".core-backup-{transaction}")));
    let metadata = CoreMetadata {
        core_version: Some(release.version.clone()),
        asset: Some(release.asset.name.clone()),
        digest: Some(api_digest),
        installed_at: Some(Utc::now().to_rfc3339()),
        latest_version: Some(release.version.clone()),
        previous_core_dir: backup.as_ref().and_then(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        }),
        previous: backup.as_ref().map(|_| Box::new(previous_metadata.clone())),
    };
    let journal = CoreInstallJournal {
        backup_core_dir: backup.as_ref().and_then(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        }),
        failed_core_dir: format!(".failed-core-{transaction}"),
        previous: previous_metadata,
        pending: metadata.clone(),
    };
    write_install_journal(root, &journal)?;
    if let Err(error) = replace_core_directory(&core, &staged_core, backup.as_deref()) {
        let _ = recover_incomplete_core_install(root);
        return Err(error);
    }
    if let Err(error) = write_core_metadata_for_root(root, &metadata) {
        return match recover_incomplete_core_install(root) {
            Ok(()) => Err(error),
            Err(recovery_error) => Err(format!("{error}；{recovery_error}")),
        };
    }
    fs::remove_file(install_journal_path(root))
        .map_err(|_| "CLIProxyAPI 内核已切换，但清理安装事务标记失败。".to_string())?;
    Ok(())
}

fn replace_core_directory(
    core: &Path,
    staged_core: &Path,
    backup: Option<&Path>,
) -> Result<(), String> {
    if let Some(backup) = backup {
        fs::rename(core, backup).map_err(|_| "备份现有 CLIProxyAPI 内核失败。".to_string())?;
    }
    if fs::rename(staged_core, core).is_err() {
        if let Some(backup) = backup {
            let _ = fs::rename(backup, core);
        }
        return Err("切换 CLIProxyAPI 内核失败；已保留旧内核。".into());
    }
    Ok(())
}

async fn download_verified(asset: &GithubAsset, destination: &Path) -> Result<(), String> {
    let expected = require_sha256_digest(&asset.digest)?;
    let client = Client::builder()
        .user_agent("Codex-Manager/CLIProxyAPI-core")
        // GitHub asset URLs intentionally redirect to a signed release-assets
        // origin. Integrity is still fail-closed against the Release API digest.
        .redirect(Policy::limited(5))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| "创建 CLIProxyAPI 下载客户端失败。".to_string())?;
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|_| "下载 CLIProxyAPI 官方内核失败。".to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > CORE_DOWNLOAD_LIMIT)
    {
        return Err("CLIProxyAPI 官方内核下载被拒绝。".into());
    }
    let mut output = private_file(destination)?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "CLIProxyAPI 官方内核下载中断。".to_string())?;
        size = size.saturating_add(chunk.len() as u64);
        if size > CORE_DOWNLOAD_LIMIT {
            return Err("CLIProxyAPI 官方内核超过下载大小上限。".into());
        }
        output
            .write_all(&chunk)
            .map_err(|_| "写入 CLIProxyAPI 下载文件失败。".to_string())?;
    }
    output
        .sync_all()
        .map_err(|_| "同步 CLIProxyAPI 下载文件失败。".to_string())?;
    if sha256_file(destination)? != expected {
        return Err("CLIProxyAPI 官方内核下载校验失败。".into());
    }
    Ok(())
}

fn checksum_for_asset(contents: &str, asset: &str) -> Result<String, String> {
    let found = contents.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let filename = parts.next()?.trim_start_matches('*');
        (filename == asset).then(|| digest.to_ascii_lowercase())
    });
    let digest = found.ok_or_else(|| "checksums.txt 未包含所选 CLIProxyAPI 内核。".to_string())?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("checksums.txt 中的 SHA-256 无效。".into());
    }
    Ok(digest)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|_| "读取 CLIProxyAPI 下载文件失败。".to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "读取 CLIProxyAPI 下载文件失败。".to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn extract_archive(archive: &Path, target: &Path) -> Result<(), String> {
    let name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if name.ends_with(".tar.gz") {
        let decoder = GzDecoder::new(
            File::open(archive).map_err(|_| "打开 CLIProxyAPI 压缩包失败。".to_string())?,
        );
        let mut tar = tar::Archive::new(decoder);
        let entries = tar
            .entries()
            .map_err(|_| "读取 CLIProxyAPI 压缩包失败。".to_string())?;
        let mut total = 0_u64;
        for entry in entries {
            let mut entry = entry.map_err(|_| "读取 CLIProxyAPI 压缩条目失败。".to_string())?;
            let entry_path = entry
                .path()
                .map_err(|_| "CLIProxyAPI 压缩条目路径无效。".to_string())?;
            let relative = safe_archive_path(&entry_path)?.to_path_buf();
            let kind = entry.header().entry_type();
            let destination = target.join(relative);
            if kind.is_dir() {
                ensure_private_dir(&destination)?;
            } else if kind.is_file() {
                total = total.saturating_add(entry.size());
                if total > CORE_EXTRACT_LIMIT {
                    return Err("CLIProxyAPI 解压内容超过安全上限。".into());
                }
                if let Some(parent) = destination.parent() {
                    ensure_private_dir(parent)?;
                }
                let mut output = private_file(&destination)?;
                io::copy(&mut entry, &mut output)
                    .map_err(|_| "写入 CLIProxyAPI 解压文件失败。".to_string())?;
            } else {
                return Err("CLIProxyAPI 压缩包包含不允许的链接或特殊文件。".into());
            }
        }
    } else if name.ends_with(".zip") {
        let file = File::open(archive).map_err(|_| "打开 CLIProxyAPI 压缩包失败。".to_string())?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|_| "读取 CLIProxyAPI 压缩包失败。".to_string())?;
        let mut total = 0_u64;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|_| "读取 CLIProxyAPI 压缩条目失败。".to_string())?;
            let relative = safe_archive_path(Path::new(entry.name()))?;
            let mode = entry.unix_mode().unwrap_or(0);
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err("CLIProxyAPI 压缩包包含不允许的链接或特殊文件。".into());
            }
            let destination = target.join(relative);
            if entry.is_dir() {
                ensure_private_dir(&destination)?;
            } else {
                total = total.saturating_add(entry.size());
                if total > CORE_EXTRACT_LIMIT {
                    return Err("CLIProxyAPI 解压内容超过安全上限。".into());
                }
                if let Some(parent) = destination.parent() {
                    ensure_private_dir(parent)?;
                }
                let mut output = private_file(&destination)?;
                io::copy(&mut entry, &mut output)
                    .map_err(|_| "写入 CLIProxyAPI 解压文件失败。".to_string())?;
            }
        }
    } else {
        return Err("CLIProxyAPI 官方内核归档格式不受支持。".into());
    }
    Ok(())
}

fn safe_archive_path(path: &Path) -> Result<&Path, String> {
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("CLIProxyAPI 压缩包包含不安全路径。".into());
    }
    Ok(path)
}

fn find_unique_core_binary(root: &Path) -> Result<PathBuf, String> {
    let expected = if cfg!(windows) {
        "cli-proxy-api.exe"
    } else {
        "cli-proxy-api"
    };
    let binaries = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == expected)
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    if binaries.len() != 1 {
        return Err("CLIProxyAPI 压缩包未包含唯一的 cli-proxy-api 可执行文件。".into());
    }
    Ok(binaries.into_iter().next().unwrap())
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("CLIProxyAPI 私有目录类型不安全。".into());
        }
    }
    fs::create_dir_all(path).map_err(|_| "创建 CLIProxyAPI 私有目录失败。".to_string())?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "验证 CLIProxyAPI 私有目录失败。".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("CLIProxyAPI 私有目录类型不安全。".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "设置 CLIProxyAPI 私有目录权限失败。".to_string())?;
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "创建 CLIProxyAPI 私有文件失败。".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "设置 CLIProxyAPI 私有文件权限失败。".to_string())?;
    }
    Ok(file)
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut file = private_file(&temporary)?;
    file.write_all(bytes)
        .map_err(|_| "写入 CLIProxyAPI 私有文件失败。".to_string())?;
    file.sync_all()
        .map_err(|_| "同步 CLIProxyAPI 私有文件失败。".to_string())?;
    fs::rename(&temporary, path).map_err(|_| "原子替换 CLIProxyAPI 私有文件失败。".to_string())?;
    Ok(())
}

fn write_core_metadata_for_root(root: &Path, metadata: &CoreMetadata) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|_| "写入 CLIProxyAPI 内核元数据失败。".to_string())?;
    atomic_private_write(&root.join("core-metadata.json"), &bytes)
}

fn write_install_journal(root: &Path, journal: &CoreInstallJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|_| "写入 CLIProxyAPI 安装事务标记失败。".to_string())?;
    atomic_private_write(&install_journal_path(root), &bytes)
}

fn write_rollback_journal(root: &Path, journal: &CoreRollbackJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|_| "写入 CLIProxyAPI 回滚事务标记失败。".to_string())?;
    atomic_private_write(&rollback_journal_path(root), &bytes)
}

fn transaction_core_path(root: &Path, name: &str, prefix: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || !name.starts_with(prefix)
    {
        return Err("CLIProxyAPI 安装事务路径无效。".into());
    }
    Ok(root.join(path))
}

fn remove_private_tree(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("读取 CLIProxyAPI 内核目录失败。".into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("CLIProxyAPI 内核目录类型无效，已停止恢复。".into());
    }
    fs::remove_dir_all(path).map_err(|_| "清理 CLIProxyAPI 内核目录失败。".to_string())
}

/// Restores the last committed core state after a process interruption between
/// the directory switch and the atomic metadata commit. The journal is kept
/// until recovery is complete, so this routine is safe to run again.
fn recover_incomplete_core_install(root: &Path) -> Result<(), String> {
    let journal_path = install_journal_path(root);
    let journal_metadata = match fs::symlink_metadata(&journal_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("读取 CLIProxyAPI 安装事务标记失败。".into()),
    };
    if !journal_metadata.is_file()
        || journal_metadata.file_type().is_symlink()
        || journal_metadata.len() > 64 * 1024
    {
        return Err("CLIProxyAPI 安装事务标记无效；为安全起见未启动内核。".into());
    }
    let journal: CoreInstallJournal = serde_json::from_slice(
        &fs::read(&journal_path).map_err(|_| "读取 CLIProxyAPI 安装事务标记失败。".to_string())?,
    )
    .map_err(|_| "CLIProxyAPI 安装事务标记损坏；为安全起见未启动内核。".to_string())?;
    let core = root.join("core");
    let current = read_core_metadata_from_root(root)?;
    if current == journal.pending && core.is_dir() {
        fs::remove_file(&journal_path)
            .map_err(|_| "清理 CLIProxyAPI 安装事务标记失败。".to_string())?;
        return Ok(());
    }

    let failed = transaction_core_path(root, &journal.failed_core_dir, ".failed-core-")?;
    if let Some(backup_name) = journal.backup_core_dir.as_deref() {
        let backup = transaction_core_path(root, backup_name, ".core-backup-")?;
        if backup.is_dir() {
            if core.exists() {
                if failed.exists() {
                    return Err("CLIProxyAPI 安装事务状态冲突；为安全起见未自动覆盖内核。".into());
                }
                fs::rename(&core, &failed)
                    .map_err(|_| "隔离未完成安装的 CLIProxyAPI 内核失败。".to_string())?;
            }
            if fs::rename(&backup, &core).is_err() {
                if failed.exists() {
                    let _ = fs::rename(&failed, &core);
                }
                return Err("恢复安装中断前的 CLIProxyAPI 内核失败。".into());
            }
        } else if !core.is_dir() {
            return Err("CLIProxyAPI 安装事务缺少可恢复的上一版内核。".into());
        }
    } else {
        remove_private_tree(&core)?;
    }

    write_core_metadata_for_root(root, &journal.previous)?;
    remove_private_tree(&failed)?;
    fs::remove_file(&journal_path)
        .map_err(|_| "清理 CLIProxyAPI 安装事务标记失败。".to_string())?;
    Ok(())
}

/// Completes a health-check rollback after an interruption. In particular,
/// this prevents an old restored binary from being paired with the new
/// version's metadata when the process dies between the directory rename and
/// metadata commit.
fn recover_incomplete_core_rollback(root: &Path) -> Result<(), String> {
    let journal_path = rollback_journal_path(root);
    let journal_metadata = match fs::symlink_metadata(&journal_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("读取 CLIProxyAPI 回滚事务标记失败。".into()),
    };
    if !journal_metadata.is_file()
        || journal_metadata.file_type().is_symlink()
        || journal_metadata.len() > 64 * 1024
    {
        return Err("CLIProxyAPI 回滚事务标记无效；为安全起见未启动内核。".into());
    }
    let journal: CoreRollbackJournal = serde_json::from_slice(
        &fs::read(&journal_path).map_err(|_| "读取 CLIProxyAPI 回滚事务标记失败。".to_string())?,
    )
    .map_err(|_| "CLIProxyAPI 回滚事务标记损坏；为安全起见未启动内核。".to_string())?;
    let core = root.join("core");
    let failed = transaction_core_path(root, &journal.failed_core_dir, ".failed-core-")?;
    let backup = transaction_core_path(root, &journal.backup_core_dir, ".core-backup-")?;
    let current = read_core_metadata_from_root(root)?;
    if current != journal.restored {
        if backup.is_dir() {
            if core.exists() {
                if failed.exists() {
                    return Err("CLIProxyAPI 回滚事务状态冲突；为安全起见未自动覆盖内核。".into());
                }
                fs::rename(&core, &failed)
                    .map_err(|_| "隔离未通过健康检查的 CLIProxyAPI 内核失败。".to_string())?;
            }
            if fs::rename(&backup, &core).is_err() {
                if failed.exists() {
                    let _ = fs::rename(&failed, &core);
                }
                return Err("恢复上一版 CLIProxyAPI 内核失败。".into());
            }
        } else if !core.is_dir() {
            return Err("CLIProxyAPI 回滚事务缺少可恢复的上一版内核。".into());
        }
        write_core_metadata_for_root(root, &journal.restored)?;
    } else if !core.is_dir() {
        return Err("CLIProxyAPI 回滚元数据已提交，但内核目录缺失。".into());
    }

    remove_private_tree(&failed)?;
    fs::remove_file(&journal_path)
        .map_err(|_| "清理 CLIProxyAPI 回滚事务标记失败。".to_string())?;
    Ok(())
}

fn previous_core_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    transaction_core_path(root, name, ".core-backup-")
        .map_err(|_| "CLIProxyAPI 回滚元数据无效。".to_string())
}

fn finalize_core_update(data_dir: &Path) -> Result<(), String> {
    let root = core_root(data_dir);
    let mut metadata = read_core_metadata_from_root(&root)?;
    let Some(previous_name) = metadata.previous_core_dir.take() else {
        return Ok(());
    };
    let previous = previous_core_path(&root, &previous_name)?;
    metadata.previous = None;
    write_core_metadata(data_dir, &metadata)?;
    let _ = fs::remove_dir_all(previous);
    Ok(())
}

fn rollback_core_update(data_dir: &Path) -> Result<bool, String> {
    let root = core_root(data_dir);
    let metadata = read_core_metadata_from_root(&root)?;
    let Some(previous_name) = metadata.previous_core_dir.as_deref() else {
        return Ok(false);
    };
    let previous = previous_core_path(&root, previous_name)?;
    if !previous.is_dir() {
        return Err("CLIProxyAPI 上一版内核缺失，无法自动回滚。".into());
    }
    let mut restored = metadata.previous.map(|value| *value).unwrap_or_default();
    restored.latest_version = metadata.latest_version;
    restored.previous_core_dir = None;
    restored.previous = None;
    let failed_name = format!(".failed-core-{}", Uuid::new_v4());
    write_rollback_journal(
        &root,
        &CoreRollbackJournal {
            backup_core_dir: previous_name.to_owned(),
            failed_core_dir: failed_name,
            restored,
        },
    )?;
    recover_incomplete_core_rollback(&root)?;
    Ok(true)
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "设置 CLIProxyAPI 内核权限失败。".to_string())?;
    }
    Ok(())
}

async fn start_sidecar(
    row: CodexProviderRow,
    upstream_key: String,
    token: Zeroizing<String>,
    port: u16,
    data_dir: &Path,
) -> Result<ServerHandle, String> {
    let core = core_dir(data_dir);
    let binary = find_unique_core_binary(&core)?;
    ensure_private_dir(&core_root(data_dir))?;
    let port_guard = reserve_loopback_port(port)?;
    let runtime_root = runtime_dir(data_dir);
    ensure_private_dir(&runtime_root)?;
    let runtime = runtime_root.join(format!("session-{}", Uuid::new_v4()));
    ensure_private_dir(&runtime)?;
    ensure_private_dir(&runtime.join("auth"))?;
    ensure_private_dir(&runtime.join("logs"))?;
    let config = sidecar_config(&row, &upstream_key, &token, port, &runtime);
    atomic_private_write(&runtime.join("config.yaml"), config.as_bytes())?;
    drop(port_guard);
    let child = match Command::new(&binary)
        .arg("-config")
        .arg(runtime.join("config.yaml"))
        .arg("-local-model")
        .current_dir(&core)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            let _ = fs::remove_dir_all(&runtime);
            return Err("启动 CLIProxyAPI 内核失败。".into());
        }
    };
    let mut handle = ServerHandle {
        port,
        source: "external-provider".into(),
        provider_id: Some(row.id),
        provider_name: row.name,
        started_at: Utc::now().to_rfc3339(),
        endpoint: format!("http://127.0.0.1:{port}/v1"),
        token,
        sidecar: Some(SidecarProcess {
            child: Mutex::new(child),
            data_dir: data_dir.to_path_buf(),
            runtime_dir: runtime,
        }),
        proxy_auth_store: None,
        proxy_auth_runtime: None,
    };
    if let Err(error) = wait_for_health(&mut handle).await {
        let _ = handle.stop().await;
        return Err(error);
    }
    Ok(handle)
}

async fn start_oauth_sidecar(
    token: Zeroizing<String>,
    port: u16,
    data_dir: &Path,
    port_guard: TcpListener,
    proxy_auth_store: Arc<ProxyAuthStore>,
    materialization: RuntimeMaterialization,
) -> Result<ServerHandle, String> {
    let core = core_dir(data_dir);
    let binary = find_unique_core_binary(&core)?;
    ensure_private_dir(&core_root(data_dir))?;
    let runtime_root = runtime_dir(data_dir);
    ensure_private_dir(&runtime_root)?;
    let runtime = runtime_root.join(format!("session-{}", Uuid::new_v4()));
    ensure_private_dir(&runtime)?;
    ensure_private_dir(&runtime.join("logs"))?;
    let config = oauth_sidecar_config(&token, port, materialization.auth_dir(), &runtime);
    atomic_private_write(&runtime.join("config.yaml"), config.as_bytes())?;
    drop(port_guard);
    let child = match Command::new(&binary)
        .arg("-config")
        .arg(runtime.join("config.yaml"))
        .arg("-local-model")
        .current_dir(&core)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            let _ = fs::remove_dir_all(&runtime);
            let _ = proxy_auth_store.checkpoint_materialized_runtime(&materialization);
            return Err("启动 CLIProxyAPI 内核失败。".into());
        }
    };
    let mut handle = ServerHandle {
        port,
        source: "oauth-pool".into(),
        provider_id: None,
        provider_name: "CLIProxyAPI OAuth 凭据池".into(),
        started_at: Utc::now().to_rfc3339(),
        endpoint: format!("http://127.0.0.1:{port}/v1"),
        token,
        sidecar: Some(SidecarProcess {
            child: Mutex::new(child),
            data_dir: data_dir.to_path_buf(),
            runtime_dir: runtime,
        }),
        proxy_auth_store: Some(proxy_auth_store),
        proxy_auth_runtime: Some(materialization),
    };
    if let Err(error) = wait_for_health(&mut handle).await {
        let _ = handle.stop().await;
        return Err(error);
    }
    Ok(handle)
}

async fn wait_for_health(handle: &mut ServerHandle) -> Result<(), String> {
    let client = Client::builder()
        .no_proxy()
        .build()
        .map_err(|_| "创建 CLIProxyAPI 健康检查客户端失败。".to_string())?;
    let deadline = tokio::time::Instant::now() + CORE_HEALTH_TIMEOUT;
    loop {
        if handle.sidecar.as_ref().is_some_and(|sidecar| {
            sidecar
                .child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok().flatten())
                .is_some()
        }) {
            return Err("CLIProxyAPI 内核启动后异常退出。".into());
        }
        let healthy = client
            .get(format!("http://127.0.0.1:{}/healthz", handle.port))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", handle.token.as_str()),
            )
            .send()
            .await
            .map(|response| response.status() == reqwest::StatusCode::OK)
            .unwrap_or(false);
        if healthy {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if handle.sidecar.as_ref().is_some_and(|sidecar| {
                sidecar
                    .child
                    .lock()
                    .ok()
                    .and_then(|mut child| child.try_wait().ok().flatten())
                    .is_some()
            }) {
                return Err("CLIProxyAPI 健康端口不属于新启动的受管进程。".into());
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("CLIProxyAPI 内核健康检查超时。".into());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn stop_sidecar(sidecar: SidecarProcess) -> Result<(), String> {
    let mut child = sidecar
        .child
        .lock()
        .map_err(|_| "CLIProxyAPI 进程状态不可用。".to_string())?;
    if child
        .try_wait()
        .map_err(|_| "读取 CLIProxyAPI 进程状态失败。".to_string())?
        .is_some()
    {
        let _ = fs::remove_dir_all(sidecar.runtime_dir);
        return Ok(());
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(windows)]
    child
        .kill()
        .map_err(|_| "停止 CLIProxyAPI 进程失败。".to_string())?;
    let deadline = std::time::Instant::now() + CORE_STOP_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if child
            .try_wait()
            .map_err(|_| "读取 CLIProxyAPI 进程状态失败。".to_string())?
            .is_some()
        {
            let _ = fs::remove_dir_all(&sidecar.runtime_dir);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child
        .kill()
        .map_err(|_| "强制停止 CLIProxyAPI 进程失败。".to_string())?;
    let _ = child.wait();
    let _ = fs::remove_dir_all(sidecar.runtime_dir);
    Ok(())
}

fn sidecar_config(
    row: &CodexProviderRow,
    api_key: &str,
    token: &str,
    port: u16,
    runtime: &Path,
) -> String {
    let quote = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
    format!(
        "host: \"127.0.0.1\"\nport: {port}\nauth-dir: {}\napi-keys:\n  - {}\ndebug: false\ncommercial-mode: true\nrequest-log: false\nlogging-to-file: false\nusage-statistics-enabled: false\nrequest-retry: 0\nplugins:\n  enabled: false\n  dir: {}\nremote-management:\n  allow-remote: false\n  secret-key: \"\"\n  disable-control-panel: true\n  disable-auto-update-panel: true\nopenai-compatibility:\n  - name: \"codex-manager\"\n    disabled: false\n    base-url: {}\n    api-key-entries:\n      - api-key: {}\n    models:\n      - name: {}\n        alias: {}\n",
        quote(&runtime.join("auth").to_string_lossy()),
        quote(token),
        quote(&runtime.join("plugins").to_string_lossy()),
        quote(&row.base_url),
        quote(api_key),
        quote(&row.model),
        quote(&row.model)
    )
}

fn oauth_sidecar_config(token: &str, port: u16, auth_dir: &Path, runtime: &Path) -> String {
    let quote = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into());
    format!(
        "host: \"127.0.0.1\"\nport: {port}\nauth-dir: {}\napi-keys:\n  - {}\ndebug: false\ncommercial-mode: true\nrequest-log: false\nlogging-to-file: false\nusage-statistics-enabled: false\nrequest-retry: 0\nplugins:\n  enabled: false\n  dir: {}\nremote-management:\n  allow-remote: false\n  secret-key: \"\"\n  disable-control-panel: true\n  disable-auto-update-panel: true\n",
        quote(&auth_dir.to_string_lossy()),
        quote(token),
        quote(&runtime.join("plugins").to_string_lossy()),
    )
}

#[derive(Default)]
struct CoreStatusFields {
    state: &'static str,
    source: Option<String>,
    provider_id: Option<String>,
    provider_name: Option<String>,
    endpoint: Option<String>,
    started_at: Option<String>,
    last_error: Option<String>,
    process_id: Option<u32>,
    installing: bool,
}

fn status_from_core(data_dir: &Path, port: u16, mut fields: CoreStatusFields) -> GatewayStatus {
    let metadata = match read_core_metadata(data_dir) {
        Ok(metadata) => metadata,
        Err(error) => {
            fields.state = "error";
            if fields.last_error.is_none() {
                fields.last_error = Some(error);
            }
            CoreMetadata::default()
        }
    };
    let installed = metadata.core_version.is_some()
        && fields.last_error.is_none()
        && find_unique_core_binary(&core_dir(data_dir)).is_ok();
    GatewayStatus {
        state: fields.state.into(),
        source: fields.source.unwrap_or_else(|| "none".into()),
        provider_id: fields.provider_id,
        provider_name: fields.provider_name,
        endpoint: fields.endpoint,
        started_at: fields.started_at,
        requests: None,
        failed: None,
        in_flight: None,
        last_error: fields.last_error,
        port,
        installed,
        core_version: metadata.core_version.clone(),
        latest_version: metadata.latest_version.clone(),
        update_available: metadata.core_version.is_some()
            && metadata.latest_version.is_some()
            && metadata.core_version != metadata.latest_version,
        installing: fields.installing,
        process_id: fields.process_id,
        core_source: format!("GitHub Release: {CORE_REPOSITORY}"),
    }
}

pub fn stopped_status_with_core(
    data_dir: &Path,
    port: u16,
    last_error: Option<String>,
    installing: bool,
) -> GatewayStatus {
    cleanup_stale_runtime(data_dir);
    status_from_core(
        data_dir,
        port,
        CoreStatusFields {
            state: if last_error.is_some() {
                "error"
            } else {
                "stopped"
            },
            last_error,
            installing,
            ..CoreStatusFields::default()
        },
    )
}

fn gateway_token(keychain: Arc<dyn KeychainBackend>) -> Result<Zeroizing<String>, String> {
    let existing = keychain
        .read(KEYCHAIN_GATEWAY_TOKEN)
        .map_err(public_error)?;
    let value = match existing {
        Some(value) => {
            String::from_utf8(value.to_vec()).map_err(|_| "本地反代凭据无效。".to_string())?
        }
        None => {
            let generated = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            keychain
                .write(KEYCHAIN_GATEWAY_TOKEN, generated.as_bytes())
                .map_err(public_error)?;
            generated
        }
    };
    if value.len() < 32 || value.len() > 256 || value.chars().any(char::is_control) {
        return Err("本地反代凭据无效。".into());
    }
    Ok(Zeroizing::new(value))
}

/// Resolves one allowlisted `auth.command` reference for a native Codex
/// process. The caller must write the returned value directly to stdout and
/// must never expose it through a Tauri command or ordinary DTO.
pub fn credential_helper_secret(secret_ref: &str) -> Result<Zeroizing<String>, String> {
    credential_helper_secret_with_keychain(secret_ref, mac_keychain())
}

fn credential_helper_secret_with_keychain(
    secret_ref: &str,
    keychain: Arc<dyn KeychainBackend>,
) -> Result<Zeroizing<String>, String> {
    if secret_ref == "local-proxy-client" {
        return gateway_token(keychain);
    }
    let provider_id = secret_ref
        .strip_prefix("external-provider:")
        .ok_or_else(|| "凭据引用不受支持。".to_string())?;
    Uuid::parse_str(provider_id).map_err(|_| "凭据引用无效。".to_string())?;
    let secret = keychain
        .read(&provider_account(provider_id))
        .map_err(public_error)?
        .ok_or_else(|| "凭据不存在。".to_string())?;
    let value = String::from_utf8(secret.to_vec()).map_err(|_| "凭据无效。".to_string())?;
    if value.is_empty() || value.len() > 32 * 1024 || value.chars().any(char::is_control) {
        return Err("凭据无效。".into());
    }
    Ok(Zeroizing::new(value))
}
#[cfg(test)]
mod sidecar_tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryKeychain {
        entries: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl KeychainBackend for MemoryKeychain {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, GatewayError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(Zeroizing::new))
        }

        fn write(&self, account: &str, value: &[u8]) -> Result<(), GatewayError> {
            self.entries
                .lock()
                .unwrap()
                .insert(account.into(), value.to_vec());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), GatewayError> {
            self.entries.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[test]
    fn release_asset_requires_exact_platform_digest_and_non_plugin_asset() {
        let digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let release = GithubRelease {
            tag_name: "v7.2.145".into(),
            assets: vec![
                GithubAsset {
                    name: "checksums.txt".into(),
                    browser_download_url: "https://example.test/checksums".into(),
                    digest: Some(digest.into()),
                },
                GithubAsset {
                    name: format!(
                        "CLIProxyAPI_7.2.145_{}_{}.{}",
                        if cfg!(target_os = "macos") {
                            "darwin"
                        } else if cfg!(target_os = "linux") {
                            "linux"
                        } else {
                            "windows"
                        },
                        if cfg!(target_arch = "aarch64") {
                            "aarch64"
                        } else {
                            "amd64"
                        },
                        if cfg!(target_os = "windows") {
                            "zip"
                        } else {
                            "tar.gz"
                        }
                    ),
                    browser_download_url: "https://example.test/core".into(),
                    digest: Some(digest.into()),
                },
            ],
        };
        assert!(select_release_asset(release.clone()).is_ok());
        let mut prerelease = release.clone();
        prerelease.tag_name = "v7.2.145-rc1".into();
        assert!(select_release_asset(prerelease).is_err());
        let mut mismatched = release;
        mismatched.tag_name = "v7.2.146".into();
        assert!(select_release_asset(mismatched).is_err());
    }

    #[test]
    fn checksum_and_archive_paths_fail_closed() {
        let digest = "a".repeat(64);
        assert_eq!(
            checksum_for_asset(&format!("{digest}  core.tar.gz"), "core.tar.gz").unwrap(),
            digest
        );
        assert!(checksum_for_asset("not-a-digest  core.tar.gz", "core.tar.gz").is_err());
        assert!(safe_archive_path(Path::new("../escape")).is_err());
        assert!(safe_archive_path(Path::new("/absolute")).is_err());
    }

    #[test]
    fn generated_config_is_loopback_and_private_by_contract() {
        let row = CodexProviderRow {
            id: "id".into(),
            name: "provider".into(),
            base_url: "https://example.test/v1".into(),
            model: "model".into(),
            reasoning_effort: "medium".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        };
        let config = sidecar_config(
            &row,
            "provider-secret",
            "local-secret",
            8317,
            Path::new("/tmp/runtime"),
        );
        for expected in [
            "host: \"127.0.0.1\"",
            "commercial-mode: true",
            "request-log: false",
            "logging-to-file: false",
            "usage-statistics-enabled: false",
            "request-retry: 0",
            "secret-key: \"\"",
            "disable-control-panel: true",
            "disable-auto-update-panel: true",
            "enabled: false",
        ] {
            assert!(config.contains(expected), "{expected}");
        }
        let status = stopped_status_with_core(Path::new("/not-installed"), 8317, None, false);
        let dto = serde_json::to_string(&status).unwrap();
        assert!(!dto.contains("provider-secret") && !dto.contains("local-secret"));
        assert!(status.requests.is_none() && status.failed.is_none() && status.in_flight.is_none());
    }

    #[test]
    fn oauth_config_uses_projected_auth_dir_without_management_or_external_key() {
        let config = oauth_sidecar_config(
            "local-client-secret",
            8317,
            Path::new("/tmp/private-oauth"),
            Path::new("/tmp/runtime"),
        );
        assert!(config.contains("auth-dir: \"/tmp/private-oauth\""));
        assert!(config.contains("host: \"127.0.0.1\""));
        assert!(config.contains("allow-remote: false"));
        assert!(!config.contains("openai-compatibility"));
        assert!(!config.contains("provider-secret"));
    }

    #[test]
    fn credential_helper_accepts_only_fixed_refs_and_never_permits_arbitrary_keychain_items() {
        let keychain = Arc::new(MemoryKeychain::default());
        let provider_id = Uuid::new_v4().to_string();
        keychain
            .write(&provider_account(&provider_id), b"provider-secret")
            .unwrap();
        assert_eq!(
            credential_helper_secret_with_keychain(
                &format!("external-provider:{provider_id}"),
                keychain.clone(),
            )
            .unwrap()
            .as_str(),
            "provider-secret"
        );
        let local =
            credential_helper_secret_with_keychain("local-proxy-client", keychain.clone()).unwrap();
        assert!(local.len() >= 32);
        for rejected in [
            "provider:arbitrary",
            "external-provider:not-a-uuid",
            KEYCHAIN_GATEWAY_TOKEN,
        ] {
            assert!(credential_helper_secret_with_keychain(rejected, keychain.clone()).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_paths_use_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("runtime");
        ensure_private_dir(&runtime).unwrap();
        let config = runtime.join("config.yaml");
        atomic_private_write(&config, b"sensitive: value").unwrap();
        assert_eq!(
            fs::metadata(&runtime).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let linked_directory = directory.path().join("linked-runtime");
        symlink(&runtime, &linked_directory).unwrap();
        assert!(ensure_private_dir(&linked_directory).is_err());
        let linked_file = directory.path().join("linked-config.yaml");
        symlink(&config, &linked_file).unwrap();
        assert!(private_file(&linked_file).is_err());
    }

    #[test]
    fn failed_install_switch_restores_previous_core() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core");
        fs::create_dir_all(&core).unwrap();
        fs::write(core.join("old-marker"), b"old").unwrap();
        let backup = directory.path().join(".core-backup-test");
        assert!(
            replace_core_directory(&core, &directory.path().join("missing"), Some(&backup))
                .is_err()
        );
        assert_eq!(fs::read(core.join("old-marker")).unwrap(), b"old");
    }

    #[test]
    fn interrupted_install_transaction_restores_previous_core_and_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let root = core_root(directory.path());
        let core = root.join("core");
        let backup_name = ".core-backup-transaction-test";
        let backup = root.join(backup_name);
        ensure_private_dir(&core).unwrap();
        ensure_private_dir(&backup).unwrap();
        fs::write(core.join("new-marker"), b"new").unwrap();
        fs::write(backup.join("old-marker"), b"old").unwrap();
        let previous = CoreMetadata {
            core_version: Some("7.2.145".into()),
            ..CoreMetadata::default()
        };
        write_core_metadata(directory.path(), &previous).unwrap();
        let pending = CoreMetadata {
            core_version: Some("7.2.146".into()),
            previous_core_dir: Some(backup_name.into()),
            previous: Some(Box::new(previous.clone())),
            ..CoreMetadata::default()
        };
        write_install_journal(
            &root,
            &CoreInstallJournal {
                backup_core_dir: Some(backup_name.into()),
                failed_core_dir: ".failed-core-transaction-test".into(),
                previous: previous.clone(),
                pending,
            },
        )
        .unwrap();

        recover_incomplete_core_install(&root).unwrap();

        assert_eq!(fs::read(core.join("old-marker")).unwrap(), b"old");
        assert!(!core.join("new-marker").exists());
        assert_eq!(read_core_metadata(directory.path()).unwrap(), previous);
        assert!(!install_journal_path(&root).exists());
    }

    #[test]
    fn committed_install_transaction_keeps_pending_health_rollback() {
        let directory = tempfile::tempdir().unwrap();
        let root = core_root(directory.path());
        let core = root.join("core");
        let backup_name = ".core-backup-committed-test";
        let backup = root.join(backup_name);
        ensure_private_dir(&core).unwrap();
        ensure_private_dir(&backup).unwrap();
        fs::write(core.join("new-marker"), b"new").unwrap();
        fs::write(backup.join("old-marker"), b"old").unwrap();
        let previous = CoreMetadata {
            core_version: Some("7.2.145".into()),
            ..CoreMetadata::default()
        };
        let pending = CoreMetadata {
            core_version: Some("7.2.146".into()),
            previous_core_dir: Some(backup_name.into()),
            previous: Some(Box::new(previous.clone())),
            ..CoreMetadata::default()
        };
        write_core_metadata(directory.path(), &pending).unwrap();
        write_install_journal(
            &root,
            &CoreInstallJournal {
                backup_core_dir: Some(backup_name.into()),
                failed_core_dir: ".failed-core-committed-test".into(),
                previous,
                pending: pending.clone(),
            },
        )
        .unwrap();

        recover_incomplete_core_install(&root).unwrap();

        assert_eq!(read_core_metadata(directory.path()).unwrap(), pending);
        assert_eq!(fs::read(core.join("new-marker")).unwrap(), b"new");
        assert_eq!(fs::read(backup.join("old-marker")).unwrap(), b"old");
        assert!(!install_journal_path(&root).exists());
    }

    #[test]
    fn occupied_loopback_port_fails_before_sidecar_spawn() {
        let first = reserve_loopback_port(0).unwrap();
        let port = first.local_addr().unwrap().port();
        assert!(reserve_loopback_port(port).is_err());
    }

    #[test]
    fn stale_runtime_secrets_are_removed_before_a_new_session() {
        let directory = tempfile::tempdir().unwrap();
        let session = runtime_dir(directory.path()).join("session-stale");
        ensure_private_dir(&session).unwrap();
        atomic_private_write(&session.join("config.yaml"), b"api-key: secret").unwrap();
        cleanup_stale_runtime(directory.path());
        assert!(!runtime_dir(directory.path()).exists());
    }

    #[test]
    fn failed_new_core_health_can_restore_previous_version() {
        let directory = tempfile::tempdir().unwrap();
        let root = core_root(directory.path());
        let core = root.join("core");
        let backup_name = ".core-backup-test";
        let backup = root.join(backup_name);
        ensure_private_dir(&core).unwrap();
        ensure_private_dir(&backup).unwrap();
        fs::write(core.join("new-marker"), b"new").unwrap();
        fs::write(backup.join("old-marker"), b"old").unwrap();
        write_core_metadata(
            directory.path(),
            &CoreMetadata {
                core_version: Some("7.2.146".into()),
                latest_version: Some("7.2.146".into()),
                previous_core_dir: Some(backup_name.into()),
                previous: Some(Box::new(CoreMetadata {
                    core_version: Some("7.2.145".into()),
                    ..CoreMetadata::default()
                })),
                ..CoreMetadata::default()
            },
        )
        .unwrap();
        assert!(rollback_core_update(directory.path()).unwrap());
        assert_eq!(fs::read(core.join("old-marker")).unwrap(), b"old");
        assert!(!core.join("new-marker").exists());
        let restored = read_core_metadata(directory.path()).unwrap();
        assert_eq!(restored.core_version.as_deref(), Some("7.2.145"));
        assert_eq!(restored.latest_version.as_deref(), Some("7.2.146"));
        assert!(restored.previous_core_dir.is_none());
    }

    #[test]
    fn interrupted_health_rollback_repairs_restored_core_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let root = core_root(directory.path());
        let core = root.join("core");
        let failed_name = ".failed-core-rollback-interrupted-test";
        let failed = root.join(failed_name);
        ensure_private_dir(&core).unwrap();
        ensure_private_dir(&failed).unwrap();
        fs::write(core.join("old-marker"), b"old").unwrap();
        fs::write(failed.join("new-marker"), b"new").unwrap();
        let restored = CoreMetadata {
            core_version: Some("7.2.145".into()),
            latest_version: Some("7.2.146".into()),
            ..CoreMetadata::default()
        };
        write_core_metadata(
            directory.path(),
            &CoreMetadata {
                core_version: Some("7.2.146".into()),
                latest_version: Some("7.2.146".into()),
                previous_core_dir: Some(".core-backup-rollback-interrupted-test".into()),
                previous: Some(Box::new(restored.clone())),
                ..CoreMetadata::default()
            },
        )
        .unwrap();
        write_rollback_journal(
            &root,
            &CoreRollbackJournal {
                backup_core_dir: ".core-backup-rollback-interrupted-test".into(),
                failed_core_dir: failed_name.into(),
                restored: restored.clone(),
            },
        )
        .unwrap();

        recover_incomplete_core_rollback(&root).unwrap();

        assert_eq!(fs::read(core.join("old-marker")).unwrap(), b"old");
        assert_eq!(read_core_metadata(directory.path()).unwrap(), restored);
        assert!(!failed.exists());
        assert!(!rollback_journal_path(&root).exists());
    }

    #[tokio::test]
    #[ignore = "downloads the current official CLIProxyAPI GitHub Release"]
    async fn official_release_download_and_extract_uses_the_real_installer() {
        let directory = tempfile::tempdir().unwrap();
        check_latest_core(directory.path()).await.unwrap();
        install_latest_core(directory.path()).await.unwrap();
        let metadata = read_core_metadata(directory.path()).unwrap();
        assert_eq!(metadata.core_version, metadata.latest_version);
        assert!(find_unique_core_binary(&core_dir(directory.path())).is_ok());
    }
}
