//! Small platform boundary for executable discovery, stable file identity and
//! bounded subprocesses. Core normalization/storage remains platform-neutral.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};
use wait_timeout::ChildExt;

#[cfg(test)]
fn stable_file_identity(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(display_error)?;
    if !metadata.is_file() {
        return Err("采集源不是普通文件。".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let canonical = fs::canonicalize(path).map_err(display_error)?;
        Ok(format!("portable:{}", canonical.to_string_lossy()))
    }
}

pub fn codex_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(explicit) = std::env::var_os("CODEX_MANAGER_CODEX_BIN") {
        let path = PathBuf::from(explicit);
        if is_executable(&path) {
            candidates.push(path);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let desktop = PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex");
        if is_executable(&desktop) {
            candidates.push(desktop);
        }
    }
    if let Some(path) = find_executable("codex") {
        candidates.push(path);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

pub fn run_capture<I, S>(program: &Path, args: I, timeout: Duration) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(display_error)?;
    match child.wait_timeout(timeout).map_err(display_error)? {
        Some(_) => child.wait_with_output().map_err(display_error),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("子进程超过 {} 秒超时。", timeout.as_secs()))
        }
    }
}

pub fn run_quiet_status<I, S>(program: &Path, args: I, timeout: Duration) -> Result<bool, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(display_error)?;
    match child.wait_timeout(timeout).map_err(display_error)? {
        Some(status) => Ok(status.success()),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!("子进程超过 {} 秒超时。", timeout.as_secs()))
        }
    }
}

pub fn find_executable(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let names = {
        let mut values = vec![name.to_string()];
        if Path::new(name).extension().is_none() {
            let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
            values.extend(
                extensions
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(|extension| format!("{name}{extension}")),
            );
        }
        values
    };
    #[cfg(not(windows))]
    let names = vec![name.to_string()];
    for directory in std::env::split_paths(&paths) {
        for candidate_name in &names {
            let candidate = directory.join(candidate_name);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_identity_does_not_change_when_file_is_appended() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"one").unwrap();
        let first = stable_file_identity(file.path()).unwrap();
        file.write_all(b"two").unwrap();
        file.flush().unwrap();
        let second = stable_file_identity(file.path()).unwrap();
        assert_eq!(first, second);
    }
}
