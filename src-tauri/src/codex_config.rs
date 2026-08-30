//! Transactional, field-scoped Codex configuration profiles.
//!
//! This module deliberately owns only `model`, `model_provider`, and the
//! selected `model_providers.<id>` table. It never accepts raw TOML from the
//! WebView and never stores bearer/API-key material. Credentials are resolved
//! at Codex runtime through a fixed native `auth.command` helper and an opaque
//! Keychain reference.

use crate::safe_fs::{self, FileStamp, WriteExpectation, WriteMode};
use chrono::Utc;
use codex_storage::{CodexConfigProfileRow, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use toml::{Table, Value};
use url::Url;
use uuid::Uuid;

const MAX_CONFIG_BYTES: u64 = 512 * 1024;
const PROFILE_ID_MAX: usize = 96;
const PROFILE_TEXT_MAX: usize = 256;
const CREDENTIAL_HELPER_ARGUMENT: &str = "--codex-manager-credential";
const JOURNAL_DIRECTORY: &str = "codex-config";
const JOURNAL_FILE: &str = "active-journal.json";
const LOCK_FILE: &str = "mutation.lock";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigProfileKind {
    OfficialDirect,
    LocalCliproxy,
    ExternalCompatible,
}

impl ConfigProfileKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "official-direct" => Ok(Self::OfficialDirect),
            "local-cliproxy" => Ok(Self::LocalCliproxy),
            "external-compatible" => Ok(Self::ExternalCompatible),
            _ => Err("未知 Codex 配置档案类型。".into()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::OfficialDirect => "official-direct",
            Self::LocalCliproxy => "local-cliproxy",
            Self::ExternalCompatible => "external-compatible",
        }
    }
}

/// A WebView-safe profile DTO. `secret_ref` is an opaque Keychain lookup key,
/// never a token. It is supplied to the helper as one argument, not interpolated
/// into a shell command.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigProfile {
    pub id: String,
    pub name: String,
    pub kind: ConfigProfileKind,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub secret_ref: Option<String>,
    pub is_active: bool,
    pub is_verified: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSnapshot {
    pub profiles: Vec<ConfigProfile>,
    pub active_profile_id: Option<String>,
    pub active_revision: Option<String>,
    pub state: String,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    pub profile_id: String,
    pub expected_revision: Option<String>,
    pub config_snippet: String,
    pub managed_fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigApplyResult {
    pub changed: bool,
    pub state: String,
    pub active_profile_id: Option<String>,
    pub active_revision: Option<String>,
    pub verified: bool,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum JournalState {
    Prepared,
    Active,
}

/// Stored only in the app-private `data_dir/codex-config` directory (0700
/// directory, 0600 file). It can contain a pre-existing user config backup,
/// so it must never be moved into SQLite, WebView DTOs, logs, or diagnostics.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigJournal {
    version: u8,
    state: JournalState,
    config_path: PathBuf,
    before_exists: bool,
    before_bytes: Vec<u8>,
    before_sha256: Option<String>,
    after_sha256: String,
    active_profile_id: String,
    previous_after_sha256: Option<String>,
    previous_profile_id: Option<String>,
    created_at: String,
}

struct ConfigFile {
    root: PathBuf,
    bytes: Vec<u8>,
    stamp: Option<FileStamp>,
}

struct ConfigMutationLock {
    file: File,
}

impl Drop for ConfigMutationLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: this process owns the advisory lock descriptor.
        unsafe {
            use std::os::fd::AsRawFd;
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

/// Returns all profile metadata and the current managed-state diagnosis. This
/// does not read or expose credential values.
pub fn snapshot(
    store: &Store,
    codex_home: &Path,
    data_dir: &Path,
) -> Result<ConfigSnapshot, String> {
    let profiles = store
        .codex_config_profiles()
        .map_err(display_error)?
        .into_iter()
        .map(profile_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let active_profile_id = profiles
        .iter()
        .find(|profile| profile.is_active)
        .map(|profile| profile.id.clone());
    let config = read_config(codex_home)?;
    let revision = config.stamp.as_ref().map(|stamp| stamp.sha256.clone());
    let journal = read_journal(data_dir)?;
    let (state, message, active_revision) = match journal {
        Some(journal) if journal.state == JournalState::Active => {
            if revision.as_deref() == Some(journal.after_sha256.as_str()) {
                ("active".into(), None, revision)
            } else {
                (
                    "drifted".into(),
                    Some("Codex 配置已在应用外被修改；请先恢复或重新加载。".into()),
                    revision,
                )
            }
        }
        Some(_) => (
            "recovery-required".into(),
            Some("检测到未完成的 Codex 配置事务；请执行恢复。".into()),
            revision,
        ),
        None if active_profile_id.is_some() => ("official-direct".into(), None, revision),
        None => ("unmanaged".into(), None, revision),
    };
    Ok(ConfigSnapshot {
        profiles,
        active_profile_id,
        active_revision,
        state,
        message,
    })
}

pub fn get_profile(store: &Store, id: &str) -> Result<Option<ConfigProfile>, String> {
    store
        .codex_config_profile(id)
        .map_err(display_error)?
        .map(profile_from_row)
        .transpose()
}

/// Creates or updates routing metadata. `isActive`/`isVerified` are output
/// fields and are ignored on input; only `apply_profile` commits them.
pub fn save_profile(store: &Store, profile: ConfigProfile) -> Result<ConfigProfile, String> {
    validate_profile(&profile)?;
    let existing = store
        .codex_config_profile(&profile.id)
        .map_err(display_error)?;
    let now = Utc::now().to_rfc3339();
    let row = CodexConfigProfileRow {
        id: profile.id,
        name: profile.name,
        kind: profile.kind.as_str().into(),
        model: profile.model.unwrap_or_default(),
        reasoning_effort: profile.reasoning_effort,
        base_url: normalized_base_url(profile.kind, profile.base_url.as_deref())?,
        secret_ref: profile.secret_ref,
        active: existing.as_ref().is_some_and(|value| value.active),
        verified_at: existing
            .as_ref()
            .and_then(|value| value.verified_at.clone()),
        created_at: existing
            .as_ref()
            .map(|value| value.created_at.clone())
            .unwrap_or_else(|| {
                if profile.created_at.is_empty() {
                    now.clone()
                } else {
                    profile.created_at
                }
            }),
        updated_at: now,
    };
    store
        .save_codex_config_profile(&row)
        .map_err(display_error)?;
    profile_from_row(
        store
            .codex_config_profile(&row.id)
            .map_err(display_error)?
            .ok_or_else(|| "保存后的 Codex 配置档案不存在。".to_string())?,
    )
}

pub fn delete_profile(store: &Store, id: &str) -> Result<bool, String> {
    let current = store.codex_config_profile(id).map_err(display_error)?;
    if current.as_ref().is_some_and(|profile| profile.active) {
        return Err("当前正在使用该配置档案；请先恢复官方直连或切换其他档案。".into());
    }
    store.delete_codex_config_profile(id).map_err(display_error)
}

/// Generates a non-sensitive TOML snippet and current CAS revision. The
/// supplied profile id is resolved server-side; raw TOML is never accepted.
pub fn preview_profile(
    store: &Store,
    codex_home: &Path,
    profile_id: &str,
) -> Result<ConfigPreview, String> {
    let profile = get_required_profile(store, profile_id)?;
    let config = read_config(codex_home)?;
    let (config_snippet, managed_fields) = match profile.kind {
        ConfigProfileKind::OfficialDirect => (
            "# 恢复到此应用接管前的 Codex 配置；不会写入任何 bearer 或 API Key。\n".into(),
            vec![
                "model".into(),
                "model_provider".into(),
                "model_providers.<managed>".into(),
            ],
        ),
        ConfigProfileKind::LocalCliproxy | ConfigProfileKind::ExternalCompatible => {
            let document = managed_document(&profile)?;
            (
                toml::to_string(&document).map_err(display_error)?,
                managed_fields(&profile),
            )
        }
    };
    Ok(ConfigPreview {
        profile_id: profile.id,
        expected_revision: config.stamp.map(|stamp| stamp.sha256),
        config_snippet,
        managed_fields,
    })
}

/// Applies a resolved profile with process locking, a private durable journal,
/// descriptor-based CAS write, and readback verification. `expected_revision`
/// comes from `preview_profile` and is optional for native/automation callers;
/// filesystem CAS is always enforced even when it is absent.
pub fn apply_profile(
    store: &Store,
    codex_home: &Path,
    data_dir: &Path,
    profile_id: &str,
    expected_revision: Option<&str>,
) -> Result<ConfigApplyResult, String> {
    let profile = get_required_profile(store, profile_id)?;
    if profile.kind == ConfigProfileKind::OfficialDirect {
        let mut result = restore_config(store, codex_home, data_dir, expected_revision)?;
        let verified_at = Utc::now().to_rfc3339();
        store
            .mark_codex_config_profile_active(&profile.id, &verified_at, &verified_at)
            .map_err(display_error)?;
        result.active_profile_id = Some(profile.id);
        result.verified = true;
        result.state = "official-direct".into();
        result.message = Some("已恢复到应用接管前的 Codex 配置。".into());
        return Ok(result);
    }

    let _lock = acquire_lock(data_dir)?;
    let current = read_config(codex_home)?;
    assert_expected_revision(&current, expected_revision)?;
    let target = merge_managed_fields(&current.bytes, &profile)?;
    let target_sha = sha256(&target);
    if current
        .stamp
        .as_ref()
        .is_some_and(|stamp| stamp.sha256 == target_sha)
    {
        let verified_at = Utc::now().to_rfc3339();
        store
            .mark_codex_config_profile_active(&profile.id, &verified_at, &verified_at)
            .map_err(display_error)?;
        return Ok(ConfigApplyResult {
            changed: false,
            state: "active".into(),
            active_profile_id: Some(profile.id),
            active_revision: Some(target_sha),
            verified: true,
            message: Some("配置已是目标状态。".into()),
        });
    }

    let previous_journal = read_journal(data_dir)?;
    let journal = prepare_journal(
        &current,
        &target_sha,
        &profile.id,
        previous_journal.as_ref(),
    )?;
    write_journal(data_dir, &journal)?;
    let written = match write_config(&current, &target) {
        Ok(stamp) => stamp,
        Err(error) => {
            restore_previous_journal(data_dir, previous_journal.as_ref())?;
            return Err(error);
        }
    };
    if written.sha256 != target_sha {
        let _ = conditional_rollback(&current, &written.sha256);
        restore_previous_journal(data_dir, previous_journal.as_ref())?;
        return Err("Codex 配置写入后校验失败，已尝试回滚。".into());
    }

    let mut active_journal = journal;
    active_journal.state = JournalState::Active;
    if let Err(error) = write_journal(data_dir, &active_journal) {
        let rollback = conditional_rollback(&current, &target_sha);
        if rollback.is_ok() {
            let _ = restore_previous_journal(data_dir, previous_journal.as_ref());
        }
        return Err(format!("无法确认 Codex 配置事务：{error}"));
    }
    let verified_at = Utc::now().to_rfc3339();
    if let Err(error) =
        store.mark_codex_config_profile_active(&profile.id, &verified_at, &verified_at)
    {
        let rollback = conditional_rollback(&current, &target_sha);
        if rollback.is_ok() {
            let _ = restore_previous_journal(data_dir, previous_journal.as_ref());
        }
        return Err(format!("无法提交 Codex 配置状态：{}", display_error(error)));
    }
    Ok(ConfigApplyResult {
        changed: true,
        state: "active".into(),
        active_profile_id: Some(profile.id),
        active_revision: Some(target_sha),
        verified: true,
        message: Some("已写入并复核 Codex 配置。".into()),
    })
}

/// Restores the exact pre-takeover configuration if and only if the current
/// file is still the journalled version. A drifted file is never overwritten.
pub fn restore_config(
    store: &Store,
    codex_home: &Path,
    data_dir: &Path,
    expected_revision: Option<&str>,
) -> Result<ConfigApplyResult, String> {
    let _lock = acquire_lock(data_dir)?;
    let Some(journal) = read_journal(data_dir)? else {
        return Ok(ConfigApplyResult {
            changed: false,
            state: "unmanaged".into(),
            active_profile_id: None,
            active_revision: read_config(codex_home)?.stamp.map(|stamp| stamp.sha256),
            verified: false,
            message: Some("没有可恢复的 Codex 配置事务。".into()),
        });
    };
    let current = read_config(codex_home)?;
    assert_expected_revision(&current, expected_revision)?;
    let current_sha = current.stamp.as_ref().map(|stamp| stamp.sha256.as_str());
    if current_sha != Some(journal.after_sha256.as_str()) {
        return Err("Codex 配置已在应用外被修改；为避免覆盖，恢复已停止。".into());
    }
    let revision = if journal.before_exists {
        let stamp = write_config(&current, &journal.before_bytes)?;
        if Some(stamp.sha256.as_str()) != journal.before_sha256.as_deref() {
            return Err("恢复后的 Codex 配置校验失败。".into());
        }
        Some(stamp.sha256)
    } else {
        remove_created_config(&current)?;
        None
    };
    remove_journal(data_dir)?;
    let now = Utc::now().to_rfc3339();
    store
        .clear_active_codex_config_profile(&now)
        .map_err(display_error)?;
    Ok(ConfigApplyResult {
        changed: true,
        state: "unmanaged".into(),
        active_profile_id: None,
        active_revision: revision,
        verified: true,
        message: Some("已恢复应用接管前的 Codex 配置。".into()),
    })
}

/// Completes or rolls back a journal left by an interrupted apply. This is
/// intended for app startup, before any new apply/restore action is accepted.
pub fn recover_pending_transaction(
    store: &Store,
    codex_home: &Path,
    data_dir: &Path,
) -> Result<ConfigApplyResult, String> {
    let _lock = acquire_lock(data_dir)?;
    let Some(mut journal) = read_journal(data_dir)? else {
        return Ok(ConfigApplyResult {
            changed: false,
            state: "unmanaged".into(),
            active_profile_id: None,
            active_revision: read_config(codex_home)?.stamp.map(|stamp| stamp.sha256),
            verified: false,
            message: None,
        });
    };
    let current = read_config(codex_home)?;
    let current_sha = current.stamp.as_ref().map(|stamp| stamp.sha256.clone());
    match journal.state {
        JournalState::Active => {
            if current_sha.as_deref() != Some(journal.after_sha256.as_str()) {
                return Err("Codex 配置事务处于漂移状态，不能自动恢复。".into());
            }
        }
        JournalState::Prepared if current_sha.as_deref() == Some(journal.after_sha256.as_str()) => {
            journal.state = JournalState::Active;
            write_journal(data_dir, &journal)?;
        }
        JournalState::Prepared
            if journal
                .previous_after_sha256
                .as_deref()
                .is_some_and(|sha| current_sha.as_deref() == Some(sha)) =>
        {
            journal.state = JournalState::Active;
            journal.after_sha256 = journal
                .previous_after_sha256
                .clone()
                .expect("guarded above");
            journal.active_profile_id = journal.previous_profile_id.clone().unwrap_or_default();
            journal.previous_after_sha256 = None;
            journal.previous_profile_id = None;
            write_journal(data_dir, &journal)?;
        }
        JournalState::Prepared
            if !journal.before_exists && current.stamp.is_none()
                || journal.before_exists
                    && current_sha.as_deref() == journal.before_sha256.as_deref() =>
        {
            remove_journal(data_dir)?;
            return Ok(ConfigApplyResult {
                changed: false,
                state: "unmanaged".into(),
                active_profile_id: None,
                active_revision: current_sha,
                verified: false,
                message: Some("未完成的配置事务未写入目标文件，已安全清理。".into()),
            });
        }
        JournalState::Prepared => {
            return Err("Codex 配置事务状态与当前文件不匹配，需人工恢复。".into());
        }
    }
    let now = Utc::now().to_rfc3339();
    store
        .mark_codex_config_profile_active(&journal.active_profile_id, &now, &now)
        .map_err(display_error)?;
    Ok(ConfigApplyResult {
        changed: false,
        state: "recovered".into(),
        active_profile_id: Some(journal.active_profile_id),
        active_revision: current_sha,
        verified: true,
        message: Some("已恢复上次中断的 Codex 配置事务。".into()),
    })
}

fn get_required_profile(store: &Store, id: &str) -> Result<ConfigProfile, String> {
    get_profile(store, id)?.ok_or_else(|| "Codex 配置档案不存在。".into())
}

fn profile_from_row(row: CodexConfigProfileRow) -> Result<ConfigProfile, String> {
    Ok(ConfigProfile {
        id: row.id,
        name: row.name,
        kind: ConfigProfileKind::parse(&row.kind)?,
        base_url: row.base_url,
        model: (!row.model.is_empty()).then_some(row.model),
        reasoning_effort: row.reasoning_effort,
        secret_ref: row.secret_ref,
        is_active: row.active,
        is_verified: row.verified_at.is_some(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn validate_profile(profile: &ConfigProfile) -> Result<(), String> {
    if !valid_id(&profile.id) {
        return Err("配置档案 ID 只能包含字母、数字、点、下划线和连字符，长度不超过 96。".into());
    }
    validate_text("配置档案名称", &profile.name, 128)?;
    if let Some(model) = &profile.model {
        validate_text("模型", model, PROFILE_TEXT_MAX)?;
    }
    if let Some(effort) = &profile.reasoning_effort {
        validate_text("推理强度", effort, 64)?;
    }
    if let Some(reference) = &profile.secret_ref {
        if reference.is_empty()
            || reference.len() > PROFILE_TEXT_MAX
            || !reference.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':')
            })
        {
            return Err("凭据引用格式无效。".into());
        }
    }
    match profile.kind {
        ConfigProfileKind::OfficialDirect => {
            if profile.base_url.is_some() || profile.secret_ref.is_some() {
                return Err("官方直连档案不能包含代理地址或凭据引用。".into());
            }
        }
        ConfigProfileKind::LocalCliproxy | ConfigProfileKind::ExternalCompatible => {
            if profile.model.as_deref().is_none_or(str::is_empty) {
                return Err("代理档案必须指定模型。".into());
            }
            if profile.secret_ref.as_deref().is_none_or(str::is_empty) {
                return Err("代理档案必须引用 Keychain 中的本地凭据。".into());
            }
            normalized_base_url(profile.kind, profile.base_url.as_deref())?;
        }
    }
    match profile.kind {
        ConfigProfileKind::OfficialDirect => {}
        ConfigProfileKind::LocalCliproxy => {
            if profile.secret_ref.as_deref() != Some("local-proxy-client") {
                return Err("本地 CLIProxyAPI 档案只能引用安装级 loopback 凭据。".into());
            }
        }
        ConfigProfileKind::ExternalCompatible => {
            let Some(provider_id) = profile
                .secret_ref
                .as_deref()
                .and_then(|value| value.strip_prefix("external-provider:"))
            else {
                return Err("外部兼容档案必须引用已保存的供应商凭据。".into());
            };
            Uuid::parse_str(provider_id).map_err(|_| "外部供应商凭据引用无效。".to_string())?;
        }
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= PROFILE_ID_MAX
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_text(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max || value.contains(['\n', '\r', '\0']) {
        return Err(format!("{label}不能为空、不能换行，且长度不超过 {max}。"));
    }
    Ok(())
}

fn normalized_base_url(
    kind: ConfigProfileKind,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    if kind == ConfigProfileKind::OfficialDirect {
        return Ok(None);
    }
    let raw = value.ok_or_else(|| "代理档案必须提供 Base URL。".to_string())?;
    let parsed = Url::parse(raw).map_err(|_| "Base URL 无效。".to_string())?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Base URL 不能包含凭据、查询参数或片段。".into());
    }
    match kind {
        ConfigProfileKind::LocalCliproxy => {
            if parsed.scheme() != "http"
                || parsed.host_str() != Some("127.0.0.1")
                || !matches!(parsed.port(), Some(1024..=65535))
                || !matches!(parsed.path(), "/v1" | "/v1/")
            {
                return Err("本地 CLIProxyAPI 必须是 http://127.0.0.1:<1024-65535>/v1。".into());
            }
        }
        ConfigProfileKind::ExternalCompatible => {
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || parsed.port_or_known_default().is_none()
                || !matches!(parsed.path(), "/v1" | "/v1/")
            {
                return Err("外部兼容代理必须是 HTTP(S) /v1 地址。".into());
            }
        }
        ConfigProfileKind::OfficialDirect => unreachable!(),
    }
    Ok(Some(raw.trim_end_matches('/').to_string()))
}

fn managed_provider_id(profile: &ConfigProfile) -> String {
    format!("codex-manager-{}", profile.id)
}

fn managed_fields(profile: &ConfigProfile) -> Vec<String> {
    vec![
        "model".into(),
        "model_provider".into(),
        format!("model_providers.{}", managed_provider_id(profile)),
    ]
}

fn managed_document(profile: &ConfigProfile) -> Result<Value, String> {
    let model = profile
        .model
        .as_ref()
        .ok_or_else(|| "代理档案缺少模型。".to_string())?;
    let base_url = profile
        .base_url
        .as_ref()
        .ok_or_else(|| "代理档案缺少 Base URL。".to_string())?;
    let secret_ref = profile
        .secret_ref
        .as_ref()
        .ok_or_else(|| "代理档案缺少凭据引用。".to_string())?;
    let provider_id = managed_provider_id(profile);
    let helper =
        std::env::current_exe().map_err(|_| "无法解析 Codex Manager 凭据助手路径。".to_string())?;
    let helper = helper
        .to_str()
        .ok_or_else(|| "Codex Manager 凭据助手路径不是 UTF-8。".to_string())?;
    let mut auth = Table::new();
    auth.insert("command".into(), Value::String(helper.into()));
    auth.insert(
        "args".into(),
        Value::Array(vec![
            Value::String(CREDENTIAL_HELPER_ARGUMENT.into()),
            Value::String(secret_ref.clone()),
        ]),
    );
    let mut provider = Table::new();
    provider.insert("name".into(), Value::String(profile.name.clone()));
    provider.insert("base_url".into(), Value::String(base_url.clone()));
    provider.insert("wire_api".into(), Value::String("responses".into()));
    provider.insert("auth".into(), Value::Table(auth));
    let mut providers = Table::new();
    providers.insert(provider_id.clone(), Value::Table(provider));
    let mut root = Table::new();
    root.insert("model".into(), Value::String(model.clone()));
    root.insert("model_provider".into(), Value::String(provider_id));
    root.insert("model_providers".into(), Value::Table(providers));
    Ok(Value::Table(root))
}

fn merge_managed_fields(current: &[u8], profile: &ConfigProfile) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(current)
        .map_err(|_| "config.toml 不是 UTF-8，已拒绝修改。".to_string())?;
    let mut document = if text.trim().is_empty() {
        Value::Table(Table::new())
    } else {
        toml::from_str::<Value>(text).map_err(|error| format!("无法解析 config.toml：{error}"))?
    };
    let target = managed_document(profile)?;
    let root = document
        .as_table_mut()
        .ok_or_else(|| "config.toml 顶层必须是 TOML table。".to_string())?;
    let target_root = target.as_table().expect("managed document is a table");
    root.insert(
        "model".into(),
        target_root.get("model").expect("managed model").clone(),
    );
    root.insert(
        "model_provider".into(),
        target_root
            .get("model_provider")
            .expect("managed provider")
            .clone(),
    );
    let providers = root
        .entry("model_providers")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| "config.toml 的 model_providers 必须是 TOML table。".to_string())?;
    let managed = target_root
        .get("model_providers")
        .and_then(Value::as_table)
        .expect("managed provider table");
    let provider_id = managed_provider_id(profile);
    providers.insert(
        provider_id.clone(),
        managed
            .get(&provider_id)
            .expect("managed provider node")
            .clone(),
    );
    toml::to_string(&document)
        .map(|value| value.into_bytes())
        .map_err(display_error)
}

fn prepare_journal(
    current: &ConfigFile,
    after_sha256: &str,
    profile_id: &str,
    previous: Option<&ConfigJournal>,
) -> Result<ConfigJournal, String> {
    let (before_exists, before_bytes, before_sha256, previous_after_sha256, previous_profile_id) =
        if let Some(previous) = previous {
            if previous.state != JournalState::Active {
                return Err("检测到未完成的 Codex 配置事务；请先恢复。".into());
            }
            let current_sha = current.stamp.as_ref().map(|stamp| stamp.sha256.as_str());
            if current_sha != Some(previous.after_sha256.as_str()) {
                return Err("Codex 配置已发生漂移；请先恢复或重新加载。".into());
            }
            (
                previous.before_exists,
                previous.before_bytes.clone(),
                previous.before_sha256.clone(),
                Some(previous.after_sha256.clone()),
                Some(previous.active_profile_id.clone()),
            )
        } else {
            (
                current.stamp.is_some(),
                current.bytes.clone(),
                current.stamp.as_ref().map(|stamp| stamp.sha256.clone()),
                None,
                None,
            )
        };
    Ok(ConfigJournal {
        version: 1,
        state: JournalState::Prepared,
        config_path: current.root.join("config.toml"),
        before_exists,
        before_bytes,
        before_sha256,
        after_sha256: after_sha256.into(),
        active_profile_id: profile_id.into(),
        previous_after_sha256,
        previous_profile_id,
        created_at: Utc::now().to_rfc3339(),
    })
}

fn read_config(codex_home: &Path) -> Result<ConfigFile, String> {
    let root = safe_fs::canonical_authorized_root(codex_home)?;
    let path = root.join("config.toml");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile {
            root,
            bytes: Vec::new(),
            stamp: None,
        }),
        Err(error) => Err(format!("无法检查 config.toml：{error}")),
        Ok(_) => {
            let read =
                safe_fs::read_authorized(&root, Path::new(""), "config.toml", MAX_CONFIG_BYTES)?;
            Ok(ConfigFile {
                root,
                bytes: read.bytes,
                stamp: Some(read.stamp),
            })
        }
    }
}

fn assert_expected_revision(current: &ConfigFile, expected: Option<&str>) -> Result<(), String> {
    if let Some(expected) = expected {
        if current.stamp.as_ref().map(|stamp| stamp.sha256.as_str()) != Some(expected) {
            return Err("Codex 配置已变更；请先重新预览。".into());
        }
    }
    Ok(())
}

fn write_config(current: &ConfigFile, bytes: &[u8]) -> Result<FileStamp, String> {
    match &current.stamp {
        Some(stamp) => {
            let expectation = WriteExpectation {
                sha256: &stamp.sha256,
                mtime_ms: stamp.mtime_ms,
            };
            let (written, ()) = safe_fs::write_authorized_with_create_mode(
                &current.root,
                Path::new(""),
                "config.toml",
                WriteMode::Replace,
                Some(expectation),
                bytes,
                MAX_CONFIG_BYTES,
                0o600,
                |_, _| Ok(()),
            )?;
            Ok(written.stamp)
        }
        None => {
            let (written, ()) = safe_fs::write_authorized_with_create_mode(
                &current.root,
                Path::new(""),
                "config.toml",
                WriteMode::Create,
                None,
                bytes,
                MAX_CONFIG_BYTES,
                0o600,
                |_, _| Ok(()),
            )?;
            Ok(written.stamp)
        }
    }
}

fn conditional_rollback(current: &ConfigFile, expected_after_sha: &str) -> Result<(), String> {
    let observed = read_config(&current.root)?;
    if observed.stamp.as_ref().map(|stamp| stamp.sha256.as_str()) != Some(expected_after_sha) {
        return Err("Codex 配置在回滚前已发生外部修改；未覆盖。".into());
    }
    if current.stamp.is_some() {
        let restored = write_config(&observed, &current.bytes)?;
        if current.stamp.as_ref().map(|stamp| stamp.sha256.as_str())
            != Some(restored.sha256.as_str())
        {
            return Err("条件回滚后的配置校验失败。".into());
        }
    } else {
        remove_created_config(&observed)?;
    }
    Ok(())
}

fn remove_created_config(current: &ConfigFile) -> Result<(), String> {
    let Some(expected) = &current.stamp else {
        return Ok(());
    };
    let verified = read_config(&current.root)?;
    if verified.stamp.as_ref().map(|stamp| stamp.sha256.as_str()) != Some(expected.sha256.as_str())
    {
        return Err("Codex 配置在删除前已发生外部修改；未覆盖。".into());
    }
    // `config.toml` is a fixed filename below the canonical Codex home. The
    // process lock serializes Codex Manager instances; the descriptor read
    // above rejects symlinks and detects intervening content changes.
    fs::remove_file(current.root.join("config.toml"))
        .map_err(|error| format!("无法移除本应用创建的 config.toml：{error}"))?;
    Ok(())
}

fn journal_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(JOURNAL_DIRECTORY)
}

fn journal_path(data_dir: &Path) -> PathBuf {
    journal_dir(data_dir).join(JOURNAL_FILE)
}

fn ensure_private_journal_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let root = journal_dir(data_dir);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("Codex 配置事务目录无效。".into());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(&root).map_err(|_| "无法创建 Codex 配置事务目录。".to_string())?;
        }
        Err(_) => return Err("无法检查 Codex 配置事务目录。".into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法收紧 Codex 配置事务目录权限。".to_string())?;
        let metadata =
            fs::metadata(&root).map_err(|_| "无法验证 Codex 配置事务目录。".to_string())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err("Codex 配置事务目录权限或所有者无效。".into());
        }
    }
    Ok(root)
}

fn acquire_lock(data_dir: &Path) -> Result<ConfigMutationLock, String> {
    let root = ensure_private_journal_dir(data_dir)?;
    #[cfg(unix)]
    {
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(root.join(LOCK_FILE))
            .map_err(|_| "无法打开 Codex 配置进程锁。".to_string())?;
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            return Err("另一 Codex Manager 实例正在修改 Codex 配置。".into());
        }
        Ok(ConfigMutationLock { file })
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err("Codex 配置事务当前仅支持 macOS。".into())
    }
}

fn read_journal(data_dir: &Path) -> Result<Option<ConfigJournal>, String> {
    let path = journal_path(data_dir);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法检查 Codex 配置事务：{error}")),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("Codex 配置事务文件无效。".into())
        }
        Ok(_) => {
            let bytes = fs::read(&path).map_err(|_| "无法读取 Codex 配置事务。".to_string())?;
            let journal: ConfigJournal = serde_json::from_slice(&bytes)
                .map_err(|_| "Codex 配置事务损坏，已拒绝自动覆盖。".to_string())?;
            if journal.version != 1
                || journal
                    .config_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    != Some("config.toml")
            {
                return Err("Codex 配置事务版本或目标无效。".into());
            }
            Ok(Some(journal))
        }
    }
}

fn write_journal(data_dir: &Path, journal: &ConfigJournal) -> Result<(), String> {
    let root = ensure_private_journal_dir(data_dir)?;
    let bytes = serde_json::to_vec(journal).map_err(display_error)?;
    atomic_private_write(&root.join(JOURNAL_FILE), &bytes)
}

fn restore_previous_journal(
    data_dir: &Path,
    previous: Option<&ConfigJournal>,
) -> Result<(), String> {
    match previous {
        Some(previous) => write_journal(data_dir, previous),
        None => remove_journal(data_dir),
    }
}

fn remove_journal(data_dir: &Path) -> Result<(), String> {
    let path = journal_path(data_dir);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法清理 Codex 配置事务：{error}")),
    }
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "配置事务路径无父目录。".to_string())?;
    let target = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("配置事务文件不是普通文件。".into());
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("无法检查配置事务文件：{error}")),
    };
    let temporary = parent.join(format!(".{}-{}.tmp", JOURNAL_FILE, Uuid::new_v4()));
    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        }
        options.open(&temporary)
    }
    .map_err(|_| "无法创建私有配置事务临时文件。".to_string())?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("无法写入配置事务：{error}"));
    }
    #[cfg(unix)]
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
        .map_err(|_| "无法设置配置事务权限。".to_string())?;
    // `rename` is same-directory and therefore atomic. The parent directory is
    // app-private (0700), and the target was rejected if it is a symlink.
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("无法原子提交配置事务：{error}"));
    }
    #[cfg(unix)]
    if target {
        let metadata = fs::metadata(path).map_err(|_| "无法复核配置事务权限。".to_string())?;
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("配置事务权限过宽。".into());
        }
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn profile(id: &str, kind: ConfigProfileKind) -> ConfigProfile {
        ConfigProfile {
            id: id.into(),
            name: format!("{id} profile"),
            kind,
            base_url: match kind {
                ConfigProfileKind::LocalCliproxy => Some("http://127.0.0.1:8317/v1".into()),
                ConfigProfileKind::ExternalCompatible => {
                    Some("https://proxy.example.com/v1".into())
                }
                ConfigProfileKind::OfficialDirect => None,
            },
            model: (kind != ConfigProfileKind::OfficialDirect).then_some("gpt-test".into()),
            reasoning_effort: Some("low".into()),
            secret_ref: match kind {
                ConfigProfileKind::OfficialDirect => None,
                ConfigProfileKind::LocalCliproxy => Some("local-proxy-client".into()),
                ConfigProfileKind::ExternalCompatible => {
                    Some(format!("external-provider:{}", Uuid::new_v4()))
                }
            },
            is_active: false,
            is_verified: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn external_profile_accepts_http_and_https_without_network_resolution() {
        assert_eq!(
            normalized_base_url(
                ConfigProfileKind::ExternalCompatible,
                Some("http://192.168.10.20:8317/v1")
            )
            .unwrap()
            .as_deref(),
            Some("http://192.168.10.20:8317/v1")
        );
        assert_eq!(
            normalized_base_url(
                ConfigProfileKind::ExternalCompatible,
                Some("https://offline-provider.invalid/v1")
            )
            .unwrap()
            .as_deref(),
            Some("https://offline-provider.invalid/v1")
        );
        assert!(
            normalized_base_url(
                ConfigProfileKind::ExternalCompatible,
                Some("ftp://provider.example/v1")
            )
            .is_err()
        );
    }

    fn setup() -> (tempfile::TempDir, Store, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("codex-home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        let data = directory.path().join("data");
        let store = Store::open(&data.join("store.db")).unwrap();
        (directory, store, home, data)
    }

    #[test]
    fn apply_preserves_unmanaged_toml_and_uses_command_auth() {
        let (_directory, store, home, data) = setup();
        let original = b"model = \"old\"\nmodel_reasoning_effort = \"high\"\nkeep_me = true\n\n[model_providers.openai]\nname = \"OpenAI\"\n";
        fs::write(home.join("config.toml"), original).unwrap();
        save_profile(&store, profile("local", ConfigProfileKind::LocalCliproxy)).unwrap();
        let preview = preview_profile(&store, &home, "local").unwrap();
        assert!(!preview.config_snippet.contains("experimental_bearer_token"));
        assert!(preview.config_snippet.contains("auth"));
        apply_profile(
            &store,
            &home,
            &data,
            "local",
            preview.expected_revision.as_deref(),
        )
        .unwrap();
        let document: Value =
            toml::from_str(&fs::read_to_string(home.join("config.toml")).unwrap()).unwrap();
        assert_eq!(document.get("keep_me").and_then(Value::as_bool), Some(true));
        assert_eq!(
            document
                .get("model_reasoning_effort")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            document["model_providers"]["openai"]["name"].as_str(),
            Some("OpenAI")
        );
        assert_eq!(
            document["model_providers"]["codex-manager-local"]["wire_api"].as_str(),
            Some("responses")
        );
        assert!(
            document["model_providers"]["codex-manager-local"]["auth"]["command"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(
            document["model_providers"]["codex-manager-local"]["auth"]["args"][0].as_str(),
            Some(CREDENTIAL_HELPER_ARGUMENT)
        );
    }

    #[test]
    fn apply_rejects_preview_cas_conflict() {
        let (_directory, store, home, data) = setup();
        fs::write(home.join("config.toml"), b"model = \"old\"\n").unwrap();
        save_profile(&store, profile("local", ConfigProfileKind::LocalCliproxy)).unwrap();
        let preview = preview_profile(&store, &home, "local").unwrap();
        fs::write(home.join("config.toml"), b"model = \"outside\"\n").unwrap();
        assert!(
            apply_profile(
                &store,
                &home,
                &data,
                "local",
                preview.expected_revision.as_deref()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(home.join("config.toml")).unwrap(),
            b"model = \"outside\"\n"
        );
    }

    #[test]
    fn restore_refuses_conditional_rollback_after_external_drift() {
        let (_directory, store, home, data) = setup();
        fs::write(home.join("config.toml"), b"model = \"old\"\n").unwrap();
        save_profile(&store, profile("local", ConfigProfileKind::LocalCliproxy)).unwrap();
        apply_profile(&store, &home, &data, "local", None).unwrap();
        fs::write(home.join("config.toml"), b"model = \"outside\"\n").unwrap();
        assert!(restore_config(&store, &home, &data, None).is_err());
        assert_eq!(
            fs::read(home.join("config.toml")).unwrap(),
            b"model = \"outside\"\n"
        );
    }

    #[test]
    fn official_direct_restores_exact_pre_takeover_state() {
        let (_directory, store, home, data) = setup();
        let original = b"model = \"old\"\nkeep_me = true\n";
        fs::write(home.join("config.toml"), original).unwrap();
        save_profile(&store, profile("local", ConfigProfileKind::LocalCliproxy)).unwrap();
        save_profile(
            &store,
            profile("official", ConfigProfileKind::OfficialDirect),
        )
        .unwrap();
        apply_profile(&store, &home, &data, "local", None).unwrap();
        let result = apply_profile(&store, &home, &data, "official", None).unwrap();
        assert_eq!(result.state, "official-direct");
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), original);
        assert!(get_profile(&store, "official").unwrap().unwrap().is_active);
    }

    #[test]
    fn startup_recovery_commits_prepared_file_state() {
        let (_directory, store, home, data) = setup();
        fs::write(home.join("config.toml"), b"model = \"old\"\n").unwrap();
        save_profile(&store, profile("local", ConfigProfileKind::LocalCliproxy)).unwrap();
        apply_profile(&store, &home, &data, "local", None).unwrap();
        let mut journal = read_journal(&data).unwrap().unwrap();
        journal.state = JournalState::Prepared;
        write_journal(&data, &journal).unwrap();
        store
            .clear_active_codex_config_profile("2026-08-30T00:00:00Z")
            .unwrap();
        let result = recover_pending_transaction(&store, &home, &data).unwrap();
        assert_eq!(result.state, "recovered");
        assert_eq!(result.active_profile_id.as_deref(), Some("local"));
        assert!(get_profile(&store, "local").unwrap().unwrap().is_active);
    }
}
