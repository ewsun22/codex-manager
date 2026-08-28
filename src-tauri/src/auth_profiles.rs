//! Encrypted-at-rest OAuth authentication profile metadata.
//!
//! This module owns only the private Keychain records for imported Codex
//! ChatGPT OAuth credentials.  It deliberately does not read or write the
//! official Codex `auth.json`, start a browser, or determine an account's
//! identity.  The caller must obtain `verified_identity_fingerprint` from the
//! signed official Codex App Server running in an isolated environment.
//!
//! Raw credential JSON is accepted only as bytes, kept in `Zeroizing`, and is
//! never `Serialize`, `Debug`, logged, or included in public DTOs.

use chrono::{DateTime, Duration, Utc};
use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
use thiserror::Error;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const KEYCHAIN_SERVICE: &str = "cc.codex.manager.auth-profiles.v1";
const INDEX_ACCOUNT: &str = "index";
const PROFILE_ACCOUNT_PREFIX: &str = "profile:";
const MAX_IMPORT_BYTES: usize = 128 * 1024;
const MAX_LABEL_BYTES: usize = 64;
pub const SOFT_DELETE_RETENTION_DAYS: u8 = 30;
const INDEX_VERSION: u8 = 1;
const MAX_JSON_NESTING: usize = 16;
const MAX_JSON_FIELDS: usize = 128;

/// Public, intentionally non-sensitive profile metadata.
///
/// It has no email address, source path/name, token, identity fingerprint, or
/// credential-derived hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfile {
    pub id: String,
    pub label: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileList {
    pub active_profile_id: Option<String>,
    pub profiles: Vec<AuthProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub profile: AuthProfile,
    /// `created`, `updated`, or `restored`; callers never need raw identities
    /// to decide which UI notice to display.
    pub action: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthProfileError {
    #[cfg(not(target_os = "macos"))]
    #[error("认证档案功能当前仅支持 macOS Keychain。")]
    UnsupportedPlatform,
    #[error("无法访问 macOS Keychain。")]
    KeychainUnavailable,
    #[error("认证文件为空或超过 128 KiB 限制。")]
    InvalidSize,
    #[error("认证文件必须是 JSON object。")]
    InvalidJsonObject,
    #[error("认证文件不是受支持的 ChatGPT OAuth 凭据。")]
    UnsupportedCredential,
    #[error("认证档案标签不符合要求。")]
    InvalidLabel,
    #[error("经验证的身份指纹无效。")]
    InvalidIdentityFingerprint,
    #[error("认证档案不存在。")]
    NotFound,
    #[error("认证档案已删除。")]
    AlreadyDeleted,
    #[error("认证档案尚未删除。")]
    NotDeleted,
    #[error("不能删除当前生效的认证档案。")]
    CannotDeleteActive,
    #[error("至少必须保留一个可用认证档案。")]
    CannotDeleteLastProfile,
    #[error("更新后的凭据不属于目标认证档案。")]
    IdentityMismatch,
    #[error("认证档案元数据损坏。")]
    CorruptIndex,
}

pub type Result<T> = std::result::Result<T, AuthProfileError>;

/// The Keychain abstraction is intentionally private.  Unit tests exercise
/// all state transitions through an in-memory implementation and never touch
/// the user's real Keychain.
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
            // `errSecItemNotFound`; do not expose platform error details.
            Err(error) if error.code() == -25300 => Ok(None),
            Err(_) => Err(AuthProfileError::KeychainUnavailable),
        }
    }

    fn write(&self, account: &str, bytes: &[u8]) -> Result<()> {
        security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, account, bytes)
            .map_err(|_| AuthProfileError::KeychainUnavailable)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match security_framework::passwords::delete_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(AuthProfileError::KeychainUnavailable),
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedKeychain;

#[cfg(not(target_os = "macos"))]
impl KeychainBackend for UnsupportedKeychain {
    fn read(&self, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Err(AuthProfileError::UnsupportedPlatform)
    }

    fn write(&self, _: &str, _: &[u8]) -> Result<()> {
        Err(AuthProfileError::UnsupportedPlatform)
    }

    fn delete(&self, _: &str) -> Result<()> {
        Err(AuthProfileError::UnsupportedPlatform)
    }
}

/// Keychain-backed profile repository.  Construct it once in `AppState`.
pub struct AuthProfileStore {
    backend: Arc<dyn KeychainBackend>,
}

impl AuthProfileStore {
    pub fn load() -> Self {
        #[cfg(target_os = "macos")]
        let backend: Arc<dyn KeychainBackend> = Arc::new(MacKeychain);
        #[cfg(not(target_os = "macos"))]
        let backend: Arc<dyn KeychainBackend> = Arc::new(UnsupportedKeychain);
        Self { backend }
    }

    /// Lists profile metadata.  Deleted profiles are included only when the
    /// caller explicitly asks for recovery UI.
    pub fn list(&self, include_deleted: bool) -> Result<AuthProfileList> {
        let index = self.read_index()?;
        Ok(index.to_public(include_deleted))
    }

    /// Checks an identity fingerprint without returning, serializing, or
    /// logging the stored fingerprint. This is the native post-switch check
    /// used by the caller after the official App Server identifies the active
    /// isolated Codex environment.
    pub fn profile_matches_verified_identity(
        &self,
        profile_id: &str,
        verified_identity_fingerprint: &str,
    ) -> Result<bool> {
        let fingerprint = normalize_identity_fingerprint(verified_identity_fingerprint)?;
        Ok(self
            .read_index()?
            .active_record(profile_id)?
            .identity_fingerprint
            .as_str()
            == fingerprint)
    }

    /// Reconciles persisted metadata with an identity confirmed by the signed
    /// official App Server.  No live environment is modified here.  When the
    /// server observes no authenticated identity, `None` safely clears active.
    pub fn reconcile_active_identity(
        &self,
        verified_identity_fingerprint: Option<&str>,
    ) -> Result<AuthProfileList> {
        let expected = verified_identity_fingerprint
            .map(normalize_identity_fingerprint)
            .transpose()?;
        let mut index = self.read_index()?;
        let selected = expected.and_then(|fingerprint| {
            index
                .profiles
                .iter()
                .find(|record| {
                    record.deleted_at.is_none() && record.identity_fingerprint == fingerprint
                })
                .map(|record| record.id.clone())
        });
        if index.active_profile_id != selected {
            index.active_profile_id = selected;
            self.write_index(&index)?;
        }
        Ok(index.to_public(false))
    }

    /// Validates and stores a ChatGPT OAuth document.
    ///
    /// `verified_identity_fingerprint` is deliberately caller-supplied: it
    /// must be a SHA-256 hex fingerprint derived only after the signed official
    /// Codex App Server has validated this exact credential in an isolated
    /// CODEX_HOME.  This module never interprets JWTs or raw account IDs.
    pub fn import_validated(
        &self,
        source: &[u8],
        label: Option<&str>,
        verified_identity_fingerprint: &str,
    ) -> Result<ImportOutcome> {
        let secret = validated_secret(source)?;
        let fingerprint = normalize_identity_fingerprint(verified_identity_fingerprint)?;
        let label = normalize_label(label)?;
        let now = Utc::now();
        let mut index = self.read_index()?;
        let active_id = index.active_profile_id.clone();

        if let Some(position) = index
            .profiles
            .iter()
            .position(|record| record.identity_fingerprint == fingerprint)
        {
            let profile_id = index.profiles[position].id.clone();
            let secret_account = profile_account(&profile_id);
            let old_secret = self.backend.read(&secret_account)?;
            let record = &mut index.profiles[position];
            // Write first so metadata never claims a profile with a missing
            // secret.  The credential bytes are zeroized after this function.
            self.backend.write(&secret_account, &secret)?;
            record.updated_at = now;
            let action = if record.deleted_at.take().is_some() {
                "restored"
            } else {
                "updated"
            };
            record.label = label;
            let public = public_record(record, active_id.as_deref() == Some(&record.id));
            if let Err(error) = self.write_index(&index) {
                self.restore_or_remove_secret(
                    &secret_account,
                    old_secret.as_ref().map(|secret| secret.as_slice()),
                );
                return Err(error);
            }
            return Ok(ImportOutcome {
                profile: public,
                action: action.into(),
            });
        }

        let record = ProfileRecord {
            id: Uuid::new_v4().to_string(),
            label,
            identity_fingerprint: fingerprint,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        let secret_account = profile_account(&record.id);
        self.backend.write(&secret_account, &secret)?;
        let public = index.public_record(&record);
        index.profiles.push(record);
        if let Err(error) = self.write_index(&index) {
            self.restore_or_remove_secret(&secret_account, None);
            return Err(error);
        }
        Ok(ImportOutcome {
            profile: public,
            action: "created".into(),
        })
    }

    /// Returns raw credential bytes only to the native caller.  The return
    /// type intentionally cannot be serialized by this module.
    pub fn read_target_secret(&self, profile_id: &str) -> Result<Zeroizing<Vec<u8>>> {
        let index = self.read_index()?;
        let record = index.active_record(profile_id)?;
        self.backend
            .read(&profile_account(&record.id))?
            .ok_or(AuthProfileError::NotFound)
    }

    /// Replaces a profile secret after the caller has independently verified
    /// that the exact candidate belongs to this existing profile.
    pub fn update_secret(
        &self,
        profile_id: &str,
        source: &[u8],
        verified_identity_fingerprint: &str,
    ) -> Result<AuthProfile> {
        let secret = validated_secret(source)?;
        let fingerprint = normalize_identity_fingerprint(verified_identity_fingerprint)?;
        let mut index = self.read_index()?;
        let active_id = index.active_profile_id.clone();
        let position = index.position(profile_id)?;
        let secret_account = profile_account(&index.profiles[position].id);
        let old_secret = self.backend.read(&secret_account)?;
        let public = {
            let record = index.active_record_mut(profile_id)?;
            if record.identity_fingerprint != fingerprint {
                return Err(AuthProfileError::IdentityMismatch);
            }
            self.backend.write(&secret_account, &secret)?;
            record.updated_at = Utc::now();
            public_record(record, active_id.as_deref() == Some(&record.id))
        };
        if let Err(error) = self.write_index(&index) {
            self.restore_or_remove_secret(
                &secret_account,
                old_secret.as_ref().map(|secret| secret.as_slice()),
            );
            return Err(error);
        }
        Ok(public)
    }

    /// Marks a profile recoverable for 30 days.  The Keychain secret remains
    /// until `purge_expired` succeeds, so restore never needs an external file.
    pub fn soft_delete(&self, profile_id: &str) -> Result<AuthProfileList> {
        let mut index = self.read_index()?;
        let available_count = index
            .profiles
            .iter()
            .filter(|record| record.deleted_at.is_none())
            .count();
        let target_position = index.position(profile_id)?;
        let target = &index.profiles[target_position];
        if target.deleted_at.is_some() {
            return Err(AuthProfileError::AlreadyDeleted);
        }
        if index.active_profile_id.as_deref() == Some(&target.id) {
            return Err(AuthProfileError::CannotDeleteActive);
        }
        if available_count <= 1 {
            return Err(AuthProfileError::CannotDeleteLastProfile);
        }
        let now = Utc::now();
        let target = &mut index.profiles[target_position];
        target.deleted_at = Some(now);
        target.updated_at = now;
        self.write_index(&index)?;
        Ok(index.to_public(true))
    }

    pub fn restore(&self, profile_id: &str) -> Result<AuthProfile> {
        let mut index = self.read_index()?;
        let active_id = index.active_profile_id.clone();
        let record = index.record_mut(profile_id)?;
        if record.deleted_at.is_none() {
            return Err(AuthProfileError::NotDeleted);
        }
        if self.backend.read(&profile_account(&record.id))?.is_none() {
            return Err(AuthProfileError::NotFound);
        }
        record.deleted_at = None;
        record.updated_at = Utc::now();
        let public = public_record(record, active_id.as_deref() == Some(&record.id));
        self.write_index(&index)?;
        Ok(public)
    }

    /// Persists which profile is active *after* the caller has atomically
    /// switched the live isolated Codex environment and verified it.  This
    /// method must never be used as the switch itself.
    pub fn set_active_after_live_switch(&self, profile_id: &str) -> Result<AuthProfile> {
        let mut index = self.read_index()?;
        let position = index.position(profile_id)?;
        if index.profiles[position].deleted_at.is_some() {
            return Err(AuthProfileError::AlreadyDeleted);
        }
        index.active_profile_id = Some(index.profiles[position].id.clone());
        let public = index.public_record(&index.profiles[position]);
        self.write_index(&index)?;
        Ok(public)
    }

    /// Removes secrets and metadata whose soft-delete retention elapsed.
    /// Individual Keychain deletions happen before publishing the new index;
    /// if a deletion fails, its metadata remains so a later cleanup can retry.
    pub fn purge_expired(&self, now: DateTime<Utc>) -> Result<usize> {
        let mut index = self.read_index()?;
        let expired: Vec<String> = index
            .profiles
            .iter()
            .filter_map(|record| {
                let deleted_at = record.deleted_at?;
                (deleted_at + Duration::days(i64::from(SOFT_DELETE_RETENTION_DAYS)) <= now)
                    .then(|| record.id.clone())
            })
            .collect();
        let mut deleted_secrets = Vec::with_capacity(expired.len());
        for id in &expired {
            let account = profile_account(id);
            let previous = match self.backend.read(&account) {
                Ok(previous) => previous,
                Err(error) => {
                    self.restore_deleted_secrets(&deleted_secrets);
                    return Err(error);
                }
            };
            if let Err(error) = self.backend.delete(&account) {
                self.restore_deleted_secrets(&deleted_secrets);
                return Err(error);
            }
            if let Some(secret) = previous {
                deleted_secrets.push((account, secret));
            }
        }
        if expired.is_empty() {
            return Ok(0);
        }
        let expired: HashSet<&str> = expired.iter().map(String::as_str).collect();
        index
            .profiles
            .retain(|record| !expired.contains(record.id.as_str()));
        if let Err(error) = self.write_index(&index) {
            self.restore_deleted_secrets(&deleted_secrets);
            return Err(error);
        }
        Ok(expired.len())
    }

    fn read_index(&self) -> Result<ProfileIndex> {
        let Some(bytes) = self.backend.read(INDEX_ACCOUNT)? else {
            return Ok(ProfileIndex::empty());
        };
        serde_json::from_slice::<ProfileIndex>(&bytes)
            .map_err(|_| AuthProfileError::CorruptIndex)
            .and_then(ProfileIndex::validate)
    }

    fn write_index(&self, index: &ProfileIndex) -> Result<()> {
        let encoded = serde_json::to_vec(index).map_err(|_| AuthProfileError::CorruptIndex)?;
        self.backend.write(INDEX_ACCOUNT, &encoded)
    }

    /// Best-effort compensation for a failed metadata commit. A successful
    /// caller-visible operation always has both its Keychain secret and index;
    /// if compensation itself fails, the stable Keychain error avoids claiming
    /// that the operation committed.
    fn restore_or_remove_secret(&self, account: &str, previous: Option<&[u8]>) {
        match previous {
            Some(secret) => {
                let _ = self.backend.write(account, secret);
            }
            None => {
                let _ = self.backend.delete(account);
            }
        }
    }

    fn restore_deleted_secrets(&self, deleted_secrets: &[(String, Zeroizing<Vec<u8>>)]) {
        for (account, secret) in deleted_secrets.iter().rev() {
            let _ = self.backend.write(account, secret);
        }
    }

    #[cfg(test)]
    fn with_backend(backend: Arc<dyn KeychainBackend>) -> Self {
        Self { backend }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileIndex {
    version: u8,
    active_profile_id: Option<String>,
    profiles: Vec<ProfileRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileRecord {
    id: String,
    label: String,
    identity_fingerprint: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

impl ProfileIndex {
    fn empty() -> Self {
        Self {
            version: INDEX_VERSION,
            active_profile_id: None,
            profiles: Vec::new(),
        }
    }

    fn validate(self) -> Result<Self> {
        if self.version != INDEX_VERSION {
            return Err(AuthProfileError::CorruptIndex);
        }
        let mut ids = HashSet::new();
        for record in &self.profiles {
            if Uuid::parse_str(&record.id).is_err()
                || normalize_label(Some(&record.label)).is_err()
                || normalize_identity_fingerprint(&record.identity_fingerprint).is_err()
                || !ids.insert(record.id.as_str())
            {
                return Err(AuthProfileError::CorruptIndex);
            }
        }
        if let Some(active_id) = &self.active_profile_id {
            let Some(active) = self.profiles.iter().find(|record| &record.id == active_id) else {
                return Err(AuthProfileError::CorruptIndex);
            };
            if active.deleted_at.is_some() {
                return Err(AuthProfileError::CorruptIndex);
            }
        }
        Ok(self)
    }

    fn position(&self, id: &str) -> Result<usize> {
        self.profiles
            .iter()
            .position(|record| record.id == id)
            .ok_or(AuthProfileError::NotFound)
    }

    fn record_mut(&mut self, id: &str) -> Result<&mut ProfileRecord> {
        self.profiles
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or(AuthProfileError::NotFound)
    }

    fn active_record(&self, id: &str) -> Result<&ProfileRecord> {
        let record = self
            .profiles
            .iter()
            .find(|record| record.id == id)
            .ok_or(AuthProfileError::NotFound)?;
        if record.deleted_at.is_some() {
            return Err(AuthProfileError::AlreadyDeleted);
        }
        Ok(record)
    }

    fn active_record_mut(&mut self, id: &str) -> Result<&mut ProfileRecord> {
        let record = self.record_mut(id)?;
        if record.deleted_at.is_some() {
            return Err(AuthProfileError::AlreadyDeleted);
        }
        Ok(record)
    }

    fn public_record(&self, record: &ProfileRecord) -> AuthProfile {
        public_record(
            record,
            self.active_profile_id.as_deref() == Some(&record.id),
        )
    }

    fn to_public(&self, include_deleted: bool) -> AuthProfileList {
        AuthProfileList {
            active_profile_id: self.active_profile_id.clone(),
            profiles: self
                .profiles
                .iter()
                .filter(|record| include_deleted || record.deleted_at.is_none())
                .map(|record| self.public_record(record))
                .collect(),
        }
    }
}

fn public_record(record: &ProfileRecord, is_active: bool) -> AuthProfile {
    AuthProfile {
        id: record.id.clone(),
        label: record.label.clone(),
        is_active,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        deleted_at: record.deleted_at.map(|value| value.to_rfc3339()),
    }
}

fn profile_account(id: &str) -> String {
    format!("{PROFILE_ACCOUNT_PREFIX}{id}")
}

fn normalize_label(label: Option<&str>) -> Result<String> {
    let label = label.unwrap_or("Codex 认证档案").trim();
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || label.contains('@')
        || label.contains('/')
        || label.contains('\\')
        || label.chars().any(char::is_control)
    {
        return Err(AuthProfileError::InvalidLabel);
    }
    Ok(label.to_owned())
}

fn normalize_identity_fingerprint(value: &str) -> Result<String> {
    let value = value.trim();
    if value.len() != 64 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(AuthProfileError::InvalidIdentityFingerprint);
    }
    Ok(value.to_ascii_lowercase())
}

/// Performs only bounded JSON and ChatGPT OAuth shape validation. It neither
/// invokes Codex nor derives identity from JWT/account-id content. Main calls
/// this before starting the isolated official App Server; import repeats it.
pub fn validate_structure(source: &[u8]) -> Result<()> {
    if source.is_empty() || source.len() > MAX_IMPORT_BYTES {
        return Err(AuthProfileError::InvalidSize);
    }
    validate_json_shape(source)?;
    let mut value: Value =
        serde_json::from_slice(source).map_err(|_| AuthProfileError::InvalidJsonObject)?;
    let result = validate_chatgpt_oauth_value(&value);
    zeroize_json_value(&mut value);
    result
}

fn validated_secret(source: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    validate_structure(source)?;
    Ok(Zeroizing::new(source.to_vec()))
}

fn validate_chatgpt_oauth_value(value: &Value) -> Result<()> {
    let root = value
        .as_object()
        .ok_or(AuthProfileError::InvalidJsonObject)?;
    reject_api_key_values(value)?;
    for key in ["auth_mode", "authMode", "type", "provider"] {
        if let Some(Value::String(mode)) = root.get(key)
            && !mode.eq_ignore_ascii_case("chatgpt")
        {
            return Err(AuthProfileError::UnsupportedCredential);
        }
    }
    let tokens = root
        .get("tokens")
        .and_then(Value::as_object)
        .ok_or(AuthProfileError::UnsupportedCredential)?;
    for required in ["access_token", "refresh_token"] {
        let Some(Value::String(token)) = tokens.get(required) else {
            return Err(AuthProfileError::UnsupportedCredential);
        };
        if token.trim().is_empty() || token.len() > 16 * 1024 || token.chars().any(char::is_control)
        {
            return Err(AuthProfileError::UnsupportedCredential);
        }
    }
    Ok(())
}

fn reject_api_key_values(value: &Value) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let api_key_name = matches!(key.as_str(), "api_key" | "apiKey" | "OPENAI_API_KEY");
                if api_key_name && value.as_str().is_some_and(|text| !text.trim().is_empty()) {
                    return Err(AuthProfileError::UnsupportedCredential);
                }
                reject_api_key_values(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_api_key_values(value)?;
            }
        }
        _ => {}
    }
    Ok(())
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
        .map_err(|_| AuthProfileError::InvalidJsonObject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, sync::Mutex};

    #[derive(Default)]
    struct MemoryKeychain {
        entries: Mutex<HashMap<String, Vec<u8>>>,
        failed_writes: Mutex<HashMap<String, usize>>,
        failed_deletes: Mutex<HashMap<String, usize>>,
    }

    impl MemoryKeychain {
        fn fail_next_write(&self, account: &str) {
            self.failed_writes
                .lock()
                .unwrap()
                .entry(account.to_owned())
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }

        fn fail_next_delete(&self, account: &str) {
            self.failed_deletes
                .lock()
                .unwrap()
                .entry(account.to_owned())
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }

        fn consume_failure(failures: &Mutex<HashMap<String, usize>>, account: &str) -> bool {
            let mut failures = failures.lock().unwrap();
            let Some(count) = failures.get_mut(account) else {
                return false;
            };
            *count -= 1;
            if *count == 0 {
                failures.remove(account);
            }
            true
        }

        fn value(&self, account: &str) -> Option<Vec<u8>> {
            self.entries.lock().unwrap().get(account).cloned()
        }

        fn profile_secret_count(&self) -> usize {
            self.entries
                .lock()
                .unwrap()
                .keys()
                .filter(|account| account.starts_with(PROFILE_ACCOUNT_PREFIX))
                .count()
        }
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
            if Self::consume_failure(&self.failed_writes, account) {
                return Err(AuthProfileError::KeychainUnavailable);
            }
            self.entries
                .lock()
                .unwrap()
                .insert(account.to_owned(), bytes.to_vec());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<()> {
            if Self::consume_failure(&self.failed_deletes, account) {
                return Err(AuthProfileError::KeychainUnavailable);
            }
            self.entries.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn store() -> AuthProfileStore {
        AuthProfileStore::with_backend(Arc::new(MemoryKeychain::default()))
    }

    fn store_with_failures() -> (AuthProfileStore, Arc<MemoryKeychain>) {
        let backend = Arc::new(MemoryKeychain::default());
        let store = AuthProfileStore::with_backend(backend.clone());
        (store, backend)
    }
    fn fingerprint(seed: u8) -> String {
        format!("{seed:064x}")
    }
    fn fixture(access: &str, refresh: &str) -> Vec<u8> {
        format!(r#"{{"OPENAI_API_KEY":null,"tokens":{{"access_token":"{access}","refresh_token":"{refresh}"}}}}"#).into_bytes()
    }

    #[test]
    fn public_dto_is_allowlisted() {
        let profile = AuthProfile {
            id: "uuid".into(),
            label: "个人".into(),
            is_active: true,
            created_at: "now".into(),
            updated_at: "now".into(),
            deleted_at: None,
        };
        let rendered = serde_json::to_string(&profile).unwrap();
        for forbidden in ["email", "token", "path", "hash", "filename", "fingerprint"] {
            assert!(!rendered.contains(forbidden));
        }
    }

    #[test]
    fn accepts_chatgpt_tokens_but_rejects_api_key_and_jwt_identity_is_not_read() {
        assert!(validate_structure(&fixture("access", "refresh")).is_ok());
        let api_key = br#"{"tokens":{"access_token":"a","refresh_token":"b"},"OPENAI_API_KEY":"sk-real-key"}"#;
        assert_eq!(
            validate_structure(api_key),
            Err(AuthProfileError::UnsupportedCredential)
        );
        let missing_refresh = br#"{"tokens":{"access_token":"a"}}"#;
        assert_eq!(
            validate_structure(missing_refresh),
            Err(AuthProfileError::UnsupportedCredential)
        );
        let control_token = br#"{"tokens":{"access_token":"a","refresh_token":"line\nbreak"}}"#;
        assert_eq!(
            validate_structure(control_token),
            Err(AuthProfileError::UnsupportedCredential)
        );
        let no_object = br#"[]"#;
        assert_eq!(
            validate_structure(no_object),
            Err(AuthProfileError::InvalidJsonObject)
        );
    }

    #[test]
    fn validation_rejects_duplicate_keys_and_excessive_json_shape() {
        let duplicate =
            br#"{"tokens":{"access_token":"a","access_token":"b","refresh_token":"r"}}"#;
        assert_eq!(
            validate_structure(duplicate),
            Err(AuthProfileError::InvalidJsonObject)
        );
        let deep_value = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_NESTING + 1),
            "]".repeat(MAX_JSON_NESTING + 1)
        );
        let deep = format!(
            r#"{{"tokens":{{"access_token":"a","refresh_token":"r"}},"extra":{deep_value}}}"#
        );
        assert_eq!(
            validate_structure(deep.as_bytes()),
            Err(AuthProfileError::InvalidJsonObject)
        );
        let fields = (0..MAX_JSON_FIELDS)
            .map(|index| format!(r#""field{index}":null"#))
            .collect::<Vec<_>>()
            .join(",");
        let many = format!(r#"{{"tokens":{{"access_token":"a","refresh_token":"r"}},{fields}}}"#);
        assert_eq!(
            validate_structure(many.as_bytes()),
            Err(AuthProfileError::InvalidJsonObject)
        );
    }

    #[test]
    fn import_deduplicates_only_on_caller_verified_fingerprint() {
        let store = store();
        let first = store
            .import_validated(
                &fixture("first", "first-refresh"),
                Some("个人"),
                &fingerprint(1),
            )
            .unwrap();
        let updated = store
            .import_validated(
                &fixture("second", "second-refresh"),
                Some("更新标签"),
                &fingerprint(1),
            )
            .unwrap();
        assert_eq!(first.profile.id, updated.profile.id);
        assert_eq!(updated.action, "updated");
        assert_eq!(store.list(false).unwrap().profiles.len(), 1);
        assert_eq!(
            store
                .read_target_secret(&first.profile.id)
                .unwrap()
                .as_slice(),
            fixture("second", "second-refresh").as_slice()
        );
    }

    #[test]
    fn new_import_removes_orphan_secret_when_index_write_fails() {
        let (store, backend) = store_with_failures();
        backend.fail_next_write(INDEX_ACCOUNT);

        assert_eq!(
            store.import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1)),
            Err(AuthProfileError::KeychainUnavailable)
        );
        assert_eq!(backend.profile_secret_count(), 0);
        assert_eq!(backend.value(INDEX_ACCOUNT), None);
    }

    #[test]
    fn update_restores_previous_secret_when_index_write_fails() {
        let (store, backend) = store_with_failures();
        let profile = store
            .import_validated(&fixture("old", "old-refresh"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        backend.fail_next_write(INDEX_ACCOUNT);

        assert_eq!(
            store.update_secret(&profile.id, &fixture("new", "new-refresh"), &fingerprint(1)),
            Err(AuthProfileError::KeychainUnavailable)
        );
        assert_eq!(
            backend.value(&profile_account(&profile.id)),
            Some(fixture("old", "old-refresh"))
        );
    }

    #[test]
    fn duplicate_import_restores_previous_secret_when_index_write_fails() {
        let (store, backend) = store_with_failures();
        let profile = store
            .import_validated(
                &fixture("old", "old-refresh"),
                Some("原标签"),
                &fingerprint(1),
            )
            .unwrap()
            .profile;
        backend.fail_next_write(INDEX_ACCOUNT);

        assert_eq!(
            store.import_validated(
                &fixture("new", "new-refresh"),
                Some("新标签"),
                &fingerprint(1)
            ),
            Err(AuthProfileError::KeychainUnavailable)
        );
        assert_eq!(
            backend.value(&profile_account(&profile.id)),
            Some(fixture("old", "old-refresh"))
        );
        assert_eq!(store.list(false).unwrap().profiles[0].label, "原标签");
    }

    #[test]
    fn soft_delete_restore_and_expiry_preserve_then_remove_secret() {
        let store = store();
        let active = store
            .import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        let other = store
            .import_validated(&fixture("b", "rb"), Some("B"), &fingerprint(2))
            .unwrap()
            .profile;
        store.set_active_after_live_switch(&active.id).unwrap();
        assert_eq!(
            store.soft_delete(&active.id),
            Err(AuthProfileError::CannotDeleteActive)
        );
        store.soft_delete(&other.id).unwrap();
        assert!(store.read_target_secret(&other.id).is_err());
        assert!(store.restore(&other.id).is_ok());
        store.soft_delete(&other.id).unwrap();
        assert_eq!(store.purge_expired(Utc::now()), Ok(0));
        assert_eq!(store.purge_expired(Utc::now() + Duration::days(31)), Ok(1));
        assert_eq!(store.restore(&other.id), Err(AuthProfileError::NotFound));
    }

    #[test]
    fn purge_restores_previously_deleted_secrets_when_later_delete_fails() {
        let (store, backend) = store_with_failures();
        let first = store
            .import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        let second = store
            .import_validated(&fixture("b", "rb"), Some("B"), &fingerprint(2))
            .unwrap()
            .profile;
        let third = store
            .import_validated(&fixture("c", "rc"), Some("C"), &fingerprint(3))
            .unwrap()
            .profile;
        store.set_active_after_live_switch(&first.id).unwrap();
        store.soft_delete(&second.id).unwrap();
        store.soft_delete(&third.id).unwrap();
        backend.fail_next_delete(&profile_account(&third.id));

        assert_eq!(
            store.purge_expired(Utc::now() + Duration::days(31)),
            Err(AuthProfileError::KeychainUnavailable)
        );
        assert_eq!(
            backend.value(&profile_account(&second.id)),
            Some(fixture("b", "rb"))
        );
        assert_eq!(
            backend.value(&profile_account(&third.id)),
            Some(fixture("c", "rc"))
        );
        assert_eq!(store.list(true).unwrap().profiles.len(), 3);
    }

    #[test]
    fn purge_restores_secret_when_index_write_fails() {
        let (store, backend) = store_with_failures();
        let active = store
            .import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        let deleted = store
            .import_validated(&fixture("b", "rb"), Some("B"), &fingerprint(2))
            .unwrap()
            .profile;
        store.set_active_after_live_switch(&active.id).unwrap();
        store.soft_delete(&deleted.id).unwrap();
        backend.fail_next_write(INDEX_ACCOUNT);

        assert_eq!(
            store.purge_expired(Utc::now() + Duration::days(31)),
            Err(AuthProfileError::KeychainUnavailable)
        );
        assert_eq!(
            backend.value(&profile_account(&deleted.id)),
            Some(fixture("b", "rb"))
        );
        assert_eq!(store.list(true).unwrap().profiles.len(), 2);
    }

    #[test]
    fn update_requires_same_verified_identity_without_changing_active_profile() {
        let store = store();
        let one = store
            .import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        let _other = store
            .import_validated(&fixture("b", "rb"), Some("B"), &fingerprint(2))
            .unwrap()
            .profile;
        store.set_active_after_live_switch(&one.id).unwrap();
        assert_eq!(
            store.update_secret(&one.id, &fixture("new", "rn"), &fingerprint(2)),
            Err(AuthProfileError::IdentityMismatch)
        );
        assert_eq!(store.list(false).unwrap().active_profile_id, Some(one.id));
    }

    #[test]
    fn reconcile_only_persists_identity_observed_by_the_official_boundary() {
        let store = store();
        let imported = store
            .import_validated(&fixture("a", "ra"), Some("A"), &fingerprint(1))
            .unwrap()
            .profile;
        assert_eq!(store.list(false).unwrap().active_profile_id, None);
        assert_eq!(
            store
                .reconcile_active_identity(Some(&fingerprint(1)))
                .unwrap()
                .active_profile_id,
            Some(imported.id.clone())
        );
        assert!(
            store
                .profile_matches_verified_identity(&imported.id, &fingerprint(1))
                .unwrap()
        );
        assert!(
            !store
                .profile_matches_verified_identity(&imported.id, &fingerprint(2))
                .unwrap()
        );
        assert_eq!(
            store
                .reconcile_active_identity(None)
                .unwrap()
                .active_profile_id,
            None
        );
    }
}
