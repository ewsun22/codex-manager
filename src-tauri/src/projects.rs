//! Cross-platform project discovery with bounded scans and explicit Git
//! worktree detection.

use crate::platform;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_SCAN_DEPTH: usize = 5;
const MAX_PROJECTS_PER_ROOT: usize = 2_000;
const MAX_ENTRIES_PER_ROOT: usize = 50_000;
const MAX_MARKERS_PER_ROOT: usize = 5_000;
const MAX_GIT_PROBES_PER_ROOT: usize = 128;
const MAX_DISCOVERY_TIME_PER_ROOT: Duration = Duration::from_secs(3);
const MAX_OBSERVED_CWDS: usize = 5_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredProject {
    pub canonical_path: PathBuf,
    pub name: String,
    pub source: String,
    pub is_git: bool,
    pub worktree: bool,
}

#[derive(Clone, Debug)]
pub struct GitInfo {
    pub root: PathBuf,
    pub is_linked_worktree: bool,
}

pub struct DiscoveryResult {
    pub projects: Vec<DiscoveredProject>,
    pub warnings: Vec<String>,
}

struct DiscoveryBudget {
    started: Instant,
    visited: usize,
    markers: usize,
    git_probes: usize,
    exhausted: bool,
}

impl DiscoveryBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            visited: 0,
            markers: 0,
            git_probes: 0,
            exhausted: false,
        }
    }

    fn visit(&mut self) -> bool {
        self.visited = self.visited.saturating_add(1);
        self.check(self.visited <= MAX_ENTRIES_PER_ROOT)
    }

    fn marker(&mut self) -> bool {
        self.markers = self.markers.saturating_add(1);
        self.check(self.markers <= MAX_MARKERS_PER_ROOT)
    }

    fn git_probe(&mut self) -> bool {
        self.git_probes = self.git_probes.saturating_add(1);
        self.check(self.git_probes <= MAX_GIT_PROBES_PER_ROOT)
    }

    fn check(&mut self, count_ok: bool) -> bool {
        let ok = count_ok && self.started.elapsed() < MAX_DISCOVERY_TIME_PER_ROOT;
        self.exhausted |= !ok;
        ok
    }
}

pub fn discover(observed_cwds: &[String], manual_roots: &[String]) -> DiscoveryResult {
    let mut projects = BTreeMap::<PathBuf, DiscoveredProject>::new();
    let mut warnings = Vec::new();
    let mut observed_budget = DiscoveryBudget::new();
    for cwd in observed_cwds.iter().take(MAX_OBSERVED_CWDS) {
        if !observed_budget.visit() || !observed_budget.git_probe() {
            break;
        }
        let path = Path::new(cwd);
        let Some(project) = project_from_observed(path) else {
            continue;
        };
        projects.insert(project.canonical_path.clone(), project);
    }
    if observed_cwds.len() > MAX_OBSERVED_CWDS || observed_budget.exhausted {
        warnings.push(format!(
            "已观测 cwd 发现达到安全预算（visited={} gitProbes={}），项目结果为 partial。",
            observed_budget.visited, observed_budget.git_probes
        ));
    }
    for root in manual_roots {
        let result = scan_manual_root(Path::new(root));
        if let Some(warning) = result.warning {
            warnings.push(warning);
        }
        for project in result.projects {
            projects
                .entry(project.canonical_path.clone())
                .and_modify(|existing| {
                    // An observed cwd is stronger evidence than a manual scan.
                    if existing.source != "observed" {
                        *existing = project.clone();
                    }
                })
                .or_insert(project);
        }
    }
    DiscoveryResult {
        projects: projects.into_values().collect(),
        warnings,
    }
}

pub fn git_info(path: &Path) -> Option<GitInfo> {
    let git = platform::find_executable("git")?;
    let args = vec![
        OsString::from("-C"),
        path.as_os_str().to_os_string(),
        OsString::from("rev-parse"),
        OsString::from("--path-format=absolute"),
        OsString::from("--show-toplevel"),
        OsString::from("--git-dir"),
        OsString::from("--git-common-dir"),
    ];
    let output = platform::run_capture(&git, args, Duration::from_secs(1)).ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
    let root = canonical_directory(Path::new(lines.next()?)).ok()?;
    let git_dir = absolute_or_join(&root, Path::new(lines.next()?));
    let common_dir = absolute_or_join(&root, Path::new(lines.next()?));
    let normalized_git = fs::canonicalize(&git_dir).unwrap_or(git_dir.clone());
    let normalized_common = fs::canonicalize(&common_dir).unwrap_or(common_dir.clone());
    Some(GitInfo {
        root,
        is_linked_worktree: normalized_git != normalized_common,
    })
}

fn project_from_observed(path: &Path) -> Option<DiscoveredProject> {
    let canonical = canonical_directory(path).ok()?;
    if let Some(git) = git_info(&canonical) {
        return Some(project_row(
            git.root,
            "observed",
            true,
            git.is_linked_worktree,
        ));
    }
    Some(project_row(canonical, "observed", false, false))
}

struct ManualDiscovery {
    projects: Vec<DiscoveredProject>,
    warning: Option<String>,
}

fn scan_manual_root(root: &Path) -> ManualDiscovery {
    let Ok(root) = canonical_directory(root) else {
        return ManualDiscovery {
            projects: Vec::new(),
            warning: None,
        };
    };
    let mut found = Vec::new();
    let mut budget = DiscoveryBudget::new();
    let mut seen_git_markers = BTreeSet::new();
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .max_depth(MAX_SCAN_DEPTH)
        .into_iter()
        .filter_entry(should_visit_entry);
    for entry in walker {
        if !budget.visit() {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if found.len() >= MAX_PROJECTS_PER_ROOT {
            budget.exhausted = true;
            break;
        }
        if !entry.file_type().is_dir() || !looks_like_project(entry.path()) {
            continue;
        }
        if !budget.marker() {
            break;
        }
        let path = entry.path();
        let git_marker = nearest_git_marker(path, &root);
        let project = if let Some(git_marker) = git_marker {
            let canonical_marker = fs::canonicalize(&git_marker).unwrap_or(git_marker);
            if !seen_git_markers.insert(canonical_marker.clone()) {
                continue;
            }
            if !budget.git_probe() {
                break;
            }
            let Some(git) = git_info(&canonical_marker) else {
                continue;
            };
            project_row(git.root, "manual", true, git.is_linked_worktree)
        } else {
            let Ok(canonical) = canonical_directory(path) else {
                continue;
            };
            project_row(canonical, "manual", false, false)
        };
        found.push(project);
    }
    if found.is_empty() && !budget.exhausted && looks_like_project(&root) {
        if let Some(project) = project_from_observed(&root) {
            found.push(DiscoveredProject {
                source: "manual".into(),
                ..project
            });
        }
    }
    found.sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    found.dedup_by(|left, right| left.canonical_path == right.canonical_path);
    ManualDiscovery {
        projects: found,
        warning: budget.exhausted.then(|| {
            format!(
                "项目发现已达安全预算（root={} visited={} markers={} gitProbes={}）；结果为 partial。",
                root.display(), budget.visited, budget.markers, budget.git_probes
            )
        }),
    }
}

fn nearest_git_marker(path: &Path, root: &Path) -> Option<PathBuf> {
    path.ancestors()
        .take_while(|candidate| candidate.starts_with(root))
        .find(|candidate| {
            fs::symlink_metadata(candidate.join(".git")).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file())
            })
        })
        .map(Path::to_path_buf)
}

fn project_row(path: PathBuf, source: &str, is_git: bool, worktree: bool) -> DiscoveredProject {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("项目")
        .to_string();
    DiscoveredProject {
        canonical_path: path,
        name,
        source: source.into(),
        is_git,
        worktree,
    }
}

fn looks_like_project(path: &Path) -> bool {
    [
        ".git",
        "AGENTS.md",
        "AGENTS.override.md",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "Package.swift",
    ]
    .into_iter()
    .any(|marker| path.join(marker).exists())
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

fn absolute_or_join(root: &Path, value: &Path) -> PathBuf {
    if value.is_absolute() {
        value.to_path_buf()
    } else {
        root.join(value)
    }
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
    fn manual_root_finds_bounded_project_markers() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("group/second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("package.json"), "{}").unwrap();
        fs::write(second.join("AGENTS.md"), "instructions").unwrap();
        let found = scan_manual_root(directory.path()).projects;
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|project| project.name == "first"));
        assert!(found.iter().any(|project| project.name == "second"));
    }

    #[test]
    fn skips_dependency_trees() {
        let directory = tempfile::tempdir().unwrap();
        let dependency = directory.path().join("node_modules/not-a-project");
        fs::create_dir_all(&dependency).unwrap();
        fs::write(dependency.join("package.json"), "{}").unwrap();
        assert!(scan_manual_root(directory.path()).projects.is_empty());
    }
}
