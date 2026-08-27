//! Codex-compatible AGENTS discovery and effective-chain resolution.

use crate::safe_fs;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub const DEFAULT_PROJECT_DOC_MAX_BYTES: u64 = 32 * 1024;
pub const HARD_AGENTS_READ_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_DISCOVERED_FILES: usize = 10_000;
const MAX_DISCOVERY_ENTRIES: usize = 50_000;
const MAX_DISCOVERY_TIME: Duration = Duration::from_secs(2);
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DocConfig {
    pub max_bytes: u64,
    pub fallback_filenames: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Default, Deserialize)]
struct RawDocConfig {
    project_doc_max_bytes: Option<u64>,
    project_doc_fallback_filenames: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentFile {
    pub path: String,
    pub relative_path: String,
    pub kind: String,
    pub precedence: i64,
    pub effective: bool,
    pub overridden: bool,
    pub sha256: String,
    pub mtime_ms: i64,
    pub size_bytes: u64,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub files: Vec<AgentFile>,
    pub effective_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub max_bytes: u64,
}

pub struct FileDiscovery {
    pub files: Vec<PathBuf>,
    pub warning: Option<String>,
}

pub fn load_doc_config(codex_home: Option<&Path>) -> DocConfig {
    let mut warnings = Vec::new();
    let raw = codex_home
        .and_then(|home| {
            let path = home.join("config.toml");
            if !path.exists() {
                return None;
            }
            match safe_fs::read_bounded_regular_beneath(
                home,
                Path::new("config.toml"),
                MAX_CONFIG_BYTES,
            ) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(content) => match toml::from_str::<RawDocConfig>(&content) {
                        Ok(config) => Some(config),
                        Err(error) => {
                            warnings.push(format!(
                                "无法解析 {} 的 AGENTS 配置，已使用默认值：{error}",
                                path.display()
                            ));
                            None
                        }
                    },
                    Err(_) => {
                        warnings.push(format!(
                            "无法读取 {} 的 AGENTS 配置，已使用默认值：文件不是有效 UTF-8",
                            path.display()
                        ));
                        None
                    }
                },
                Err(error) => {
                    warnings.push(format!(
                        "无法读取 {} 的 AGENTS 配置，已使用默认值：{error}",
                        path.display()
                    ));
                    None
                }
            }
        })
        .unwrap_or_default();

    let configured_max = raw
        .project_doc_max_bytes
        .unwrap_or(DEFAULT_PROJECT_DOC_MAX_BYTES)
        .max(1);
    let max_bytes = configured_max.min(HARD_AGENTS_READ_LIMIT);
    if configured_max > HARD_AGENTS_READ_LIMIT {
        warnings.push(format!(
            "project_doc_max_bytes={configured_max} 超过本应用 {} bytes 的读取安全上限，预览已截断到安全上限。",
            HARD_AGENTS_READ_LIMIT
        ));
    }

    let mut seen = HashSet::new();
    let fallback_filenames = raw
        .project_doc_fallback_filenames
        .unwrap_or_default()
        .into_iter()
        .filter_map(|name| {
            let safe = is_safe_fallback_name(&name);
            if !safe {
                warnings.push(format!("忽略不安全的 fallback 文件名：{name}"));
                return None;
            }
            if matches!(name.as_str(), "AGENTS.override.md" | "AGENTS.md") {
                return None;
            }
            seen.insert(name.clone()).then_some(name)
        })
        .collect();

    DocConfig {
        max_bytes,
        fallback_filenames,
        warnings,
    }
}

pub fn allowed_names(config: &DocConfig) -> Vec<String> {
    let mut names = vec!["AGENTS.override.md".into(), "AGENTS.md".into()];
    names.extend(config.fallback_filenames.iter().cloned());
    names
}

pub fn resolve_chain(
    project: &Path,
    selected_cwd: &Path,
    codex_home: Option<&Path>,
    authorized_roots: &[PathBuf],
) -> Result<Resolution, String> {
    let project = canonical_directory(project)?;
    let selected_cwd = canonical_directory(selected_cwd)?;
    if !selected_cwd.starts_with(&project) {
        return Err("selectedCwd 必须位于 projectPath 内。".into());
    }
    let codex_home = codex_home.map(canonical_directory).transpose()?;
    let config = load_doc_config(codex_home.as_deref());
    let mut warnings = config.warnings.clone();
    let names = allowed_names(&config);
    let mut files = Vec::new();
    let mut effective_paths = Vec::new();
    let mut combined_bytes = 0_u64;
    let mut stop_after_limit = false;
    let mut precedence = 0_i64;

    if let Some(home) = codex_home.as_deref() {
        let global_names = ["AGENTS.override.md", "AGENTS.md"];
        resolve_directory(
            home,
            Path::new(""),
            &global_names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
            "global",
            precedence,
            authorized_roots,
            &config,
            &mut combined_bytes,
            &mut stop_after_limit,
            &mut files,
            &mut effective_paths,
            &mut warnings,
        );
        precedence += 1;
    }

    let mut directories = vec![PathBuf::new()];
    let mut relative = PathBuf::new();
    for component in selected_cwd
        .strip_prefix(&project)
        .map_err(display_error)?
        .components()
    {
        if let Component::Normal(value) = component {
            relative.push(value);
            directories.push(relative.clone());
        }
    }
    for directory in directories {
        let kind = if directory.as_os_str().is_empty() {
            "project"
        } else {
            "nested"
        };
        resolve_directory(
            &project,
            &directory,
            &names,
            kind,
            precedence,
            authorized_roots,
            &config,
            &mut combined_bytes,
            &mut stop_after_limit,
            &mut files,
            &mut effective_paths,
            &mut warnings,
        );
        precedence += 1;
    }

    Ok(Resolution {
        files,
        effective_paths,
        warnings,
        max_bytes: config.max_bytes,
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_directory(
    root: &Path,
    relative_directory: &Path,
    names: &[String],
    default_kind: &str,
    precedence: i64,
    authorized_roots: &[PathBuf],
    config: &DocConfig,
    combined_bytes: &mut u64,
    stop_after_limit: &mut bool,
    files: &mut Vec<AgentFile>,
    effective_paths: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let mut first_non_empty: Option<usize> = None;
    for name in names {
        let path = root.join(relative_directory).join(name);
        if !path.exists() && fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let read = match safe_fs::read_authorized(
            root,
            relative_directory,
            name,
            HARD_AGENTS_READ_LIMIT,
        ) {
            Ok(read) => read,
            Err(error) => {
                warnings.push(format!("跳过 {}：{error}", path.display()));
                continue;
            }
        };
        let non_empty = read.bytes.iter().any(|byte| !byte.is_ascii_whitespace());
        let is_fallback = !matches!(name.as_str(), "AGENTS.override.md" | "AGENTS.md");
        let writable = read.writable
            && authorized_roots
                .iter()
                .any(|authorized| read.path.starts_with(authorized));
        let index = files.len();
        files.push(AgentFile {
            path: read.path.to_string_lossy().to_string(),
            relative_path: if default_kind == "global" {
                name.clone()
            } else {
                read.path
                    .strip_prefix(root)
                    .unwrap_or(&read.path)
                    .to_string_lossy()
                    .to_string()
            },
            kind: if is_fallback {
                "fallback".into()
            } else {
                default_kind.into()
            },
            precedence,
            effective: false,
            overridden: false,
            sha256: read.stamp.sha256,
            mtime_ms: read.stamp.mtime_ms,
            size_bytes: read.stamp.size_bytes,
            writable,
        });
        if !non_empty {
            warnings.push(format!("Codex 会跳过空指令文件：{}", path.display()));
        } else if first_non_empty.is_none() {
            first_non_empty = Some(index);
        }
    }

    let Some(selected) = first_non_empty else {
        return;
    };
    let selected_path = files[selected].path.clone();
    for file in files
        .iter_mut()
        .filter(|file| file.precedence == precedence)
    {
        if file.path != selected_path {
            file.overridden = true;
        }
    }
    if *stop_after_limit {
        warnings.push(format!(
            "已达到 project_doc_max_bytes，Codex 不会继续加入 {}。",
            selected_path
        ));
        return;
    }

    files[selected].effective = true;
    effective_paths.push(selected_path.clone());
    let next_total = combined_bytes.saturating_add(files[selected].size_bytes);
    if next_total >= config.max_bytes {
        if next_total > config.max_bytes {
            warnings.push(format!(
                "{} 会在合并到 {} bytes 时被截断；后续层级不会加入当前会话指令。",
                selected_path, config.max_bytes
            ));
        }
        *stop_after_limit = true;
    }
    *combined_bytes = next_total.min(config.max_bytes);
}

pub fn discover_all(project: &Path, config: &DocConfig) -> Result<FileDiscovery, String> {
    let project = canonical_directory(project)?;
    let names = allowed_names(config).into_iter().collect::<HashSet<_>>();
    let mut found = Vec::new();
    let started = Instant::now();
    let mut visited = 0_usize;
    let mut truncated = false;
    let walker = walkdir::WalkDir::new(&project)
        .follow_links(false)
        .max_depth(24)
        .into_iter()
        .filter_entry(should_visit_entry);
    for entry in walker {
        visited = visited.saturating_add(1);
        if visited > MAX_DISCOVERY_ENTRIES
            || found.len() >= MAX_DISCOVERED_FILES
            || started.elapsed() >= MAX_DISCOVERY_TIME
        {
            truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| names.contains(name))
        {
            found.push(entry.path().to_path_buf());
        }
    }
    found.sort();
    Ok(FileDiscovery {
        files: found,
        warning: truncated.then(|| {
            format!(
                "AGENTS 发现已达安全预算（遍历 {visited} 项）；当前结果为 partial，请缩小授权根目录。"
            )
        }),
    })
}

fn should_visit_entry(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() || entry.depth() == 0 {
        return true;
    }
    !matches!(
        entry.file_name().to_str(),
        Some(".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "venv")
    )
}

fn is_safe_fallback_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && Path::new(name).components().count() == 1
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', '\0'])
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(display_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("目录无效或为符号链接：{}", path.display()));
    }
    fs::canonicalize(path).map_err(display_error)
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_project_root_and_prefers_override_then_fallback() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let nested = project.join("src/deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(project.join("AGENTS.md"), "root").unwrap();
        fs::write(project.join("AGENTS.override.md"), "override").unwrap();
        fs::write(project.join("TEAM.md"), "fallback").unwrap();
        fs::write(project.join("config.toml"), "").unwrap();
        fs::write(project.join("src/AGENTS.md"), "   \n").unwrap();
        fs::write(project.join("src/TEAM.md"), "nested fallback").unwrap();
        let config_home = root.path().join("codex");
        fs::create_dir_all(&config_home).unwrap();
        fs::write(
            config_home.join("config.toml"),
            "project_doc_fallback_filenames = [\"TEAM.md\"]\n",
        )
        .unwrap();

        let resolved = resolve_chain(
            &project,
            &nested,
            Some(&config_home),
            std::slice::from_ref(&project),
        )
        .unwrap();
        assert!(
            resolved
                .effective_paths
                .iter()
                .any(|path| path.ends_with("project/AGENTS.override.md"))
        );
        assert!(
            resolved
                .effective_paths
                .iter()
                .any(|path| path.ends_with("project/src/TEAM.md"))
        );
        assert!(
            resolved
                .files
                .iter()
                .find(|file| file.path.ends_with("project/AGENTS.md"))
                .unwrap()
                .overridden
        );
        assert!(
            resolved
                .files
                .iter()
                .find(|file| file.path.ends_with("project/src/AGENTS.md"))
                .is_some_and(|file| !file.effective)
        );
    }

    #[test]
    fn global_uses_first_non_empty_file() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let codex_home = root.path().join("codex");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(codex_home.join("AGENTS.override.md"), "\n").unwrap();
        fs::write(codex_home.join("AGENTS.md"), "global").unwrap();
        let resolved = resolve_chain(
            &project,
            &project,
            Some(&codex_home),
            std::slice::from_ref(&project),
        )
        .unwrap();
        assert_eq!(resolved.effective_paths.len(), 1);
        assert!(resolved.effective_paths[0].ends_with("codex/AGENTS.md"));
    }

    #[test]
    fn rejects_path_fallback_names() {
        assert!(!is_safe_fallback_name("../outside.md"));
        assert!(!is_safe_fallback_name("nested/TEAM.md"));
        assert!(is_safe_fallback_name(".agents.md"));
    }
}
