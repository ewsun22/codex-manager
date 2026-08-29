//! Local, loopback-only Responses gateway for one selected Codex provider.
//!
//! The WebView sees provider metadata only. API keys and the installation-scoped
//! loopback bearer remain in the platform Keychain / server handle and are
//! never included in ordinary DTOs or log messages.

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use codex_storage::{CodexProviderRow, Store};
use futures_util::StreamExt;
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::oneshot, time::timeout};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYCHAIN_SERVICE: &str = "cc.codex.manager.providers.v1";
const KEYCHAIN_PREFIX: &str = "provider:";
const KEYCHAIN_GATEWAY_TOKEN: &str = "gateway-client-token";
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONCURRENT_REQUESTS: usize = 4;
const MAX_FIELD_BYTES: usize = 256;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(not(test))]
const RESPONSE_HEADERS_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg(test)]
const RESPONSE_HEADERS_TIMEOUT: Duration = Duration::from_millis(75);
#[cfg(not(test))]
const RESPONSE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const RESPONSE_IDLE_TIMEOUT: Duration = Duration::from_millis(75);

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
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub endpoint: Option<String>,
    pub started_at: Option<String>,
    pub requests: u64,
    pub failed: u64,
    pub in_flight: u32,
    pub last_error: Option<String>,
    pub port: u16,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySetup {
    pub port: u16,
    pub endpoint: String,
    /// Sensitive by design: returned only after an explicit native macOS
    /// confirmation. The token is deliberately not a separate DTO field.
    pub config_snippet: String,
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
    #[error("本地反代端口不可用；未启动服务。")]
    PortUnavailable,
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

#[derive(Default)]
struct Counters {
    requests: u64,
    failed: u64,
    in_flight: u32,
    last_error: Option<String>,
}
#[derive(Clone)]
struct RuntimeState {
    client: Client,
    upstream_base: String,
    upstream_key: Arc<Zeroizing<String>>,
    local_token: Arc<Zeroizing<String>>,
    local_authority: String,
    model: String,
    counters: Arc<Mutex<Counters>>,
}

pub struct ServerHandle {
    port: u16,
    provider_id: String,
    provider_name: String,
    model: String,
    reasoning_effort: String,
    started_at: String,
    endpoint: String,
    token: Zeroizing<String>,
    counters: Arc<Mutex<Counters>>,
    stop: Option<oneshot::Sender<()>>,
    join: Option<tauri::async_runtime::JoinHandle<()>>,
}
impl ServerHandle {
    pub fn status(&self) -> GatewayStatus {
        let counters = self
            .counters
            .lock()
            .map(|value| {
                (
                    value.requests,
                    value.failed,
                    value.in_flight,
                    value.last_error.clone(),
                )
            })
            .unwrap_or((0, 0, 0, Some("本地反代状态不可用。".into())));
        GatewayStatus {
            state: "running".into(),
            provider_id: Some(self.provider_id.clone()),
            provider_name: Some(self.provider_name.clone()),
            endpoint: Some(self.endpoint.clone()),
            started_at: Some(self.started_at.clone()),
            requests: counters.0,
            failed: counters.1,
            in_flight: counters.2,
            last_error: counters.3,
            port: self.port,
        }
    }
    pub fn setup(&self) -> GatewaySetup {
        GatewaySetup {
            port: self.port,
            endpoint: self.endpoint.clone(),
            config_snippet: format!(
                "model_provider = \"local-gateway\"\nmodel = {}\nmodel_reasoning_effort = {}\n\n[model_providers.local-gateway]\nname = \"Codex Manager Local Gateway\"\nbase_url = {}\nwire_api = \"responses\"\nexperimental_bearer_token = {}",
                toml_quote(&self.model),
                toml_quote(&self.reasoning_effort),
                toml_quote(&self.endpoint),
                toml_quote(&self.token)
            ),
        }
    }
    pub async fn stop(mut self) -> Result<(), String> {
        if let Some(sender) = self.stop.take() {
            let _ = sender.send(());
        }
        let Some(mut join) = self.join.take() else {
            return Ok(());
        };
        match timeout(SHUTDOWN_TIMEOUT, &mut join).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("本地反代停止失败；端口状态未知。".into()),
            Err(_) => {
                join.abort();
                let _ = timeout(SHUTDOWN_TIMEOUT, &mut join).await;
                Err("本地反代停止超时；已中止服务。".into())
            }
        }
    }
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(sender) = self.stop.take() {
            let _ = sender.send(());
        }
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

pub async fn start(store: &Store, provider_id: &str, port: u16) -> Result<ServerHandle, String> {
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
    start_with_secret(row, upstream_key, token, port)
        .await
        .map_err(public_error)
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
async fn start_with_secret(
    row: CodexProviderRow,
    upstream_key: String,
    token: Zeroizing<String>,
    port: u16,
) -> Result<ServerHandle, GatewayError> {
    if !(1024..=65535).contains(&port) {
        return Err(GatewayError::PortUnavailable);
    }
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        .await
        .map_err(|_| GatewayError::PortUnavailable)?;
    let endpoint = format!("http://127.0.0.1:{port}/v1");
    let upstream = Url::parse(&row.base_url).map_err(|_| GatewayError::InvalidProvider)?;
    let remote_host = upstream.host_str().ok_or(GatewayError::InvalidProvider)?;
    let resolved = if upstream.scheme() == "https" {
        Some(resolve_safe_remote(
            remote_host,
            upstream.port_or_known_default().unwrap(),
        )?)
    } else {
        None
    };
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10));
    if let Some(address) = resolved {
        builder = builder.resolve(remote_host, address);
    }
    let client = builder.build().map_err(|_| GatewayError::PortUnavailable)?;
    let counters = Arc::new(Mutex::new(Counters::default()));
    let runtime = RuntimeState {
        client,
        upstream_base: row.base_url.clone(),
        upstream_key: Arc::new(Zeroizing::new(upstream_key)),
        local_token: Arc::new(token.clone()),
        local_authority: format!("127.0.0.1:{port}"),
        model: row.model.clone(),
        counters: counters.clone(),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(models))
        .route("/v1/responses", post(responses))
        .route("/v1/responses/compact", post(compact))
        .with_state(runtime);
    let (stop_tx, stop_rx) = oneshot::channel();
    let join = tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await;
    });
    Ok(ServerHandle {
        port,
        provider_id: row.id,
        provider_name: row.name,
        model: row.model,
        reasoning_effort: row.reasoning_effort,
        started_at: Utc::now().to_rfc3339(),
        endpoint,
        token,
        counters,
        stop: Some(stop_tx),
        join: Some(join),
    })
}

async fn health(State(state): State<RuntimeState>, request: Request) -> Response {
    if !authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"status\":\"ok\"}",
    )
        .into_response()
}
async fn models(State(state): State<RuntimeState>, request: Request) -> Response {
    if !authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = format!(
        "{{\"object\":\"list\",\"data\":[{{\"id\":{},\"object\":\"model\"}}]}}",
        serde_json::to_string(&state.model).unwrap()
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}
async fn responses(State(state): State<RuntimeState>, request: Request) -> Response {
    forward(state, request, "responses").await
}
async fn compact(State(state): State<RuntimeState>, request: Request) -> Response {
    forward(state, request, "responses/compact").await
}
fn authorized(state: &RuntimeState, headers: &HeaderMap) -> bool {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.local_authority);
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| value == format!("http://{}", state.local_authority));
    let candidate = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    host_ok
        && origin_ok
        && candidate
            .as_bytes()
            .ct_eq(state.local_token.as_bytes())
            .into()
}
async fn forward(state: RuntimeState, request: Request, path: &str) -> Response {
    if !authorized(&state, request.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let permit = {
        let mut counters = match state.counters.lock() {
            Ok(value) => value,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        if counters.in_flight as usize >= MAX_CONCURRENT_REQUESTS {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        counters.requests += 1;
        counters.in_flight += 1;
        InFlight(state.counters.clone())
    };
    let bytes = match to_bytes(request.into_body(), MAX_BODY_BYTES).await {
        Ok(value) => value,
        Err(_) => {
            mark_failure(&state);
            drop(permit);
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    };
    let url = format!("{}/{}", state.upstream_base, path);
    let request = state
        .client
        .post(url)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.upstream_key.as_str()),
        )
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream, application/json")
        .body(bytes);
    let response = timeout(RESPONSE_HEADERS_TIMEOUT, request.send()).await;
    let response = match response {
        Ok(Ok(value)) => value,
        Ok(Err(_)) | Err(_) => {
            mark_failure(&state);
            drop(permit);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    if !response.status().is_success() {
        mark_failure(&state);
    }
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for key in [
        header::CONTENT_TYPE,
        header::CACHE_CONTROL,
        header::HeaderName::from_static("x-request-id"),
    ] {
        if let Some(value) = response.headers().get(&key) {
            headers.insert(key, value.clone());
        }
    }
    let stream = futures_util::stream::unfold(
        (Some(response.bytes_stream()), Some(permit), state.clone()),
        |(stream, permit, state)| async move {
            let mut stream = stream?;
            match timeout(RESPONSE_IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(bytes))) => Some((
                    Ok::<_, std::io::Error>(bytes),
                    (Some(stream), permit, state),
                )),
                Ok(Some(Err(_))) => {
                    mark_failure(&state);
                    Some((
                        Err(std::io::Error::other("upstream response interrupted")),
                        (None, None, state),
                    ))
                }
                Ok(None) => None,
                Err(_) => {
                    mark_failure(&state);
                    Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "upstream response idle timeout",
                        )),
                        (None, None, state),
                    ))
                }
            }
        },
    );
    (status, headers, Body::from_stream(stream)).into_response()
}
struct InFlight(Arc<Mutex<Counters>>);
impl Drop for InFlight {
    fn drop(&mut self) {
        if let Ok(mut counters) = self.0.lock() {
            counters.in_flight = counters.in_flight.saturating_sub(1);
        }
    }
}
fn mark_failure(state: &RuntimeState) {
    if let Ok(mut counters) = state.counters.lock() {
        counters.failed += 1;
        counters.last_error = Some("上游请求失败。".into());
    }
}
fn toml_quote(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

pub fn stopped_status(port: u16, last_error: Option<String>) -> GatewayStatus {
    GatewayStatus {
        state: if last_error.is_some() {
            "error".into()
        } else {
            "stopped".into()
        },
        provider_id: None,
        provider_name: None,
        endpoint: None,
        started_at: None,
        requests: 0,
        failed: 0,
        in_flight: 0,
        last_error,
        port,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::http::HeaderValue;
    use std::{
        collections::BTreeMap,
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };

    type SeenRequestHeaders = Arc<Mutex<Option<(String, Option<String>)>>>;
    type HeaderTestState = (SeenRequestHeaders, Arc<AtomicUsize>);

    #[derive(Default)]
    struct MockKeychain {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
    }
    impl KeychainBackend for MockKeychain {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, GatewayError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(Zeroizing::new))
        }
        fn write(&self, account: &str, value: &[u8]) -> Result<(), GatewayError> {
            self.values
                .lock()
                .unwrap()
                .insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<(), GatewayError> {
            self.values.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[test]
    fn url_validation_is_strict_and_normalizes_v1() {
        assert!(normalize_base_url("http://example.com/v1").is_err());
        assert!(normalize_base_url("https://127.0.0.1/v1").is_err());
        assert!(normalize_base_url("https://169.254.169.254/v1").is_err());
        assert!(normalize_base_url("https://user@example.com/v1").is_err());
        assert!(normalize_base_url("https://example.com/v1?key=x").is_err());
        assert!(normalize_base_url("http://127.0.0.1:3000/").is_ok());
    }

    #[test]
    fn non_public_ip_space_is_denied_by_default() {
        for value in [
            "0.0.0.0",
            "10.0.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.0.2.1",
            "192.168.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
        ] {
            assert!(unsafe_remote_ip(value.parse().unwrap()), "{value}");
        }
        for value in [
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "2001:2::1",
            "2002:0a00::1",
            "3ffe::1",
        ] {
            assert!(unsafe_remote_ip(value.parse().unwrap()), "{value}");
        }
        assert!(!unsafe_remote_ip("8.8.8.8".parse().unwrap()));
        assert!(!unsafe_remote_ip("2606:4700:4700::1111".parse().unwrap()));
    }
    #[test]
    fn auth_is_checked_without_reading_a_body() {
        let state = RuntimeState {
            client: Client::new(),
            upstream_base: "https://example.test/v1".into(),
            upstream_key: Arc::new(Zeroizing::new("upstream".into())),
            local_token: Arc::new(Zeroizing::new("local".into())),
            local_authority: "127.0.0.1:8318".into(),
            model: "test".into(),
            counters: Arc::new(Mutex::new(Counters::default())),
        };
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer no"));
        assert!(!authorized(&state, &headers));
        assert_eq!(state.counters.lock().unwrap().requests, 0);
    }

    #[test]
    fn provider_dto_never_contains_the_api_key_and_empty_updates_preserve_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("providers.db")).unwrap();
        let keychain: Arc<dyn KeychainBackend> = Arc::new(MockKeychain::default());
        let created = save_with_keychain(
            &store,
            ProviderSaveInput {
                id: None,
                name: "Example".into(),
                base_url: "http://127.0.0.1:3000".into(),
                model: "test-model".into(),
                reasoning_effort: "medium".into(),
                api_key: Some("fixture-key-123".into()),
            },
            keychain.clone(),
        )
        .unwrap();
        assert!(created.has_api_key);
        assert!(
            !serde_json::to_string(&created)
                .unwrap()
                .contains("fixture-key-123")
        );
        let updated = save_with_keychain(
            &store,
            ProviderSaveInput {
                id: Some(created.id.clone()),
                name: "Example 2".into(),
                base_url: "http://127.0.0.1:3000/v1".into(),
                model: "test-model".into(),
                reasoning_effort: "high".into(),
                api_key: Some("".into()),
            },
            keychain.clone(),
        )
        .unwrap();
        assert!(updated.has_api_key);
        assert_eq!(
            keychain
                .read(&provider_account(&created.id))
                .unwrap()
                .as_deref()
                .unwrap()
                .as_slice(),
            b"fixture-key-123"
        );
        let listed = list_with_keychain(&store, keychain).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(
            !serde_json::to_string(&listed)
                .unwrap()
                .contains("fixture-key-123")
        );
    }

    #[test]
    fn keychain_restore_compensates_a_failed_metadata_step() {
        let keychain = MockKeychain::default();
        keychain.write("provider:test", b"old").unwrap();
        let old = keychain.read("provider:test").unwrap();
        keychain.delete("provider:test").unwrap();
        assert!(complete_metadata_delete(&keychain, "provider:test", old, Err(()),).is_err());
        assert_eq!(
            keychain
                .read("provider:test")
                .unwrap()
                .as_deref()
                .unwrap()
                .as_slice(),
            b"old"
        );
    }

    fn test_row(port: u16) -> CodexProviderRow {
        CodexProviderRow {
            id: Uuid::new_v4().to_string(),
            name: "test".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            model: "test-model".into(),
            reasoning_effort: "medium".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn gateway_rejects_bad_auth_before_body_and_strips_headers_while_streaming_sse() {
        let seen = Arc::new(Mutex::new(None::<(String, Option<String>)>));
        let upstream_count = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = listener.local_addr().unwrap().port();
        let app_state = (seen.clone(), upstream_count.clone());
        let app = Router::new()
            .route(
                "/v1/responses",
                post(
                    move |headers: HeaderMap,
                          State((seen, count)): State<HeaderTestState>| async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        *seen.lock().unwrap() = Some((
                            headers
                                .get(header::AUTHORIZATION)
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .to_owned(),
                            headers
                                .get("x-leak")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                        ));
                        (
                            [(header::CONTENT_TYPE, "text/event-stream")],
                            "data: {\\\"ok\\\":true}\\n\\n",
                        )
                    },
                ),
            )
            .with_state(app_state);
        let upstream = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let local = Zeroizing::new("local-token-with-enough-entropy-123456".to_string());
        let gateway = start_with_secret(
            test_row(upstream_port),
            "fixture-upstream-key".into(),
            local,
            port,
        )
        .await
        .unwrap();
        let endpoint = gateway.status().endpoint.unwrap();
        let client = Client::new();
        let denied = client
            .post(format!("{endpoint}/responses"))
            .header(header::AUTHORIZATION, "Bearer bad")
            .body("this body must not reach upstream")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(upstream_count.load(Ordering::SeqCst), 0);
        let models = client
            .get(format!("{endpoint}/models"))
            .header(
                header::AUTHORIZATION,
                "Bearer local-token-with-enough-entropy-123456",
            )
            .send()
            .await
            .unwrap();
        let models: serde_json::Value =
            serde_json::from_str(&models.text().await.unwrap()).unwrap();
        assert_eq!(models["data"][0]["id"], "test-model");
        let allowed = client
            .post(format!("{endpoint}/responses"))
            .header(
                header::AUTHORIZATION,
                "Bearer local-token-with-enough-entropy-123456",
            )
            .header("x-leak", "must-not-forward")
            .body("{\"stream\":true}")
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            allowed.text().await.unwrap(),
            "data: {\\\"ok\\\":true}\\n\\n"
        );
        let headers = seen.lock().unwrap().clone().unwrap();
        assert_eq!(headers.0, "Bearer fixture-upstream-key");
        assert_eq!(headers.1, None);
        gateway.stop().await.unwrap();
        upstream.abort();
    }

    #[tokio::test]
    async fn conflicting_port_fails_and_stop_releases_the_listener() {
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        assert!(
            start_with_secret(
                test_row(port),
                "key".into(),
                Zeroizing::new("local-token-with-enough-entropy-123456".into()),
                port
            )
            .await
            .is_err()
        );
        drop(reserved);
        let gateway = start_with_secret(
            test_row(port),
            "key".into(),
            Zeroizing::new("local-token-with-enough-entropy-123456".into()),
            port,
        )
        .await
        .unwrap();
        gateway.stop().await.unwrap();
        assert!(
            tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn response_header_timeout_releases_the_in_flight_slot_without_exposing_body() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();
        let upstream = tokio::spawn(async move {
            let _ = upstream_listener.accept().await;
            tokio::time::sleep(RESPONSE_HEADERS_TIMEOUT + RESPONSE_HEADERS_TIMEOUT).await;
        });
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let gateway = start_with_secret(
            test_row(upstream_port),
            "fixture-upstream-key".into(),
            Zeroizing::new("local-token-with-enough-entropy-123456".into()),
            port,
        )
        .await
        .unwrap();
        let response = timeout(
            Duration::from_secs(1),
            Client::new()
                .post(format!("{}/responses", gateway.status().endpoint.unwrap()))
                .header(
                    header::AUTHORIZATION,
                    "Bearer local-token-with-enough-entropy-123456",
                )
                .body("{}")
                .send(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let status = gateway.status();
        assert_eq!(status.in_flight, 0);
        assert_eq!(status.failed, 1);
        gateway.stop().await.unwrap();
        upstream.abort();
    }

    #[tokio::test]
    async fn response_stream_idle_timeout_marks_failure_and_releases_the_slot() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream_listener.local_addr().unwrap().port();
        let app = Router::new().route(
            "/v1/responses",
            post(|| async {
                let chunks = futures_util::stream::once(async {
                    Ok::<Bytes, Infallible>(Bytes::from_static(b"data: first\\n\\n"))
                })
                .chain(futures_util::stream::pending());
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(chunks),
                )
            }),
        );
        let upstream = tokio::spawn(async move {
            let _ = axum::serve(upstream_listener, app).await;
        });
        let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let gateway = start_with_secret(
            test_row(upstream_port),
            "fixture-upstream-key".into(),
            Zeroizing::new("local-token-with-enough-entropy-123456".into()),
            port,
        )
        .await
        .unwrap();
        let response = Client::new()
            .post(format!("{}/responses", gateway.status().endpoint.unwrap()))
            .header(
                header::AUTHORIZATION,
                "Bearer local-token-with-enough-entropy-123456",
            )
            .body("{\"stream\":true}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            timeout(Duration::from_secs(1), response.text())
                .await
                .unwrap()
                .is_err()
        );
        let status = gateway.status();
        assert_eq!(status.in_flight, 0);
        assert!(status.failed >= 1);
        gateway.stop().await.unwrap();
        upstream.abort();
    }
}
