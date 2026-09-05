//! CLIProxyAPI OAuth credential vault and runtime projection boundary.
//!
//! This module is deliberately separate from `auth_profiles`: the latter owns
//! official Codex login profiles, while this store owns only credentials that
//! are explicitly imported for the local CLIProxyAPI process. Raw JSON and the
//! private index live in a different Keychain service. Public DTOs are an
//! allowlist and cannot expose account identity, source paths, token material,
//! revisions, or provider error details.
//!
//! CLIProxyAPI refreshes file-backed OAuth tokens in place. To support that
//! safely, enabled profiles are projected into a random private runtime
//! directory. A successful checkpoint validates provider, stable identity and
//! CAS revision before replacing vault secrets. A pending runtime directory is
//! recovery evidence and is never deleted merely because a new launch starts.

use chrono::{DateTime, Utc};
use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use thiserror::Error;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const KEYCHAIN_SERVICE: &str = "cc.codex.manager.cliproxy-auth.v1";
const INDEX_ACCOUNT: &str = "index";
const PROFILE_ACCOUNT_PREFIX: &str = "profile:";
const INDEX_VERSION: u8 = 1;
const RUNTIME_MANIFEST_VERSION: u8 = 1;
const RUNTIME_DIRECTORY_PREFIX: &str = "cliproxy-auth-runtime-";
const RUNTIME_MANIFEST_FILE: &str = "checkpoint.json";
const MAX_IMPORT_BYTES: usize = 128 * 1024;
const MAX_LABEL_BYTES: usize = 64;
const MAX_JSON_NESTING: usize = 16;
const MAX_JSON_FIELDS: usize = 256;
const MAX_TOKEN_BYTES: usize = 32 * 1024;
const MAX_IDENTITY_BYTES: usize = 4 * 1024;
const MAX_RUNTIME_PROFILES: usize = 128;

/// OAuth providers whose current CLIProxyAPI file format is understood here.
/// Plugin-defined providers are intentionally excluded from this native vault.
pub const SUPPORTED_PROVIDERS: &[&str] = &["codex", "claude", "antigravity", "kimi", "xai"];

/// Public metadata allowlist. No path, account/email, token, fingerprint,
/// revision, deletion timestamp, or raw error can be serialized through it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAuthProfile {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_checkpoint_at: Option<String>,
    pub state: String,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAuthProfileList {
    pub profiles: Vec<ProxyAuthProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub profile: ProxyAuthProfile,
    /// Stable UI value: `created`, `updated`, or `restored`.
    pub action: String,
}

#[cfg(test)]
struct RuntimeSecret {
    pub revision: u64,
}

/// Native-only handle for one materialized CLIProxyAPI `auth-dir`.
///
/// There is intentionally no `Drop` cleanup: losing this handle before a
/// checkpoint must leave recovery evidence on disk.
pub struct RuntimeMaterialization {
    auth_dir: PathBuf,
    entries: Vec<RuntimeEntry>,
}

impl RuntimeMaterialization {
    pub fn auth_dir(&self) -> &Path {
        &self.auth_dir
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn profile_count(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone)]
struct RuntimeEntry {
    profile_id: String,
    provider: String,
    expected_revision: u64,
    file_name: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProxyAuthError {
    #[cfg(any(
        not(target_os = "macos"),
        all(feature = "desktop-qa", debug_assertions)
    ))]
    #[error("CLIProxyAPI OAuth 档案当前仅支持 macOS Keychain。")]
    UnsupportedPlatform,
    #[error("无法访问 CLIProxyAPI OAuth Keychain。")]
    KeychainUnavailable,
    #[error("凭据仓库当前不可用。")]
    StateUnavailable,
    #[error("认证文件为空或超过 128 KiB 限制。")]
    InvalidSize,
    #[error("认证文件必须是无重复键的 UTF-8 JSON object。")]
    InvalidJsonObject,
    #[error("认证文件的 provider 不受支持。")]
    UnsupportedProvider,
    #[error("认证文件不符合 CLIProxyAPI OAuth 结构。")]
    UnsupportedCredential,
    #[error("认证档案标签不符合要求。")]
    InvalidLabel,
    #[error("认证档案不存在。")]
    NotFound,
    #[error("认证档案已删除。")]
    AlreadyDeleted,
    #[error("认证档案尚未删除。")]
    NotDeleted,
    #[error("认证档案已禁用。")]
    #[cfg(test)]
    Disabled,
    #[error("刷新后的凭据不属于目标认证档案。")]
    IdentityMismatch,
    #[error("认证档案在运行期已变更，已拒绝覆盖。")]
    RevisionConflict,
    #[error("认证档案索引损坏。")]
    CorruptIndex,
    #[error("运行时凭据目录不安全。")]
    UnsafeRuntime,
    #[error("发现未完成 checkpoint 的运行时凭据，需要先恢复。")]
    RecoveryRequired,
    #[error("无已启用的 CLIProxyAPI OAuth 档案。")]
    NoEnabledProfiles,
}

pub type Result<T> = std::result::Result<T, ProxyAuthError>;

trait KeychainBackend: Send + Sync {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn write(&self, account: &str, bytes: &[u8]) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

#[cfg(target_os = "macos")]
struct MacKeychain;

#[cfg(target_os = "macos")]
impl KeychainBackend for MacKeychain {
    fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        match security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(ProxyAuthError::KeychainUnavailable),
        }
    }

    fn write(&self, account: &str, bytes: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, bytes)
            .map_err(|_| ProxyAuthError::KeychainUnavailable)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(ProxyAuthError::KeychainUnavailable),
        }
    }
}

#[cfg(any(
    not(target_os = "macos"),
    all(feature = "desktop-qa", debug_assertions)
))]
struct UnsupportedKeychain;

#[cfg(any(
    not(target_os = "macos"),
    all(feature = "desktop-qa", debug_assertions)
))]
impl KeychainBackend for UnsupportedKeychain {
    fn read(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Err(ProxyAuthError::UnsupportedPlatform)
    }
    fn write(&self, _: &str, _: &[u8]) -> Result<()> {
        Err(ProxyAuthError::UnsupportedPlatform)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(ProxyAuthError::UnsupportedPlatform)
    }
}

/// Independent Keychain-backed repository for CLIProxyAPI OAuth auth files.
pub struct ProxyAuthStore {
    backend: Arc<dyn KeychainBackend>,
    mutation: Mutex<()>,
}

impl ProxyAuthStore {
    #[cfg(all(feature = "desktop-qa", debug_assertions))]
    pub fn disabled_for_desktop_qa() -> Self {
        Self {
            backend: Arc::new(UnsupportedKeychain),
            mutation: Mutex::new(()),
        }
    }

    pub fn load() -> Self {
        #[cfg(target_os = "macos")]
        let backend: Arc<dyn KeychainBackend> = Arc::new(MacKeychain);
        #[cfg(not(target_os = "macos"))]
        let backend: Arc<dyn KeychainBackend> = Arc::new(UnsupportedKeychain);
        Self {
            backend,
            mutation: Mutex::new(()),
        }
    }

    pub fn list(&self, include_deleted: bool) -> Result<ProxyAuthProfileList> {
        let _guard = self.lock()?;
        let index = self.read_index()?;
        Ok(ProxyAuthProfileList {
            profiles: index
                .profiles
                .iter()
                .filter(|record| include_deleted || record.deleted_at.is_none())
                .map(|record| self.public_record(record))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    /// Imports bytes already read through a native, no-follow file picker
    /// boundary. Re-importing the same provider identity updates one profile.
    pub fn import(&self, source: &[u8], label: Option<&str>) -> Result<ImportOutcome> {
        let secret = validated_secret(source)?;
        let label = label
            .map(|value| normalize_label(Some(value)))
            .transpose()?;
        let _guard = self.lock()?;
        let mut index = self.read_index()?;
        let now = Utc::now();

        if let Some(position) = index.profiles.iter().position(|record| {
            record.provider == secret.provider
                && record.identity_fingerprint.as_str() == secret.identity_fingerprint.as_str()
        }) {
            let id = index.profiles[position].id.clone();
            let account = profile_account(&id);
            let previous = self.backend.read(&account)?;
            self.backend.write(&account, secret.bytes())?;
            let record = &mut index.profiles[position];
            let action = if record.deleted_at.take().is_some() {
                record.enabled = false;
                "restored"
            } else {
                "updated"
            };
            if let Some(label) = label {
                record.label = label;
            }
            record.updated_at = now;
            record.revision = next_revision(record.revision)?;
            if let Err(error) = self.write_index(&index) {
                self.restore_or_remove_secret(
                    &account,
                    previous.as_ref().map(|value| value.as_slice()),
                );
                return Err(error);
            }
            return Ok(ImportOutcome {
                profile: self.public_record(&index.profiles[position])?,
                action: action.into(),
            });
        }

        let record = ProfileRecord {
            id: Uuid::new_v4().to_string(),
            label: label.unwrap_or(normalize_label(None)?),
            provider: secret.provider.clone(),
            identity_fingerprint: secret.identity_fingerprint.to_string(),
            enabled: secret.source_enabled,
            revision: 1,
            created_at: now,
            updated_at: now,
            last_checkpoint_at: None,
            deleted_at: None,
        };
        let account = profile_account(&record.id);
        self.backend.write(&account, secret.bytes())?;
        index.profiles.push(record);
        if let Err(error) = self.write_index(&index) {
            self.restore_or_remove_secret(&account, None);
            return Err(error);
        }
        let profile = self.public_record(index.profiles.last().expect("record was pushed"))?;
        Ok(ImportOutcome {
            profile,
            action: "created".into(),
        })
    }

    pub fn enable(&self, profile_id: &str, enabled: bool) -> Result<ProxyAuthProfile> {
        let _guard = self.lock()?;
        let mut index = self.read_index()?;
        let position = index.position(profile_id)?;
        if index.profiles[position].deleted_at.is_some() {
            return Err(ProxyAuthError::AlreadyDeleted);
        }
        if self
            .backend
            .read(&profile_account(&index.profiles[position].id))?
            .is_none()
        {
            return Err(ProxyAuthError::NotFound);
        }
        if index.profiles[position].enabled != enabled {
            let record = &mut index.profiles[position];
            record.enabled = enabled;
            record.updated_at = Utc::now();
            record.revision = next_revision(record.revision)?;
            self.write_index(&index)?;
        }
        self.public_record(&index.profiles[position])
    }

    pub fn soft_delete(&self, profile_id: &str) -> Result<ProxyAuthProfile> {
        let _guard = self.lock()?;
        let mut index = self.read_index()?;
        let record = index.record_mut(profile_id)?;
        if record.deleted_at.is_some() {
            return Err(ProxyAuthError::AlreadyDeleted);
        }
        let now = Utc::now();
        record.enabled = false;
        record.deleted_at = Some(now);
        record.updated_at = now;
        record.revision = next_revision(record.revision)?;
        self.write_index(&index)?;
        self.public_record(index.record(profile_id)?)
    }

    pub fn restore(&self, profile_id: &str) -> Result<ProxyAuthProfile> {
        let _guard = self.lock()?;
        let mut index = self.read_index()?;
        let position = index.position(profile_id)?;
        if index.profiles[position].deleted_at.is_none() {
            return Err(ProxyAuthError::NotDeleted);
        }
        if self
            .backend
            .read(&profile_account(&index.profiles[position].id))?
            .is_none()
        {
            return Err(ProxyAuthError::NotFound);
        }
        let record = &mut index.profiles[position];
        record.deleted_at = None;
        record.enabled = false;
        record.updated_at = Utc::now();
        record.revision = next_revision(record.revision)?;
        self.write_index(&index)?;
        self.public_record(&index.profiles[position])
    }

    /// Returns one enabled credential to native runtime code with its CAS
    /// revision. The stored source is projected with `disabled: false`.
    #[cfg(test)]
    fn read_secret_for_runtime(&self, profile_id: &str) -> Result<RuntimeSecret> {
        let _guard = self.lock()?;
        let index = self.read_index()?;
        let record = index.active_record(profile_id)?;
        if !record.enabled {
            return Err(ProxyAuthError::Disabled);
        }
        self.backend
            .read(&profile_account(&record.id))?
            .ok_or(ProxyAuthError::NotFound)?;
        Ok(RuntimeSecret {
            revision: record.revision,
        })
    }

    /// Checkpoints one file refreshed by CLIProxyAPI. Provider, stable identity
    /// and the caller's expected revision must all match.
    #[cfg(test)]
    fn checkpoint_from_runtime(
        &self,
        profile_id: &str,
        expected_revision: u64,
        source: &[u8],
    ) -> Result<ProxyAuthProfile> {
        let secret = validated_secret(source)?;
        let _guard = self.lock()?;
        let mut index = self.read_index()?;
        let position = index.position(profile_id)?;
        validate_checkpoint_record(&index.profiles[position], expected_revision, &secret)?;
        let account = profile_account(profile_id);
        let previous = self.backend.read(&account)?;
        self.backend.write(&account, secret.bytes())?;
        let now = Utc::now();
        let record = &mut index.profiles[position];
        record.revision = next_revision(record.revision)?;
        record.updated_at = now;
        record.last_checkpoint_at = Some(now);
        if let Err(error) = self.write_index(&index) {
            self.restore_or_remove_secret(
                &account,
                previous.as_ref().map(|value| value.as_slice()),
            );
            return Err(error);
        }
        self.public_record(&index.profiles[position])
    }

    /// Creates a random 0700 auth directory containing one 0600 file per
    /// enabled profile. Existing pending runtime evidence blocks a new launch.
    pub fn materialize_enabled_for_runtime(
        &self,
        runtime_parent: &Path,
    ) -> Result<RuntimeMaterialization> {
        let _guard = self.lock()?;
        ensure_runtime_parent(runtime_parent)?;
        cleanup_committed_runtime_directories(runtime_parent)?;
        if has_runtime_directory(runtime_parent)? {
            return Err(ProxyAuthError::RecoveryRequired);
        }

        let index = self.read_index()?;
        let enabled = index
            .profiles
            .iter()
            .filter(|record| record.deleted_at.is_none() && record.enabled)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return Err(ProxyAuthError::NoEnabledProfiles);
        }
        if enabled.len() > MAX_RUNTIME_PROFILES {
            return Err(ProxyAuthError::UnsafeRuntime);
        }

        let auth_dir = runtime_parent.join(format!("{RUNTIME_DIRECTORY_PREFIX}{}", Uuid::new_v4()));
        create_private_directory(&auth_dir)?;
        let mut entries = Vec::with_capacity(enabled.len());
        for record in enabled {
            let source = self
                .backend
                .read(&profile_account(&record.id))?
                .ok_or(ProxyAuthError::NotFound)?;
            let projected = runtime_projection_bytes(&source)?;
            let file_name = runtime_file_name(&record.id)?;
            write_private_new_file(&auth_dir.join(&file_name), &projected)?;
            entries.push(RuntimeEntry {
                profile_id: record.id.clone(),
                provider: record.provider.clone(),
                expected_revision: record.revision,
                file_name,
            });
        }
        let manifest = RuntimeManifest::pending(&entries);
        // Files may contain refreshable credentials. On failure, preserve the
        // directory as recovery evidence rather than attempting broad cleanup.
        write_manifest_new(&auth_dir, &manifest)?;
        Ok(RuntimeMaterialization { auth_dir, entries })
    }

    /// Atomically validates and checkpoints every file in a materialization,
    /// then marks the journal committed before exact allowlisted cleanup.
    pub fn checkpoint_materialized_runtime(
        &self,
        materialization: &RuntimeMaterialization,
    ) -> Result<Vec<ProxyAuthProfile>> {
        let _guard = self.lock()?;
        self.checkpoint_materialized_locked(materialization)
    }

    /// Recovers exactly one pending auth-dir beneath `runtime_parent`. It can
    /// also finish cleanup when vault commit succeeded before the committed
    /// marker was durably written.
    pub fn recover_pending_runtime(&self, runtime_parent: &Path) -> Result<Vec<ProxyAuthProfile>> {
        let _guard = self.lock()?;
        ensure_runtime_parent(runtime_parent)?;
        let mut pending = Vec::new();
        for directory in runtime_directories(runtime_parent)? {
            let manifest = read_runtime_manifest(&directory)?;
            if manifest.state == RuntimeManifestState::Committed {
                cleanup_runtime_directory(&directory, &manifest)?;
            } else {
                pending.push((directory, manifest));
            }
        }
        if pending.len() != 1 {
            return Err(ProxyAuthError::RecoveryRequired);
        }
        let (auth_dir, manifest) = pending.pop().expect("one pending directory");
        let entries = manifest.validated_entries()?;
        self.checkpoint_materialized_locked(&RuntimeMaterialization { auth_dir, entries })
    }

    fn checkpoint_materialized_locked(
        &self,
        materialization: &RuntimeMaterialization,
    ) -> Result<Vec<ProxyAuthProfile>> {
        validate_runtime_directory(&materialization.auth_dir)?;
        let manifest = read_runtime_manifest(&materialization.auth_dir)?;
        if manifest.state != RuntimeManifestState::Pending
            || manifest.validated_entries()? != materialization.entries
        {
            return Err(ProxyAuthError::UnsafeRuntime);
        }

        let mut payloads = Vec::with_capacity(materialization.entries.len());
        for entry in &materialization.entries {
            let bytes = stable_read_private_file(
                &materialization.auth_dir.join(&entry.file_name),
                MAX_IMPORT_BYTES,
            )?;
            let secret = validated_secret(&bytes)?;
            payloads.push((entry.clone(), bytes, secret));
        }

        let mut index = self.read_index()?;
        let already_committed = payloads.iter().all(|(entry, bytes, secret)| {
            let Ok(record) = index.record(&entry.profile_id) else {
                return false;
            };
            if record.revision != entry.expected_revision.saturating_add(1)
                || record.provider != secret.provider
                || record.identity_fingerprint.as_str() != secret.identity_fingerprint.as_str()
            {
                return false;
            }
            self.backend
                .read(&profile_account(&record.id))
                .ok()
                .flatten()
                .is_some_and(|stored| stored.as_slice() == bytes.as_slice())
        });
        if already_committed {
            mark_runtime_committed(&materialization.auth_dir, &manifest)?;
            cleanup_runtime_directory(&materialization.auth_dir, &manifest)?;
            return materialization
                .entries
                .iter()
                .map(|entry| self.public_record(index.record(&entry.profile_id)?))
                .collect();
        }

        for (entry, _, secret) in &payloads {
            let record = index.record(&entry.profile_id)?;
            validate_checkpoint_record(record, entry.expected_revision, secret)?;
        }

        let mut previous = Vec::with_capacity(payloads.len());
        for (entry, bytes, _) in &payloads {
            let account = profile_account(&entry.profile_id);
            let old = self.backend.read(&account)?;
            if let Err(error) = self.backend.write(&account, bytes) {
                self.restore_batch(&previous);
                return Err(error);
            }
            previous.push((account, old));
        }

        let now = Utc::now();
        for (entry, _, _) in &payloads {
            let record = index.record_mut(&entry.profile_id)?;
            record.revision = next_revision(record.revision)?;
            record.updated_at = now;
            record.last_checkpoint_at = Some(now);
        }
        if let Err(error) = self.write_index(&index) {
            self.restore_batch(&previous);
            return Err(error);
        }

        mark_runtime_committed(&materialization.auth_dir, &manifest)?;
        cleanup_runtime_directory(&materialization.auth_dir, &manifest)?;
        materialization
            .entries
            .iter()
            .map(|entry| self.public_record(index.record(&entry.profile_id)?))
            .collect()
    }

    fn read_index(&self) -> Result<ProfileIndex> {
        let Some(bytes) = self.backend.read(INDEX_ACCOUNT)? else {
            return Ok(ProfileIndex::empty());
        };
        serde_json::from_slice::<ProfileIndex>(&bytes)
            .map_err(|_| ProxyAuthError::CorruptIndex)?
            .validate()
    }

    fn write_index(&self, index: &ProfileIndex) -> Result<()> {
        let bytes =
            Zeroizing::new(serde_json::to_vec(index).map_err(|_| ProxyAuthError::CorruptIndex)?);
        self.backend.write(INDEX_ACCOUNT, &bytes)
    }

    fn public_record(&self, record: &ProfileRecord) -> Result<ProxyAuthProfile> {
        let (state, error_code) = if record.deleted_at.is_some() {
            ("deleted", None)
        } else if self.backend.read(&profile_account(&record.id))?.is_none() {
            ("error", Some("secret-missing"))
        } else if record.enabled {
            ("ready", None)
        } else {
            ("disabled", None)
        };
        Ok(ProxyAuthProfile {
            id: record.id.clone(),
            label: record.label.clone(),
            provider: record.provider.clone(),
            enabled: record.enabled && record.deleted_at.is_none(),
            created_at: record.created_at.to_rfc3339(),
            updated_at: record.updated_at.to_rfc3339(),
            last_checkpoint_at: record.last_checkpoint_at.map(|value| value.to_rfc3339()),
            state: state.into(),
            error_code: error_code.map(str::to_owned),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.mutation
            .lock()
            .map_err(|_| ProxyAuthError::StateUnavailable)
    }

    fn restore_or_remove_secret(&self, account: &str, previous: Option<&[u8]>) {
        match previous {
            Some(bytes) => {
                let _ = self.backend.write(account, bytes);
            }
            None => {
                let _ = self.backend.delete(account);
            }
        }
    }

    fn restore_batch(&self, previous: &[(String, Option<Zeroizing<Vec<u8>>>)]) {
        for (account, secret) in previous.iter().rev() {
            self.restore_or_remove_secret(account, secret.as_ref().map(|value| value.as_slice()));
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn KeychainBackend>) -> Self {
        Self {
            backend,
            mutation: Mutex::new(()),
        }
    }
}

impl PartialEq for RuntimeEntry {
    fn eq(&self, other: &Self) -> bool {
        self.profile_id == other.profile_id
            && self.provider == other.provider
            && self.expected_revision == other.expected_revision
            && self.file_name == other.file_name
    }
}
impl Eq for RuntimeEntry {}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileIndex {
    version: u8,
    profiles: Vec<ProfileRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileRecord {
    id: String,
    label: String,
    provider: String,
    identity_fingerprint: String,
    enabled: bool,
    revision: u64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_checkpoint_at: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
}

impl ProfileIndex {
    fn empty() -> Self {
        Self {
            version: INDEX_VERSION,
            profiles: Vec::new(),
        }
    }

    fn validate(self) -> Result<Self> {
        if self.version != INDEX_VERSION || self.profiles.len() > MAX_RUNTIME_PROFILES {
            return Err(ProxyAuthError::CorruptIndex);
        }
        let mut ids = HashSet::new();
        let mut identities = HashSet::new();
        for record in &self.profiles {
            if Uuid::parse_str(&record.id).is_err()
                || normalize_label(Some(&record.label)).is_err()
                || normalize_provider(&record.provider).ok().as_deref()
                    != Some(record.provider.as_str())
                || record.identity_fingerprint.len() != 64
                || !record
                    .identity_fingerprint
                    .as_bytes()
                    .iter()
                    .all(u8::is_ascii_hexdigit)
                || record.revision == 0
                || record.updated_at < record.created_at
                || record
                    .last_checkpoint_at
                    .is_some_and(|value| value > record.updated_at)
                || (record.deleted_at.is_some() && record.enabled)
                || !ids.insert(record.id.as_str())
                || !identities.insert((
                    record.provider.as_str(),
                    record.identity_fingerprint.as_str(),
                ))
            {
                return Err(ProxyAuthError::CorruptIndex);
            }
        }
        Ok(self)
    }

    fn position(&self, id: &str) -> Result<usize> {
        self.profiles
            .iter()
            .position(|record| record.id == id)
            .ok_or(ProxyAuthError::NotFound)
    }

    fn record(&self, id: &str) -> Result<&ProfileRecord> {
        self.profiles
            .iter()
            .find(|record| record.id == id)
            .ok_or(ProxyAuthError::NotFound)
    }

    fn record_mut(&mut self, id: &str) -> Result<&mut ProfileRecord> {
        self.profiles
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(ProxyAuthError::NotFound)
    }

    #[cfg(test)]
    fn active_record(&self, id: &str) -> Result<&ProfileRecord> {
        let record = self.record(id)?;
        if record.deleted_at.is_some() {
            return Err(ProxyAuthError::AlreadyDeleted);
        }
        Ok(record)
    }
}

struct ValidatedSecret {
    bytes: Zeroizing<Vec<u8>>,
    provider: String,
    identity_fingerprint: Zeroizing<String>,
    source_enabled: bool,
}

impl ValidatedSecret {
    fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

fn validated_secret(source: &[u8]) -> Result<ValidatedSecret> {
    if source.is_empty() || source.len() > MAX_IMPORT_BYTES || std::str::from_utf8(source).is_err()
    {
        return Err(ProxyAuthError::InvalidSize);
    }
    validate_json_shape(source)?;
    let mut value: Value =
        serde_json::from_slice(source).map_err(|_| ProxyAuthError::InvalidJsonObject)?;
    let result = validate_credential_value(&value);
    zeroize_json_value(&mut value);
    let (provider, identity_fingerprint, source_enabled) = result?;
    Ok(ValidatedSecret {
        bytes: Zeroizing::new(source.to_vec()),
        provider,
        identity_fingerprint: Zeroizing::new(identity_fingerprint),
        source_enabled,
    })
}

/// Bounded, provider-aware structure validation for native import callers.
#[cfg_attr(not(test), allow(dead_code))]
pub fn validate_structure(source: &[u8]) -> Result<String> {
    validated_secret(source).map(|secret| secret.provider)
}

fn validate_credential_value(value: &Value) -> Result<(String, String, bool)> {
    let root = value.as_object().ok_or(ProxyAuthError::InvalidJsonObject)?;
    let provider_value = root
        .get("type")
        .or_else(|| root.get("provider"))
        .and_then(Value::as_str)
        .ok_or(ProxyAuthError::UnsupportedProvider)?;
    let provider = normalize_provider(provider_value)?;
    require_secret_field(root, "access_token")?;
    require_secret_field(root, "refresh_token")?;
    if let Some(value) = root.get("disabled")
        && !value.is_boolean()
    {
        return Err(ProxyAuthError::UnsupportedCredential);
    }

    let (identity_key, identity_value) = provider_identity(&provider, root)?;
    let normalized_identity = normalize_identity(identity_key, identity_value)?;
    let mut digest = Sha256::new();
    digest.update(provider.as_bytes());
    digest.update([0]);
    digest.update(identity_key.as_bytes());
    digest.update([0]);
    digest.update(normalized_identity.as_bytes());
    let fingerprint = hex::encode(digest.finalize());
    Ok((
        provider,
        fingerprint,
        !root
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ))
}

fn normalize_provider(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    let canonical = match normalized.as_str() {
        "openai" => "codex",
        "anthropic" => "claude",
        "anti-gravity" => "antigravity",
        "grok" | "x-ai" | "x.ai" => "xai",
        value => value,
    };
    if !SUPPORTED_PROVIDERS.contains(&canonical) {
        return Err(ProxyAuthError::UnsupportedProvider);
    }
    Ok(canonical.to_owned())
}

fn require_secret_field(root: &serde_json::Map<String, Value>, key: &str) -> Result<()> {
    let Some(Value::String(value)) = root.get(key) else {
        return Err(ProxyAuthError::UnsupportedCredential);
    };
    if value.trim().is_empty()
        || value.len() > MAX_TOKEN_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ProxyAuthError::UnsupportedCredential);
    }
    Ok(())
}

fn provider_identity<'a>(
    provider: &str,
    root: &'a serde_json::Map<String, Value>,
) -> Result<(&'static str, &'a str)> {
    let candidates: &[&str] = match provider {
        "codex" => &["account_id", "email"],
        "claude" => &["account_uuid", "email"],
        "antigravity" => &["email", "project_id"],
        "kimi" => &["device_id"],
        "xai" => &["sub", "email"],
        _ => return Err(ProxyAuthError::UnsupportedProvider),
    };
    candidates
        .iter()
        .find_map(|key| {
            root.get(*key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| (*key, value))
        })
        .ok_or(ProxyAuthError::UnsupportedCredential)
}

fn normalize_identity(key: &str, value: &str) -> Result<Zeroizing<String>> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_IDENTITY_BYTES || value.chars().any(char::is_control) {
        return Err(ProxyAuthError::UnsupportedCredential);
    }
    let normalized = if key == "email" {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    };
    Ok(Zeroizing::new(normalized))
}

fn runtime_projection_bytes(source: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let mut value: Value =
        serde_json::from_slice(source).map_err(|_| ProxyAuthError::InvalidJsonObject)?;
    // Normalize accepted compatibility aliases to the provider identifiers
    // that CLIProxyAPI's file synthesizer actually routes.
    let provider = match validate_credential_value(&value) {
        Ok((provider, _, _)) => provider,
        Err(error) => {
            zeroize_json_value(&mut value);
            return Err(error);
        }
    };
    let object = value
        .as_object_mut()
        .expect("provider-aware validation accepted a JSON object");
    object.insert("type".into(), Value::String(provider));
    object.insert("disabled".into(), Value::Bool(false));
    let result = serde_json::to_vec(&value).map_err(|_| ProxyAuthError::InvalidJsonObject);
    zeroize_json_value(&mut value);
    result.map(Zeroizing::new)
}

fn validate_checkpoint_record(
    record: &ProfileRecord,
    expected_revision: u64,
    secret: &ValidatedSecret,
) -> Result<()> {
    if record.deleted_at.is_some() {
        return Err(ProxyAuthError::AlreadyDeleted);
    }
    if record.revision != expected_revision {
        return Err(ProxyAuthError::RevisionConflict);
    }
    if record.provider != secret.provider
        || record.identity_fingerprint.as_str() != secret.identity_fingerprint.as_str()
    {
        return Err(ProxyAuthError::IdentityMismatch);
    }
    Ok(())
}

fn normalize_label(label: Option<&str>) -> Result<String> {
    let label = label.unwrap_or("CLIProxyAPI OAuth").trim();
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || label.contains('@')
        || label.contains('/')
        || label.contains('\\')
        || label.chars().any(char::is_control)
    {
        return Err(ProxyAuthError::InvalidLabel);
    }
    Ok(label.to_owned())
}

fn next_revision(revision: u64) -> Result<u64> {
    revision.checked_add(1).ok_or(ProxyAuthError::CorruptIndex)
}

fn profile_account(id: &str) -> String {
    format!("{PROFILE_ACCOUNT_PREFIX}{id}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RuntimeManifestState {
    Pending,
    Committed,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifest {
    version: u8,
    state: RuntimeManifestState,
    entries: Vec<RuntimeManifestEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeManifestEntry {
    profile_id: String,
    provider: String,
    expected_revision: u64,
    file_name: String,
}

impl RuntimeManifest {
    fn pending(entries: &[RuntimeEntry]) -> Self {
        Self {
            version: RUNTIME_MANIFEST_VERSION,
            state: RuntimeManifestState::Pending,
            entries: entries
                .iter()
                .map(|entry| RuntimeManifestEntry {
                    profile_id: entry.profile_id.clone(),
                    provider: entry.provider.clone(),
                    expected_revision: entry.expected_revision,
                    file_name: entry.file_name.clone(),
                })
                .collect(),
        }
    }

    fn validated_entries(&self) -> Result<Vec<RuntimeEntry>> {
        if self.version != RUNTIME_MANIFEST_VERSION
            || self.entries.is_empty()
            || self.entries.len() > MAX_RUNTIME_PROFILES
        {
            return Err(ProxyAuthError::UnsafeRuntime);
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        self.entries
            .iter()
            .map(|entry| {
                if Uuid::parse_str(&entry.profile_id).is_err()
                    || normalize_provider(&entry.provider).ok().as_deref()
                        != Some(entry.provider.as_str())
                    || entry.expected_revision == 0
                    || runtime_file_name(&entry.profile_id)? != entry.file_name
                    || !ids.insert(entry.profile_id.as_str())
                    || !names.insert(entry.file_name.as_str())
                {
                    return Err(ProxyAuthError::UnsafeRuntime);
                }
                Ok(RuntimeEntry {
                    profile_id: entry.profile_id.clone(),
                    provider: entry.provider.clone(),
                    expected_revision: entry.expected_revision,
                    file_name: entry.file_name.clone(),
                })
            })
            .collect()
    }
}

fn runtime_file_name(profile_id: &str) -> Result<String> {
    let id = Uuid::parse_str(profile_id).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    Ok(format!("profile-{id}.json"))
}

fn ensure_runtime_parent(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_private_directory_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                    .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            }
            let metadata = fs::symlink_metadata(path).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            validate_private_directory_metadata(&metadata)
        }
        Err(_) => Err(ProxyAuthError::UnsafeRuntime),
    }
}

fn create_private_directory(path: &Path) -> Result<()> {
    if path.file_name().is_none()
        || path
            .components()
            .next_back()
            .is_none_or(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(path).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    validate_runtime_directory(path)
}

fn validate_runtime_directory(path: &Path) -> Result<()> {
    if !path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| {
            name.strip_prefix(RUNTIME_DIRECTORY_PREFIX)
                .is_some_and(|id| Uuid::parse_str(id).is_ok())
        })
    {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    validate_private_directory_metadata(&metadata)
}

fn validate_private_directory_metadata(metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ProxyAuthError::UnsafeRuntime);
        }
    }
    Ok(())
}

fn write_private_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    file.write_all(bytes)
        .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    file.sync_all().map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    validate_private_file_metadata(&file.metadata().map_err(|_| ProxyAuthError::UnsafeRuntime)?)
}

fn validate_private_file_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ProxyAuthError::UnsafeRuntime);
        }
    }
    Ok(())
}

fn stable_read_private_file(path: &Path, max_bytes: usize) -> Result<Zeroizing<Vec<u8>>> {
    let before = fs::symlink_metadata(path).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    validate_private_file_metadata(&before)?;
    if before.len() > max_bytes as u64 {
        return Err(ProxyAuthError::InvalidSize);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    let opened = file.metadata().map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    if !same_file_metadata(&before, &opened) {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(opened.len() as usize));
    Read::by_ref(&mut file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    if bytes.len() > max_bytes {
        return Err(ProxyAuthError::InvalidSize);
    }
    let after = file.metadata().map_err(|_| ProxyAuthError::UnsafeRuntime)?;
    if !same_file_metadata(&opened, &after) || after.len() != bytes.len() as u64 {
        return Err(ProxyAuthError::UnsafeRuntime);
    }
    Ok(bytes)
}

#[cfg(unix)]
fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.uid() == right.uid()
        && left.nlink() == right.nlink()
        && left.mode() == right.mode()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(not(unix))]
fn same_file_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.is_file()
        && right.is_file()
}

fn write_manifest_new(auth_dir: &Path, manifest: &RuntimeManifest) -> Result<()> {
    let bytes =
        Zeroizing::new(serde_json::to_vec(manifest).map_err(|_| ProxyAuthError::UnsafeRuntime)?);
    write_private_new_file(&auth_dir.join(RUNTIME_MANIFEST_FILE), &bytes)
}

fn read_runtime_manifest(auth_dir: &Path) -> Result<RuntimeManifest> {
    validate_runtime_directory(auth_dir)?;
    let bytes = stable_read_private_file(&auth_dir.join(RUNTIME_MANIFEST_FILE), 64 * 1024)?;
    let manifest: RuntimeManifest =
        serde_json::from_slice(&bytes).map_err(|_| ProxyAuthError::RecoveryRequired)?;
    manifest.validated_entries()?;
    Ok(manifest)
}

fn mark_runtime_committed(auth_dir: &Path, manifest: &RuntimeManifest) -> Result<()> {
    let mut committed = manifest.clone();
    committed.state = RuntimeManifestState::Committed;
    let bytes =
        Zeroizing::new(serde_json::to_vec(&committed).map_err(|_| ProxyAuthError::UnsafeRuntime)?);
    let temporary = auth_dir.join(format!("checkpoint.tmp-{}", Uuid::new_v4()));
    write_private_new_file(&temporary, &bytes)?;
    fs::rename(&temporary, auth_dir.join(RUNTIME_MANIFEST_FILE))
        .map_err(|_| ProxyAuthError::RecoveryRequired)
}

fn runtime_directories(runtime_parent: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(runtime_parent).map_err(|_| ProxyAuthError::UnsafeRuntime)? {
        let entry = entry.map_err(|_| ProxyAuthError::UnsafeRuntime)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(RUNTIME_DIRECTORY_PREFIX) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ProxyAuthError::RecoveryRequired);
        }
        validate_runtime_directory(&entry.path())?;
        directories.push(entry.path());
    }
    directories.sort();
    Ok(directories)
}

fn has_runtime_directory(runtime_parent: &Path) -> Result<bool> {
    Ok(!runtime_directories(runtime_parent)?.is_empty())
}

fn cleanup_committed_runtime_directories(runtime_parent: &Path) -> Result<()> {
    for directory in runtime_directories(runtime_parent)? {
        let manifest = match read_runtime_manifest(&directory) {
            Ok(manifest) => manifest,
            Err(_) => return Err(ProxyAuthError::RecoveryRequired),
        };
        if manifest.state != RuntimeManifestState::Committed {
            return Err(ProxyAuthError::RecoveryRequired);
        }
        cleanup_runtime_directory(&directory, &manifest)?;
    }
    Ok(())
}

fn cleanup_runtime_directory(auth_dir: &Path, manifest: &RuntimeManifest) -> Result<()> {
    validate_runtime_directory(auth_dir)?;
    let expected = manifest
        .validated_entries()?
        .into_iter()
        .map(|entry| entry.file_name)
        .chain(std::iter::once(RUNTIME_MANIFEST_FILE.to_owned()))
        .collect::<HashSet<_>>();
    let actual = fs::read_dir(auth_dir)
        .map_err(|_| ProxyAuthError::UnsafeRuntime)?
        .map(|entry| {
            let entry = entry.map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| ProxyAuthError::UnsafeRuntime)?;
            validate_private_file_metadata(&metadata)?;
            Ok(name)
        })
        .collect::<Result<HashSet<_>>>()?;
    if actual != expected {
        return Err(ProxyAuthError::RecoveryRequired);
    }
    for name in expected
        .iter()
        .filter(|name| name.as_str() != RUNTIME_MANIFEST_FILE)
    {
        fs::remove_file(auth_dir.join(name)).map_err(|_| ProxyAuthError::RecoveryRequired)?;
    }
    fs::remove_file(auth_dir.join(RUNTIME_MANIFEST_FILE))
        .map_err(|_| ProxyAuthError::RecoveryRequired)?;
    fs::remove_dir(auth_dir).map_err(|_| ProxyAuthError::RecoveryRequired)
}

fn zeroize_json_value(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json_value),
        Value::Object(values) => values.values_mut().for_each(zeroize_json_value),
        _ => {}
    }
}

struct JsonShape {
    depth: usize,
    fields: usize,
}

impl<'de> DeserializeSeed<'de> for &mut JsonShape {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonShapeVisitor { state: self })
    }
}

struct JsonShapeVisitor<'a> {
    state: &'a mut JsonShape,
}

impl<'de> Visitor<'de> for JsonShapeVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a bounded JSON value without duplicate object keys")
    }
    fn visit_bool<E>(self, _: bool) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_none<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_unit<E>(self) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_borrowed_str<E>(self, _: &'de str) -> std::result::Result<(), E> {
        Ok(())
    }
    fn visit_string<E>(self, mut value: String) -> std::result::Result<(), E> {
        value.zeroize();
        Ok(())
    }
    fn visit_some<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        if self.state.depth >= MAX_JSON_NESTING {
            return Err(serde::de::Error::custom("JSON nesting limit"));
        }
        self.state.depth += 1;
        while sequence.next_element_seed(&mut *self.state)?.is_some() {}
        self.state.depth -= 1;
        Ok(())
    }
    fn visit_map<A>(self, mut map: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        if self.state.depth >= MAX_JSON_NESTING {
            return Err(serde::de::Error::custom("JSON nesting limit"));
        }
        self.state.depth += 1;
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            self.state.fields += 1;
            if self.state.fields > MAX_JSON_FIELDS || !keys.insert(key) {
                return Err(serde::de::Error::custom(
                    "JSON object field limit or duplicate key",
                ));
            }
            map.next_value_seed(&mut *self.state)?;
        }
        for mut key in keys.drain() {
            key.zeroize();
        }
        self.state.depth -= 1;
        Ok(())
    }
}

fn validate_json_shape(source: &[u8]) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_slice(source);
    let mut shape = JsonShape {
        depth: 0,
        fields: 0,
    };
    (&mut shape)
        .deserialize(&mut deserializer)
        .and_then(|_| deserializer.end())
        .map_err(|_| ProxyAuthError::InvalidJsonObject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryKeychain {
        entries: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl KeychainBackend for MemoryKeychain {
        fn read(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .map(Zeroizing::new))
        }
        fn write(&self, account: &str, bytes: &[u8]) -> Result<()> {
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_owned(), bytes.to_vec());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.entries.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn store() -> ProxyAuthStore {
        ProxyAuthStore::with_backend(Arc::new(MemoryKeychain::default()))
    }

    fn codex_fixture(account: &str, access: &str, refresh: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "codex",
            "account_id": account,
            "email": "never-public@example.invalid",
            "access_token": access,
            "refresh_token": refresh
        }))
        .unwrap()
    }

    #[test]
    fn public_dto_serializes_only_the_allowlist() {
        let profile = ProxyAuthProfile {
            id: "random-id".into(),
            label: "个人".into(),
            provider: "codex".into(),
            enabled: true,
            created_at: "created".into(),
            updated_at: "updated".into(),
            last_checkpoint_at: None,
            state: "ready".into(),
            error_code: None,
        };
        let value = serde_json::to_value(&profile).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let expected = [
            "id",
            "label",
            "provider",
            "enabled",
            "createdAt",
            "updatedAt",
            "lastCheckpointAt",
            "state",
            "errorCode",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<HashSet<_>>();
        assert_eq!(keys, expected);
        let rendered = value.to_string();
        for forbidden in [
            "token",
            "email",
            "path",
            "revision",
            "fingerprint",
            "account_id",
            "deletedAt",
        ] {
            assert!(!rendered.contains(forbidden));
        }
    }

    #[test]
    fn duplicate_import_updates_one_identity_without_exposing_it() {
        let store = store();
        let first = store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), Some("A"))
            .unwrap();
        let updated = store
            .import(&codex_fixture("acct-a", "access-b", "refresh-b"), Some("B"))
            .unwrap();
        assert_eq!(first.profile.id, updated.profile.id);
        assert_eq!(updated.action, "updated");
        assert_eq!(store.list(false).unwrap().profiles.len(), 1);
        assert_eq!(
            store
                .read_secret_for_runtime(&first.profile.id)
                .unwrap()
                .revision,
            2
        );
    }

    #[test]
    fn import_without_a_webview_label_uses_a_safe_native_default() {
        let store = store();
        let outcome = store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), None)
            .unwrap();
        assert_eq!(outcome.profile.label, "CLIProxyAPI OAuth");
    }

    #[test]
    fn reimport_without_a_webview_label_preserves_the_existing_label() {
        let store = store();
        store
            .import(
                &codex_fixture("acct-a", "access-a", "refresh-a"),
                Some("个人订阅"),
            )
            .unwrap();
        let outcome = store
            .import(&codex_fixture("acct-a", "access-b", "refresh-b"), None)
            .unwrap();
        assert_eq!(outcome.action, "updated");
        assert_eq!(outcome.profile.label, "个人订阅");
    }

    #[test]
    fn checkpoint_rejects_identity_or_provider_mixup() {
        let store = store();
        let first = store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), Some("A"))
            .unwrap();
        let runtime = store.read_secret_for_runtime(&first.profile.id).unwrap();
        assert_eq!(
            store.checkpoint_from_runtime(
                &first.profile.id,
                runtime.revision,
                &codex_fixture("acct-b", "access-b", "refresh-b")
            ),
            Err(ProxyAuthError::IdentityMismatch)
        );
        assert_eq!(
            store
                .read_secret_for_runtime(&first.profile.id)
                .unwrap()
                .revision,
            runtime.revision
        );
    }

    #[test]
    fn checkpoint_uses_compare_and_swap_revision() {
        let store = store();
        let profile = store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), Some("A"))
            .unwrap()
            .profile;
        let runtime = store.read_secret_for_runtime(&profile.id).unwrap();
        let refreshed = codex_fixture("acct-a", "access-new", "refresh-new");
        let checkpointed = store
            .checkpoint_from_runtime(&profile.id, runtime.revision, &refreshed)
            .unwrap();
        assert!(checkpointed.last_checkpoint_at.is_some());
        assert_eq!(
            store.checkpoint_from_runtime(&profile.id, runtime.revision, &refreshed),
            Err(ProxyAuthError::RevisionConflict)
        );
    }

    #[cfg(unix)]
    #[test]
    fn materialized_directory_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let store = store();
        store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), Some("A"))
            .unwrap();
        let parent = tempfile::tempdir().unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = store
            .materialize_enabled_for_runtime(parent.path())
            .unwrap();
        assert_eq!(
            fs::metadata(runtime.auth_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for entry in fs::read_dir(runtime.auth_dir()).unwrap() {
            let metadata = entry.unwrap().metadata().unwrap();
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn pending_runtime_blocks_cleanup_until_recovered() {
        use std::os::unix::fs::PermissionsExt;
        let store = store();
        store
            .import(&codex_fixture("acct-a", "access-a", "refresh-a"), Some("A"))
            .unwrap();
        let parent = tempfile::tempdir().unwrap();
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = store
            .materialize_enabled_for_runtime(parent.path())
            .unwrap();
        let auth_dir = runtime.auth_dir().to_path_buf();
        drop(runtime);

        assert_eq!(
            store.materialize_enabled_for_runtime(parent.path()).err(),
            Some(ProxyAuthError::RecoveryRequired)
        );
        assert!(
            auth_dir.exists(),
            "pending secret evidence must not be deleted"
        );
        let recovered = store.recover_pending_runtime(parent.path()).unwrap();
        assert_eq!(recovered.len(), 1);
        assert!(!auth_dir.exists());
        let next = store
            .materialize_enabled_for_runtime(parent.path())
            .unwrap();
        assert_eq!(next.profile_count(), 1);
    }

    #[test]
    fn validation_rejects_duplicate_keys_unknown_provider_and_missing_identity() {
        let duplicate = br#"{"type":"codex","type":"codex","account_id":"a","access_token":"a","refresh_token":"r"}"#;
        assert_eq!(
            validate_structure(duplicate),
            Err(ProxyAuthError::InvalidJsonObject)
        );
        let unknown =
            br#"{"type":"plugin","account_id":"a","access_token":"a","refresh_token":"r"}"#;
        assert_eq!(
            validate_structure(unknown),
            Err(ProxyAuthError::UnsupportedProvider)
        );
        let no_identity = br#"{"type":"codex","access_token":"a","refresh_token":"r"}"#;
        assert_eq!(
            validate_structure(no_identity),
            Err(ProxyAuthError::UnsupportedCredential)
        );
    }

    #[test]
    fn supported_provider_shapes_are_provider_aware() {
        let fixtures = [
            br#"{"type":"codex","account_id":"a","access_token":"a","refresh_token":"r"}"#.as_slice(),
            br#"{"type":"claude","account_uuid":"a","access_token":"a","refresh_token":"r"}"#.as_slice(),
            br#"{"type":"antigravity","email":"a@example.invalid","access_token":"a","refresh_token":"r"}"#.as_slice(),
            br#"{"type":"kimi","device_id":"a","access_token":"a","refresh_token":"r"}"#.as_slice(),
            br#"{"type":"xai","sub":"a","access_token":"a","refresh_token":"r"}"#.as_slice(),
        ];
        for fixture in fixtures {
            assert!(validate_structure(fixture).is_ok());
        }
    }
}
