//! Filesystem boundary for explicit Codex account switching.
//!
//! Profile secrets stay in the dedicated platform vault. This module only
//! accepts a native-picker path once, and replaces the single active
//! `auth.json` beneath the backend-resolved Codex home using the same no-follow,
//! CAS and atomic-exchange primitive as the AGENTS editor.

use crate::safe_fs::{self, FileStamp, SecretReadResult, write_secret_authorized};
use directories_next::BaseDirs;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub const MAX_AUTH_FILE_BYTES: u64 = 128 * 1024;

pub struct CredentialMutationLock {
    file: File,
}

impl Drop for CredentialMutationLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&self.file), libc::LOCK_UN);
        }
    }
}

pub fn codex_auth_home() -> Result<PathBuf, String> {
    let configured = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| BaseDirs::new().map(|base| base.home_dir().join(".codex")))
        .ok_or_else(|| "无法确定 Codex 认证目录。".to_string())?;
    let metadata = fs::symlink_metadata(&configured)
        .map_err(|_| "Codex 认证目录不存在或不可访问。".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Codex 认证目录无效或为符号链接。".into());
    }
    fs::canonicalize(configured).map_err(|_| "无法解析 Codex 认证目录。".to_string())
}

pub fn acquire_process_lock(data_dir: &Path) -> Result<CredentialMutationLock, String> {
    #[cfg(unix)]
    {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        };

        let vault_dir = data_dir.join("auth-vault");
        match fs::symlink_metadata(&vault_dir) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("认证档案锁目录无效。".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&vault_dir)
                    .map_err(|_| "无法创建认证档案锁目录。".to_string())?;
            }
            Err(_) => return Err("无法检查认证档案锁目录。".into()),
        }
        fs::set_permissions(&vault_dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法收紧认证档案锁目录权限。".to_string())?;
        let lock_path = vault_dir.join("mutation.lock");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(lock_path)
            .map_err(|_| "无法打开认证档案进程锁。".to_string())?;
        let metadata = file
            .metadata()
            .map_err(|_| "无法验证认证档案进程锁。".to_string())?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err("认证档案进程锁权限或所有者无效。".into());
        }
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            return Err("另一 Codex Manager 实例正在修改认证档案。".into());
        }
        Ok(CredentialMutationLock { file })
    }
    #[cfg(not(unix))]
    {
        let _ = data_dir;
        Err("认证档案进程锁当前仅支持 macOS。".into())
    }
}

pub fn read_native_import(path: &Path) -> Result<Zeroizing<Vec<u8>>, String> {
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return Err("请选择 .json 认证文件。".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        let before =
            fs::symlink_metadata(path).map_err(|_| "无法读取所选认证文件。".to_string())?;
        if before.file_type().is_symlink()
            || !before.is_file()
            || before.len() > MAX_AUTH_FILE_BYTES
            || before.uid() != unsafe { libc::geteuid() }
            || before.nlink() != 1
            || before.mode() & 0o077 != 0
        {
            return Err(
                "所选认证文件必须是当前用户拥有、权限不宽于 0600 的单链接普通 JSON 文件。".into(),
            );
        }
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| "无法安全打开所选认证文件。".to_string())?;
        let opened = file
            .metadata()
            .map_err(|_| "无法验证所选认证文件。".to_string())?;
        if !opened.is_file()
            || opened.uid() != unsafe { libc::geteuid() }
            || opened.nlink() != 1
            || opened.dev() != before.dev()
            || opened.ino() != before.ino()
            || opened.mode() != before.mode()
            || opened.len() != before.len()
            || opened.mtime() != before.mtime()
            || opened.mtime_nsec() != before.mtime_nsec()
            || opened.ctime() != before.ctime()
            || opened.ctime_nsec() != before.ctime_nsec()
            || opened.len() > MAX_AUTH_FILE_BYTES
        {
            return Err("所选认证文件在打开期间发生变化或所有者无效。".into());
        }
        let mut bytes = Zeroizing::new(Vec::with_capacity(opened.len() as usize));
        file.by_ref()
            .take(MAX_AUTH_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "无法读取所选认证文件。".to_string())?;
        if bytes.len() as u64 > MAX_AUTH_FILE_BYTES {
            return Err("认证文件超过 128 KiB 安全上限。".into());
        }
        let after = file
            .metadata()
            .map_err(|_| "无法复核所选认证文件。".to_string())?;
        if after.dev() != opened.dev()
            || after.ino() != opened.ino()
            || after.uid() != opened.uid()
            || after.nlink() != opened.nlink()
            || after.mode() != opened.mode()
            || after.len() != opened.len()
            || after.mtime() != opened.mtime()
            || after.mtime_nsec() != opened.mtime_nsec()
            || after.ctime() != opened.ctime()
            || after.ctime_nsec() != opened.ctime_nsec()
        {
            return Err("所选认证文件在读取期间发生变化，已拒绝导入。".into());
        }
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("认证文件导入当前仅支持 macOS。".into())
    }
}

pub fn read_active(home: &Path) -> Result<SecretReadResult, String> {
    safe_fs::read_secret_authorized(home, Path::new(""), "auth.json", MAX_AUTH_FILE_BYTES).map_err(
        |_| "未找到安全的 file 模式 auth.json；keyring/auto 存储当前不支持多账户轮换。".to_string(),
    )
}

pub fn replace_active(
    home: &Path,
    expected: &FileStamp,
    bytes: &[u8],
) -> Result<FileStamp, String> {
    let (stamp, ()) = write_secret_authorized(
        home,
        Path::new(""),
        "auth.json",
        expected,
        bytes,
        MAX_AUTH_FILE_BYTES,
        |_, _| Ok(()),
    )?;
    Ok(stamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn active_auth_requires_private_single_link_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("auth.json");
        fs::write(&path, b"{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_active(&root).unwrap().bytes.as_slice(), b"{}");

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_active(&root).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_import_requires_private_source_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.json");
        fs::write(&path, br#"{"tokens":{}}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_native_import(&path).unwrap().as_slice(),
            br#"{"tokens":{}}"#
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_native_import(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn active_auth_replace_is_private_and_rejects_stale_revision() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let path = root.join("auth.json");
        fs::write(&path, br#"{"tokens":{"access_token":"old"}}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let before = read_active(&root).unwrap();

        let after = replace_active(
            &root,
            &before.stamp,
            br#"{"tokens":{"access_token":"new"}}"#,
        )
        .unwrap();
        assert_ne!(after.sha256, before.stamp.sha256);
        assert_eq!(
            read_active(&root).unwrap().bytes.as_slice(),
            br#"{"tokens":{"access_token":"new"}}"#
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(replace_active(&root, &before.stamp, b"{}").is_err());
    }
}
