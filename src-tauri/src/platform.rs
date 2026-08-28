//! Small platform boundary for executable discovery, stable file identity and
//! bounded subprocesses. Core normalization/storage remains platform-neutral.

use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
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

pub struct TrustedCodexAuthExecutable {
    path: PathBuf,
    _directory: tempfile::TempDir,
}

impl TrustedCodexAuthExecutable {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn trusted_codex_auth_path(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        is_openai_signed_macos_binary(path)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

/// Stages one executable that is safe to cross the OAuth credential boundary.
///
/// Capability probing may inspect a developer-provided CLI, but account access
/// is deliberately stricter. The macOS beta copies only the fixed ChatGPT
/// bundle binary into a new private directory, then verifies the exact staged
/// bytes against OpenAI's Developer ID before executing them. PATH shims,
/// environment overrides and a verify-then-swap race are excluded.
pub fn trusted_codex_auth_candidate() -> Option<TrustedCodexAuthExecutable> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;

        let source = fs::canonicalize(PathBuf::from(
            "/Applications/ChatGPT.app/Contents/Resources/codex",
        ))
        .ok()?;
        if !is_executable(&source) {
            return None;
        }
        let directory = tempfile::Builder::new()
            .prefix("codex-manager-auth-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .ok()?;
        let staged = directory.path().join("codex");
        fs::copy(source, &staged).ok()?;
        let mut permissions = fs::metadata(&staged).ok()?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&staged, permissions).ok()?;
        if is_openai_signed_macos_binary(&staged) {
            return Some(TrustedCodexAuthExecutable {
                path: staged,
                _directory: directory,
            });
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn is_openai_signed_macos_binary(path: &Path) -> bool {
    let arguments = vec![
        std::ffi::OsString::from("--verify"),
        std::ffi::OsString::from("--strict"),
        std::ffi::OsString::from("--test-requirement"),
        std::ffi::OsString::from(
            "=identifier \"codex\" and anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\"",
        ),
        path.as_os_str().to_os_string(),
    ];
    run_quiet_status(
        Path::new("/usr/bin/codesign"),
        arguments,
        Duration::from_secs(5),
    )
    .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn is_openai_signed_macos_process(pid: u32) -> bool {
    let arguments = vec![
        std::ffi::OsString::from("--verify"),
        std::ffi::OsString::from("--strict"),
        std::ffi::OsString::from("--test-requirement"),
        std::ffi::OsString::from(
            "=identifier \"codex\" and anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\"",
        ),
        std::ffi::OsString::from(format!("+{pid}")),
    ];
    run_quiet_status(
        Path::new("/usr/bin/codesign"),
        arguments,
        Duration::from_secs(5),
    )
    .unwrap_or(false)
}

fn verify_spawned_auth_process(child: &mut Child) -> Result<(), String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        if is_openai_signed_macos_process(child.id()) {
            return Ok(());
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("启动后的 Codex 进程未通过 OpenAI 动态签名验证。".into())
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        let _ = child;
        Ok(())
    }
}

fn sanitized_auth_environment(codex_home: Option<&Path>) -> Vec<(OsString, OsString)> {
    sanitized_auth_environment_from(std::env::vars_os(), codex_home)
}

fn sanitized_auth_environment_from<I>(
    source: I,
    codex_home: Option<&Path>,
) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    const ALLOWED: [&str; 8] = [
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "__CF_USER_TEXT_ENCODING",
    ];
    let mut environment = source
        .into_iter()
        .filter(|(key, _)| key.to_str().is_some_and(|key| ALLOWED.contains(&key)))
        .collect::<Vec<_>>();
    #[cfg(target_os = "macos")]
    environment.push((
        OsString::from("PATH"),
        OsString::from("/usr/bin:/bin:/usr/sbin:/sbin"),
    ));
    #[cfg(not(target_os = "macos"))]
    if let Some(path) = std::env::var_os("PATH") {
        environment.push((OsString::from("PATH"), path));
    }
    let effective_codex_home = codex_home
        .map(Path::as_os_str)
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("CODEX_HOME"));
    if let Some(codex_home) = effective_codex_home {
        environment.push((OsString::from("CODEX_HOME"), codex_home));
    }
    environment
}

enum AppServerHandle {
    Child(Child),
    #[cfg(target_os = "macos")]
    SuspendedPid(libc::pid_t),
}

pub struct AppServerProcess {
    handle: AppServerHandle,
    stdin: Option<Box<dyn Write + Send>>,
    stdout: Option<Box<dyn Read + Send>>,
    suspended: bool,
    stopped: bool,
}

impl AppServerProcess {
    pub fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        self.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout.take()
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !self.suspended {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if let AppServerHandle::SuspendedPid(pid) = self.handle {
            if unsafe { libc::kill(pid, libc::SIGCONT) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        self.suspended = false;
        Ok(())
    }

    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        match &mut self.handle {
            AppServerHandle::Child(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(target_os = "macos")]
            AppServerHandle::SuspendedPid(pid) => unsafe {
                let _ = libc::kill(*pid, libc::SIGKILL);
                let mut status = 0;
                let _ = libc::waitpid(*pid, &mut status, 0);
            },
        }
        self.stopped = true;
    }
}

impl Drop for AppServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "macos")]
fn spawn_suspended_signed_app_server(
    program: &Path,
    codex_home: Option<&Path>,
) -> Result<AppServerProcess, String> {
    use std::{
        ffi::CString,
        fs::File,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::ffi::OsStrExt,
        },
        ptr,
    };

    fn pipe_cloexec() -> Result<(OwnedFd, OwnedFd), String> {
        let mut descriptors = [0; 2];
        if unsafe { libc::pipe(descriptors.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        for descriptor in descriptors {
            if unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
                unsafe {
                    libc::close(descriptors[0]);
                    libc::close(descriptors[1]);
                }
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(unsafe {
            (
                OwnedFd::from_raw_fd(descriptors[0]),
                OwnedFd::from_raw_fd(descriptors[1]),
            )
        })
    }

    let program_c = CString::new(program.as_os_str().as_bytes())
        .map_err(|_| "Codex 可执行文件路径包含 NUL。".to_string())?;
    let app_server_c = CString::new("app-server").expect("static command has no NUL");
    let mut argv = vec![
        program_c.as_ptr().cast_mut(),
        app_server_c.as_ptr().cast_mut(),
        ptr::null_mut(),
    ];

    let mut environment = Vec::new();
    for (key, value) in sanitized_auth_environment(codex_home) {
        let mut entry = key.as_os_str().as_bytes().to_vec();
        entry.push(b'=');
        entry.extend_from_slice(value.as_os_str().as_bytes());
        environment
            .push(CString::new(entry).map_err(|_| "进程环境包含 NUL，无法安全启动 Codex。")?);
    }
    let mut envp = environment
        .iter()
        .map(|entry| entry.as_ptr().cast_mut())
        .collect::<Vec<_>>();
    envp.push(ptr::null_mut());

    let (stdin_read, stdin_write) = pipe_cloexec()?;
    let (stdout_read, stdout_write) = pipe_cloexec()?;
    let dev_null = CString::new("/dev/null").expect("static path has no NUL");
    let mut actions: libc::posix_spawn_file_actions_t = ptr::null_mut();
    let actions_init = unsafe { libc::posix_spawn_file_actions_init(&mut actions) };
    if actions_init != 0 {
        return Err(std::io::Error::from_raw_os_error(actions_init).to_string());
    }
    let actions_result = (|| -> Result<(), String> {
        for (from, to) in [
            (stdin_read.as_raw_fd(), libc::STDIN_FILENO),
            (stdout_write.as_raw_fd(), libc::STDOUT_FILENO),
        ] {
            let result = unsafe { libc::posix_spawn_file_actions_adddup2(&mut actions, from, to) };
            if result != 0 {
                return Err(std::io::Error::from_raw_os_error(result).to_string());
            }
        }
        for descriptor in [stdin_write.as_raw_fd(), stdout_read.as_raw_fd()] {
            let result =
                unsafe { libc::posix_spawn_file_actions_addclose(&mut actions, descriptor) };
            if result != 0 {
                return Err(std::io::Error::from_raw_os_error(result).to_string());
            }
        }
        let result = unsafe {
            libc::posix_spawn_file_actions_addopen(
                &mut actions,
                libc::STDERR_FILENO,
                dev_null.as_ptr(),
                libc::O_WRONLY,
                0,
            )
        };
        if result != 0 {
            return Err(std::io::Error::from_raw_os_error(result).to_string());
        }
        Ok(())
    })();
    if let Err(error) = actions_result {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut actions);
        }
        return Err(error);
    }

    let mut attributes: libc::posix_spawnattr_t = ptr::null_mut();
    let attributes_init = unsafe { libc::posix_spawnattr_init(&mut attributes) };
    if attributes_init != 0 {
        unsafe {
            libc::posix_spawn_file_actions_destroy(&mut actions);
        }
        return Err(std::io::Error::from_raw_os_error(attributes_init).to_string());
    }
    let flags =
        (libc::POSIX_SPAWN_START_SUSPENDED | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as libc::c_short;
    let flags_result = unsafe { libc::posix_spawnattr_setflags(&mut attributes, flags) };
    if flags_result != 0 {
        unsafe {
            libc::posix_spawnattr_destroy(&mut attributes);
            libc::posix_spawn_file_actions_destroy(&mut actions);
        }
        return Err(std::io::Error::from_raw_os_error(flags_result).to_string());
    }

    let mut pid = 0;
    let spawn_result = unsafe {
        libc::posix_spawn(
            &mut pid,
            program_c.as_ptr(),
            &actions,
            &attributes,
            argv.as_mut_ptr(),
            envp.as_mut_ptr(),
        )
    };
    unsafe {
        libc::posix_spawnattr_destroy(&mut attributes);
        libc::posix_spawn_file_actions_destroy(&mut actions);
    }
    if spawn_result != 0 {
        return Err(std::io::Error::from_raw_os_error(spawn_result).to_string());
    }
    if !is_openai_signed_macos_process(pid as u32) {
        unsafe {
            let _ = libc::kill(pid, libc::SIGKILL);
            let mut status = 0;
            let _ = libc::waitpid(pid, &mut status, 0);
        }
        return Err("暂停启动的 Codex App Server 未通过 OpenAI 动态签名验证。".into());
    }

    drop(stdin_read);
    drop(stdout_write);
    let stdin_file = File::from(stdin_write);
    let stdout_file = File::from(stdout_read);
    Ok(AppServerProcess {
        handle: AppServerHandle::SuspendedPid(pid),
        stdin: Some(Box::new(stdin_file)),
        stdout: Some(Box::new(stdout_file)),
        suspended: true,
        stopped: false,
    })
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

pub fn spawn_app_server_with_home(
    program: &Path,
    codex_home: Option<&Path>,
) -> Result<AppServerProcess, String> {
    #[cfg(target_os = "macos")]
    if !cfg!(test) {
        return spawn_suspended_signed_app_server(program, codex_home);
    }

    let mut command = Command::new(program);
    command
        .arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .envs(sanitized_auth_environment(codex_home));
    configure_background_command(&mut command);
    let mut child = command.spawn().map_err(display_error)?;
    verify_spawned_auth_process(&mut child)?;
    let stdin = child
        .stdin
        .take()
        .map(|stdin| Box::new(stdin) as Box<dyn Write + Send>);
    let stdout = child
        .stdout
        .take()
        .map(|stdout| Box::new(stdout) as Box<dyn Read + Send>);
    Ok(AppServerProcess {
        handle: AppServerHandle::Child(child),
        stdin,
        stdout,
        suspended: false,
        stopped: false,
    })
}

pub fn spawn_codex_login(program: &Path) -> Result<Child, String> {
    let mut command = Command::new(program);
    command
        .arg("login")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_clear()
        .envs(sanitized_auth_environment(None));
    configure_background_command(&mut command);
    let mut child = command.spawn().map_err(display_error)?;
    verify_spawned_auth_process(&mut child)?;
    Ok(child)
}

#[cfg(windows)]
fn configure_background_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_background_command(_: &mut Command) {}

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

    #[cfg(target_os = "macos")]
    use std::os::unix::fs::PermissionsExt;

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

    #[test]
    fn oauth_process_environment_is_allowlisted() {
        let source = vec![
            (OsString::from("HOME"), OsString::from("/safe-home")),
            (
                OsString::from("CODEX_REFRESH_TOKEN_URL_OVERRIDE"),
                OsString::from("https://attacker.invalid"),
            ),
            (
                OsString::from("CODEX_REVOKE_TOKEN_URL_OVERRIDE"),
                OsString::from("https://attacker.invalid"),
            ),
            (
                OsString::from("CODEX_APP_SERVER_LOGIN_CLIENT_ID"),
                OsString::from("attacker"),
            ),
            (
                OsString::from("HTTPS_PROXY"),
                OsString::from("https://attacker.invalid"),
            ),
            (
                OsString::from("SSL_CERT_FILE"),
                OsString::from("/tmp/attacker.pem"),
            ),
            (
                OsString::from("OPENAI_API_KEY"),
                OsString::from("never-inherit"),
            ),
            (OsString::from("CODEX_HOME"), OsString::from("/wrong")),
        ];
        let isolated = Path::new("/isolated-codex-home");
        let environment = sanitized_auth_environment_from(source, Some(isolated))
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(
            environment.get(OsStr::new("HOME")),
            Some(&OsString::from("/safe-home"))
        );
        assert_eq!(
            environment.get(OsStr::new("CODEX_HOME")),
            Some(&isolated.as_os_str().to_os_string())
        );
        for forbidden in [
            "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
            "CODEX_REVOKE_TOKEN_URL_OVERRIDE",
            "CODEX_APP_SERVER_LOGIN_CLIENT_ID",
            "HTTPS_PROXY",
            "SSL_CERT_FILE",
            "OPENAI_API_KEY",
        ] {
            assert!(!environment.contains_key(OsStr::new(forbidden)));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unsigned_executable_is_not_trusted_for_oauth() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = file.as_file().metadata().unwrap().permissions();
        permissions.set_mode(0o700);
        file.as_file().set_permissions(permissions).unwrap();

        assert!(!is_openai_signed_macos_binary(file.path()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an installed ChatGPT bundle and stages its large Codex binary"]
    fn installed_openai_codex_is_staged_then_verified() {
        use std::io::{BufRead, BufReader};
        let source = Path::new("/Applications/ChatGPT.app/Contents/Resources/codex");
        assert!(source.is_file());

        let executable = trusted_codex_auth_candidate().expect("signed Codex should be trusted");

        assert_ne!(executable.path(), source);
        assert!(executable.path().starts_with(std::env::temp_dir()));
        assert_eq!(
            fs::metadata(executable.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(is_openai_signed_macos_binary(executable.path()));
        assert!(
            run_quiet_status(executable.path(), ["--version"], Duration::from_secs(5)).unwrap()
        );
        let mut app_server = Command::new(executable.path())
            .arg("app-server")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(is_openai_signed_macos_process(app_server.id()));
        let _ = app_server.kill();
        let _ = app_server.wait();

        let isolated_home = tempfile::tempdir().unwrap();
        let mut suspended =
            spawn_suspended_signed_app_server(executable.path(), Some(isolated_home.path()))
                .unwrap();
        assert!(suspended.suspended);
        let mut input = suspended.take_stdin().unwrap();
        let output = suspended.take_stdout().unwrap();
        suspended.resume().unwrap();
        assert!(!suspended.suspended);
        input
            .write_all(
                br#"{"method":"initialize","id":0,"params":{"clientInfo":{"name":"codex_manager_test","title":"Codex Manager Test","version":"0"}}}
{"method":"initialized","params":{}}
{"method":"config/read","id":1,"params":{"cwd":null,"includeLayers":false}}
"#,
            )
            .unwrap();
        input.flush().unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            for _ in 0..16 {
                let mut line = String::new();
                if output.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if serde_json::from_str::<serde_json::Value>(&line)
                    .ok()
                    .and_then(|value| value.get("id").and_then(serde_json::Value::as_i64))
                    == Some(1)
                {
                    let _ = sender.send(true);
                    return;
                }
            }
            let _ = sender.send(false);
        });
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or(false)
        );
        suspended.stop();
        reader.join().unwrap();
    }
}
