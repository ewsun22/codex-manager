//! Bounded, no-follow file operations used by the AGENTS editor.
//!
//! On Unix the final directory is reached from an already-open authorized root
//! using `openat(2)` with `O_NOFOLLOW`. Reads and replacements therefore remain
//! anchored to that directory even if another process races path resolution.

use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub sha256: String,
    pub mtime_ms: i64,
    pub identity: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct ReadResult {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub stamp: FileStamp,
    pub writable: bool,
}

/// A regular file opened relative to an already-authorized directory tree.
/// Metadata used for checkpointing is taken from this exact descriptor.
pub struct OpenedRegularFile {
    pub file: File,
    pub identity: String,
    pub size_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    Replace,
    Create,
}

#[derive(Clone, Debug)]
pub struct WriteExpectation<'a> {
    pub sha256: &'a str,
    pub mtime_ms: i64,
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

pub fn newline_style(bytes: &[u8]) -> &'static str {
    if bytes.windows(2).any(|pair| pair == b"\r\n") {
        "crlf"
    } else {
        "lf"
    }
}

pub fn preserve_newlines(content: &str, style: &str) -> Vec<u8> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if style == "crlf" {
        normalized.replace('\n', "\r\n").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

pub fn canonical_authorized_root(root: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(root).map_err(display_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("授权根目录无效或为符号链接：{}", root.display()));
    }
    fs::canonicalize(root).map_err(display_error)
}

pub fn validate_relative_target(
    root: &Path,
    target: &Path,
    allowed_names: &[String],
) -> Result<(PathBuf, PathBuf, String), String> {
    let root = canonical_authorized_root(root)?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "AGENTS 文件名必须是有效 UTF-8。".to_string())?
        .to_string();
    if !allowed_names.iter().any(|candidate| candidate == &name) {
        return Err(format!("不允许编辑该文件名：{name}"));
    }

    let parent = target
        .parent()
        .ok_or_else(|| "AGENTS 文件没有父目录。".to_string())?;
    let canonical_parent = fs::canonicalize(parent).map_err(display_error)?;
    if !canonical_parent.starts_with(&root) {
        return Err("AGENTS 文件不在授权根目录内。".into());
    }
    let relative_parent = canonical_parent
        .strip_prefix(&root)
        .map_err(display_error)?
        .to_path_buf();
    if relative_parent
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("AGENTS 文件父目录包含不安全路径组件。".into());
    }
    Ok((root, relative_parent, name))
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        io,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
    };

    fn cstring(value: &OsStr) -> Result<CString, String> {
        CString::new(value.as_bytes()).map_err(|_| "路径包含 NUL 字节。".to_string())
    }

    fn open_root(root: &Path) -> Result<File, String> {
        if !root.is_absolute() {
            return Err("授权根目录必须是绝对路径。".into());
        }
        let slash = CString::new("/").expect("literal contains no NUL");
        // Start at the filesystem root and resolve every component with
        // `openat(O_NOFOLLOW)`. A single `open(root, O_NOFOLLOW)` only protects
        // the final component and would still follow a raced ancestor symlink.
        // SAFETY: `slash` is a valid C string and the returned descriptor is
        // immediately owned by `File`.
        let fd = unsafe {
            libc::open(
                slash.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: `fd` was freshly returned by `open` and has one owner.
        let mut current = unsafe { File::from_raw_fd(fd) };
        for component in root.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                _ => return Err("授权根目录包含不安全路径组件。".into()),
            };
            let name = cstring(name)?;
            // SAFETY: both descriptor and C string are valid for this call.
            let next = unsafe {
                libc::openat(
                    current.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if next < 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            // SAFETY: `next` is a newly-created descriptor with one owner.
            current = unsafe { File::from_raw_fd(next) };
        }
        Ok(current)
    }

    fn open_parent(root: &Path, relative_parent: &Path) -> Result<File, String> {
        let mut current = open_root(root)?;
        for component in relative_parent.components() {
            let name = match component {
                Component::Normal(name) => name,
                Component::CurDir => continue,
                _ => return Err("相对路径包含不安全组件。".into()),
            };
            let name = cstring(name)?;
            // SAFETY: both descriptor and C string are valid for this call.
            let next = unsafe {
                libc::openat(
                    current.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if next < 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            // SAFETY: `next` is a newly-created descriptor with one owner.
            current = unsafe { File::from_raw_fd(next) };
        }
        Ok(current)
    }

    fn open_file(parent: &File, name: &str, writable: bool) -> Result<File, String> {
        open_file_os(parent, OsStr::new(name), writable)
    }

    fn open_file_os(parent: &File, name: &OsStr, writable: bool) -> Result<File, String> {
        let name = cstring(name)?;
        let access = if writable {
            libc::O_RDWR
        } else {
            libc::O_RDONLY
        };
        // SAFETY: both descriptor and C string are valid for this call.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                access | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: `fd` is a newly-created descriptor with one owner.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub fn open_regular(root: &Path, relative: &Path) -> Result<OpenedRegularFile, String> {
        let name = relative
            .file_name()
            .ok_or_else(|| "相对路径缺少文件名。".to_string())?;
        let parent_path = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = open_parent(root, parent_path)?;
        let file = open_file_os(&parent, name, false)?;
        let metadata = file.metadata().map_err(display_error)?;
        if !metadata.is_file() {
            return Err("目标不是普通文件。".into());
        }
        Ok(OpenedRegularFile {
            file,
            identity: format!("{}:{}", metadata.dev(), metadata.ino()),
            size_bytes: metadata.len(),
        })
    }

    fn exchange(parent: &File, first: &CString, second: &CString) -> Result<(), String> {
        #[cfg(target_vendor = "apple")]
        let result = unsafe {
            // SAFETY: the directory descriptor and both C strings stay valid for
            // the duration of this atomic same-directory exchange.
            libc::renameatx_np(
                parent.as_raw_fd(),
                first.as_ptr(),
                parent.as_raw_fd(),
                second.as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            // SAFETY: equivalent Linux atomic exchange, scoped to the anchored
            // directory descriptor.
            libc::syscall(
                libc::SYS_renameat2,
                parent.as_raw_fd(),
                first.as_ptr(),
                parent.as_raw_fd(),
                second.as_ptr(),
                libc::RENAME_EXCHANGE,
            ) as libc::c_int
        };
        #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
        let result = {
            let _ = (parent, first, second);
            return Err("当前 Unix 平台不支持安全的原子文件交换。".into());
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error().to_string())
        }
    }

    fn read_from_handle(mut file: File, max_bytes: u64) -> Result<(Vec<u8>, FileStamp), String> {
        let metadata = file.metadata().map_err(display_error)?;
        if !metadata.is_file() {
            return Err("目标不是普通文件。".into());
        }
        if metadata.len() > max_bytes {
            return Err(format!("文件超过 {max_bytes} bytes 安全上限。"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        std::io::Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(display_error)?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!("文件超过 {max_bytes} bytes 安全上限。"));
        }
        let mtime_ms = metadata
            .modified()
            .map_err(display_error)?
            .duration_since(UNIX_EPOCH)
            .map_err(display_error)?
            .as_millis() as i64;
        let stamp = FileStamp {
            sha256: sha256(&bytes),
            mtime_ms,
            identity: format!("{}:{}", metadata.dev(), metadata.ino()),
            size_bytes: metadata.len(),
        };
        Ok((bytes, stamp))
    }

    pub fn read(
        root: &Path,
        relative_parent: &Path,
        name: &str,
        max_bytes: u64,
    ) -> Result<ReadResult, String> {
        let parent = open_parent(root, relative_parent)?;
        let writable = open_file(&parent, name, true).is_ok();
        let (bytes, stamp) = read_from_handle(open_file(&parent, name, false)?, max_bytes)?;
        Ok(ReadResult {
            path: root.join(relative_parent).join(name),
            bytes,
            stamp,
            writable,
        })
    }

    fn create_temp(parent: &File, mode: u32) -> Result<(String, File), String> {
        for _ in 0..8 {
            let name = format!(".codex-manager-{}.tmp", Uuid::new_v4());
            let c_name = cstring(OsStr::new(&name))?;
            // SAFETY: both descriptor and C string are valid for this call.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    c_name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW,
                    mode as libc::c_uint,
                )
            };
            if fd >= 0 {
                // SAFETY: `fd` is a newly-created descriptor with one owner.
                return Ok((name, unsafe { File::from_raw_fd(fd) }));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error.to_string());
            }
        }
        Err("无法分配安全临时文件名。".into())
    }

    fn unlink(parent: &File, name: &str) {
        if let Ok(name) = cstring(OsStr::new(name)) {
            // SAFETY: both descriptor and C string are valid for this call.
            unsafe {
                libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write<F, T>(
        root: &Path,
        relative_parent: &Path,
        name: &str,
        mode: WriteMode,
        expectation: Option<WriteExpectation<'_>>,
        bytes: &[u8],
        max_bytes: u64,
        before_commit: F,
    ) -> Result<(ReadResult, T), String>
    where
        F: FnOnce(&[u8], &[u8]) -> Result<T, String>,
    {
        if bytes.len() as u64 > max_bytes {
            return Err(format!("内容超过 {max_bytes} bytes 安全上限。"));
        }
        let parent = open_parent(root, relative_parent)?;
        let (before, original_mode) = match mode {
            WriteMode::Replace => {
                let file = open_file(&parent, name, false)?;
                let metadata = file.metadata().map_err(display_error)?;
                let (before, stamp) = read_from_handle(file, max_bytes)?;
                let expected = expectation
                    .as_ref()
                    .ok_or_else(|| "保存现有文件必须携带冲突检测 token。".to_string())?;
                if stamp.sha256 != expected.sha256 || stamp.mtime_ms != expected.mtime_ms {
                    return Err("文件已被外部修改；请重新加载后再保存。".into());
                }
                (before, metadata.mode() & 0o777)
            }
            WriteMode::Create => {
                if open_file(&parent, name, false).is_ok() {
                    return Err("AGENTS 文件已存在；请重新加载项目。".into());
                }
                (Vec::new(), 0o644)
            }
        };

        let (temp_name, mut temp) = create_temp(&parent, original_mode)?;
        let cleanup = || unlink(&parent, &temp_name);
        if let Err(error) = temp.write_all(bytes).and_then(|_| temp.sync_all()) {
            cleanup();
            return Err(error.to_string());
        }
        // `fchmod` neutralizes a restrictive/expansive process umask for replace
        // while retaining the original file mode.
        // SAFETY: the descriptor belongs to `temp` and remains open.
        if unsafe { libc::fchmod(temp.as_raw_fd(), original_mode as libc::mode_t) } != 0 {
            let error = io::Error::last_os_error().to_string();
            cleanup();
            return Err(error);
        }

        // Re-open through the anchored directory immediately before committing.
        // This closes the interval between the UI snapshot and the atomic rename.
        if mode == WriteMode::Replace {
            let (_, current) = read_from_handle(open_file(&parent, name, false)?, max_bytes)?;
            let expected = expectation.as_ref().expect("replace checked above");
            if current.sha256 != expected.sha256 || current.mtime_ms != expected.mtime_ms {
                cleanup();
                return Err("文件已被外部修改；请重新加载后再保存。".into());
            }
        }

        let token = match before_commit(&before, bytes) {
            Ok(token) => token,
            Err(error) => {
                cleanup();
                return Err(error);
            }
        };
        let c_temp = cstring(OsStr::new(&temp_name))?;
        let c_name = cstring(OsStr::new(name))?;
        let commit_result = match mode {
            WriteMode::Replace => match exchange(&parent, &c_temp, &c_name) {
                Ok(()) => 0,
                Err(error) => {
                    cleanup();
                    return Err(error);
                }
            },
            WriteMode::Create => {
                // A hard link exposes the fully-synced inode atomically and refuses
                // to overwrite a file created by a racing process.
                // SAFETY: all descriptors and C strings are valid.
                let linked = unsafe {
                    libc::linkat(
                        parent.as_raw_fd(),
                        c_temp.as_ptr(),
                        parent.as_raw_fd(),
                        c_name.as_ptr(),
                        0,
                    )
                };
                if linked == 0 {
                    unlink(&parent, &temp_name);
                }
                linked
            }
        };
        if commit_result != 0 {
            let error = io::Error::last_os_error().to_string();
            cleanup();
            return Err(error);
        }

        if mode == WriteMode::Replace {
            // After the atomic exchange, the temporary name refers to the exact
            // inode that was replaced. Validate that inode, not the pathname that
            // was checked before `before_commit` ran.
            let (_, swapped_out) =
                read_from_handle(open_file(&parent, &temp_name, false)?, max_bytes)?;
            let expected = expectation.as_ref().expect("replace checked above");
            if swapped_out.sha256 != expected.sha256 || swapped_out.mtime_ms != expected.mtime_ms {
                if let Err(error) = exchange(&parent, &c_temp, &c_name) {
                    return Err(format!(
                        "文件发生并发修改，且自动恢复失败：{error}；请保留 {temp_name} 并手工检查。"
                    ));
                }
                parent.sync_all().map_err(display_error)?;
                let restored = read_from_handle(open_file(&parent, name, false)?, max_bytes)?.1;
                let replacement =
                    read_from_handle(open_file(&parent, &temp_name, false)?, max_bytes)?.1;
                if restored.sha256 == swapped_out.sha256
                    && restored.mtime_ms == swapped_out.mtime_ms
                    && replacement.sha256 == sha256(bytes)
                {
                    cleanup();
                    return Err("文件已被外部修改；已恢复外部版本，请重新加载后再保存。".into());
                }
                return Err(format!(
                    "文件发生多方并发修改；已尽力恢复，但为避免数据丢失保留了 {temp_name}，请手工检查。"
                ));
            }
            cleanup();
        }
        parent.sync_all().map_err(display_error)?;
        let result = read(root, relative_parent, name, max_bytes)?;
        Ok((result, token))
    }
}

#[cfg(not(unix))]
mod portable {
    use super::*;
    use std::fs::OpenOptions;

    pub fn open_regular(root: &Path, relative: &Path) -> Result<OpenedRegularFile, String> {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(display_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("目标不是普通文件或为符号链接。".into());
        }
        let file = File::open(&path).map_err(display_error)?;
        let opened = file.metadata().map_err(display_error)?;
        if !opened.is_file() || opened.len() != metadata.len() {
            return Err("文件在打开期间已变化。".into());
        }
        let mtime_ms = opened
            .modified()
            .map_err(display_error)?
            .duration_since(UNIX_EPOCH)
            .map_err(display_error)?
            .as_millis();
        Ok(OpenedRegularFile {
            file,
            identity: format!("{}:{mtime_ms}", opened.len()),
            size_bytes: opened.len(),
        })
    }

    pub fn read(
        root: &Path,
        relative_parent: &Path,
        name: &str,
        max_bytes: u64,
    ) -> Result<ReadResult, String> {
        let path = root.join(relative_parent).join(name);
        let metadata = fs::symlink_metadata(&path).map_err(display_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
            return Err("目标不是受支持的普通文件，或超过大小上限。".into());
        }
        let bytes = fs::read(&path).map_err(display_error)?;
        let mtime_ms = metadata
            .modified()
            .map_err(display_error)?
            .duration_since(UNIX_EPOCH)
            .map_err(display_error)?
            .as_millis() as i64;
        Ok(ReadResult {
            path,
            stamp: FileStamp {
                sha256: sha256(&bytes),
                mtime_ms,
                identity: format!("{}:{mtime_ms}", metadata.len()),
                size_bytes: metadata.len(),
            },
            bytes,
            writable: OpenOptions::new()
                .write(true)
                .open(root.join(relative_parent))
                .is_ok(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write<F, T>(
        root: &Path,
        relative_parent: &Path,
        name: &str,
        mode: WriteMode,
        expectation: Option<WriteExpectation<'_>>,
        bytes: &[u8],
        max_bytes: u64,
        before_commit: F,
    ) -> Result<(ReadResult, T), String>
    where
        F: FnOnce(&[u8], &[u8]) -> Result<T, String>,
    {
        if bytes.len() as u64 > max_bytes {
            return Err(format!("内容超过 {max_bytes} bytes 安全上限。"));
        }
        let parent = root.join(relative_parent);
        let path = parent.join(name);
        let before = if mode == WriteMode::Replace {
            let current = read(root, relative_parent, name, max_bytes)?;
            let expected = expectation
                .as_ref()
                .ok_or_else(|| "保存现有文件必须携带冲突检测 token。".to_string())?;
            if current.stamp.sha256 != expected.sha256
                || current.stamp.mtime_ms != expected.mtime_ms
            {
                return Err("文件已被外部修改；请重新加载后再保存。".into());
            }
            current.bytes
        } else {
            if path.exists() {
                return Err("AGENTS 文件已存在；请重新加载项目。".into());
            }
            Vec::new()
        };
        let token = before_commit(&before, bytes)?;
        let mut temp = tempfile::NamedTempFile::new_in(&parent).map_err(display_error)?;
        temp.write_all(bytes).map_err(display_error)?;
        temp.as_file().sync_all().map_err(display_error)?;
        if mode == WriteMode::Create {
            temp.persist_noclobber(&path)
                .map_err(|error| error.error.to_string())?;
        } else {
            // Windows replacement semantics will be implemented and tested in
            // the dedicated Windows milestone. The second check still prevents
            // silent overwrites in the current portable fallback.
            let current = read(root, relative_parent, name, max_bytes)?;
            let expected = expectation.as_ref().expect("replace checked above");
            if current.stamp.sha256 != expected.sha256
                || current.stamp.mtime_ms != expected.mtime_ms
            {
                return Err("文件已被外部修改；请重新加载后再保存。".into());
            }
            temp.persist(&path)
                .map_err(|error| error.error.to_string())?;
        }
        Ok((read(root, relative_parent, name, max_bytes)?, token))
    }
}

pub fn open_regular_beneath(root: &Path, relative: &Path) -> Result<OpenedRegularFile, String> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err("相对路径包含不安全组件。".into());
    }
    #[cfg(unix)]
    {
        unix::open_regular(root, relative)
    }
    #[cfg(not(unix))]
    {
        portable::open_regular(root, relative)
    }
}

pub fn read_bounded_regular_beneath(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let mut opened = open_regular_beneath(root, relative)?;
    if opened.size_bytes > max_bytes {
        return Err(format!("文件超过 {max_bytes} bytes 安全上限。"));
    }
    let mut bytes = Vec::with_capacity(opened.size_bytes as usize);
    std::io::Read::by_ref(&mut opened.file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(display_error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("文件超过 {max_bytes} bytes 安全上限。"));
    }
    Ok(bytes)
}

pub fn read_authorized(
    root: &Path,
    relative_parent: &Path,
    name: &str,
    max_bytes: u64,
) -> Result<ReadResult, String> {
    #[cfg(unix)]
    {
        unix::read(root, relative_parent, name, max_bytes)
    }
    #[cfg(not(unix))]
    {
        portable::read(root, relative_parent, name, max_bytes)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn write_authorized<F, T>(
    root: &Path,
    relative_parent: &Path,
    name: &str,
    mode: WriteMode,
    expectation: Option<WriteExpectation<'_>>,
    bytes: &[u8],
    max_bytes: u64,
    before_commit: F,
) -> Result<(ReadResult, T), String>
where
    F: FnOnce(&[u8], &[u8]) -> Result<T, String>,
{
    #[cfg(unix)]
    {
        unix::write(
            root,
            relative_parent,
            name,
            mode,
            expectation,
            bytes,
            max_bytes,
            before_commit,
        )
    }
    #[cfg(not(unix))]
    {
        portable::write(
            root,
            relative_parent,
            name,
            mode,
            expectation,
            bytes,
            max_bytes,
            before_commit,
        )
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_newline_style() {
        assert_eq!(preserve_newlines("a\nb\n", "crlf"), b"a\r\nb\r\n");
        assert_eq!(preserve_newlines("a\r\nb\r\n", "lf"), b"a\nb\n");
    }

    #[test]
    fn creates_replaces_and_detects_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_authorized_root(directory.path()).unwrap();
        let names = vec!["AGENTS.md".to_string()];
        let target = root.join("AGENTS.md");
        let (checked_root, relative, name) =
            validate_relative_target(&root, &target, &names).unwrap();

        let (created, token) = write_authorized(
            &checked_root,
            &relative,
            &name,
            WriteMode::Create,
            None,
            b"one\n",
            32 * 1024,
            |before, after| {
                assert!(before.is_empty());
                assert_eq!(after, b"one\n");
                Ok("created")
            },
        )
        .unwrap();
        assert_eq!(token, "created");
        assert_eq!(created.bytes, b"one\n");

        let expectation = WriteExpectation {
            sha256: &created.stamp.sha256,
            mtime_ms: created.stamp.mtime_ms,
        };
        let (saved, _) = write_authorized(
            &checked_root,
            &relative,
            &name,
            WriteMode::Replace,
            Some(expectation),
            b"two\n",
            32 * 1024,
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(saved.bytes, b"two\n");

        let stale = WriteExpectation {
            sha256: &created.stamp.sha256,
            mtime_ms: created.stamp.mtime_ms,
        };
        assert!(
            write_authorized(
                &checked_root,
                &relative,
                &name,
                WriteMode::Replace,
                Some(stale),
                b"three\n",
                32 * 1024,
                |_, _| Ok(()),
            )
            .unwrap_err()
            .contains("外部修改")
        );
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    #[test]
    fn atomic_exchange_rejects_post_check_race_and_restores_external_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = canonical_authorized_root(directory.path()).unwrap();
        let target = root.join("AGENTS.md");
        fs::write(&target, b"original\n").unwrap();
        let initial = read_authorized(&root, Path::new(""), "AGENTS.md", 1024).unwrap();
        let expected_sha = initial.stamp.sha256.clone();
        let expected_mtime = initial.stamp.mtime_ms;

        let error = write_authorized(
            &root,
            Path::new(""),
            "AGENTS.md",
            WriteMode::Replace,
            Some(WriteExpectation {
                sha256: &expected_sha,
                mtime_ms: expected_mtime,
            }),
            b"manager\n",
            1024,
            |_, _| {
                fs::write(&target, b"external\n").map_err(display_error)?;
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("外部修改"));
        assert_eq!(fs::read(&target).unwrap(), b"external\n");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_target() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), directory.path().join("AGENTS.md")).unwrap();
        let root = canonical_authorized_root(directory.path()).unwrap();
        let error = read_authorized(&root, Path::new(""), "AGENTS.md", 32 * 1024).unwrap_err();
        assert!(!error.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_raced_symlink_in_authorized_root_ancestor() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("codex-home");
        let sessions = home.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(sessions.join("rollout.jsonl"), b"authorized\n").unwrap();
        let authorized_sessions = fs::canonicalize(&sessions).unwrap();

        let displaced = directory.path().join("codex-home-before-race");
        fs::rename(&home, &displaced).unwrap();
        let outside = directory.path().join("outside");
        fs::create_dir_all(outside.join("sessions")).unwrap();
        fs::write(outside.join("sessions/rollout.jsonl"), b"outside\n").unwrap();
        symlink(&outside, &home).unwrap();

        let error = open_regular_beneath(&authorized_sessions, Path::new("rollout.jsonl"))
            .err()
            .expect("a raced ancestor symlink must never be followed");
        assert!(!error.is_empty());
    }
}
