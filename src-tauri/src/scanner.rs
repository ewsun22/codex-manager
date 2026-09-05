//! Bounded rollout scheduling. Events prioritize concrete files; reconciliation
//! retains its directory cursor until the complete configured tree is covered.

use std::{
    collections::{HashSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_PENDING_PATHS: usize = 1_024;
pub const MAX_DISCOVERY_ENTRIES: usize = 4_096;
pub const MAX_DISCOVERY_DURATION: Duration = Duration::from_millis(100);
pub const MAX_FILES_PER_ROUND: usize = 4_096;
pub const CONTINUATION_INTERVAL: Duration = Duration::from_millis(250);
const MAX_DEPTH: usize = 16;

#[derive(Default)]
struct PathQueue {
    paths: VecDeque<PathBuf>,
    present: HashSet<PathBuf>,
}

impl PathQueue {
    fn push(&mut self, path: PathBuf) -> bool {
        if self.present.contains(&path) {
            return true;
        }
        if self.paths.len() >= MAX_PENDING_PATHS {
            return false;
        }
        self.present.insert(path.clone());
        self.paths.push_back(path);
        true
    }

    fn pop(&mut self) -> Option<PathBuf> {
        let path = self.paths.pop_front()?;
        self.present.remove(&path);
        Some(path)
    }

    fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// Kept separately from the scheduler so a notification callback never waits
/// for filesystem discovery, parsing or a SQLite transaction.
#[derive(Default)]
pub struct ChangeInbox {
    files: PathQueue,
    reconcile: bool,
}

#[derive(Default)]
pub struct Changes {
    files: Vec<PathBuf>,
    reconcile: bool,
}

impl ChangeInbox {
    pub fn record_file(&mut self, path: PathBuf) {
        if !self.files.push(path) {
            self.reconcile = true;
        }
    }

    pub fn request_reconcile(&mut self) {
        self.reconcile = true;
    }

    pub fn take(&mut self) -> Changes {
        let mut files = Vec::with_capacity(self.files.paths.len());
        while let Some(path) = self.files.pop() {
            files.push(path);
        }
        Changes {
            files,
            reconcile: std::mem::take(&mut self.reconcile),
        }
    }
}

pub struct DiscoveryBudget {
    elapsed: Duration,
    max_entries: usize,
    max_duration: Duration,
    pub visited: usize,
}

impl DiscoveryBudget {
    pub fn new(max_entries: usize, max_duration: Duration) -> Self {
        Self {
            elapsed: Duration::ZERO,
            max_entries,
            max_duration,
            visited: 0,
        }
    }

    fn exhausted(&self, current_slice: Duration) -> bool {
        self.visited >= self.max_entries || self.elapsed + current_slice >= self.max_duration
    }
}

struct DiscoveryCursor {
    roots: VecDeque<PathBuf>,
    walker: Option<walkdir::IntoIter>,
}

impl DiscoveryCursor {
    fn new(roots: &[PathBuf]) -> Self {
        Self {
            roots: roots.iter().cloned().collect(),
            walker: None,
        }
    }

    fn next_file(&mut self, budget: &mut DiscoveryBudget) -> Option<PathBuf> {
        let started = Instant::now();
        let result = self.next_file_slice(budget, started);
        budget.elapsed += started.elapsed();
        result
    }

    fn next_file_slice(
        &mut self,
        budget: &mut DiscoveryBudget,
        started: Instant,
    ) -> Option<PathBuf> {
        while !budget.exhausted(started.elapsed()) {
            if self.walker.is_none() {
                let root = self.roots.pop_front()?;
                budget.visited += 1;
                if !fs::symlink_metadata(&root)
                    .is_ok_and(|entry| entry.is_dir() && !entry.file_type().is_symlink())
                {
                    continue;
                }
                // No eager directory sorting or all-file candidate vector.
                // The depth also bounds open directory cursors (at most 17).
                self.walker = Some(
                    walkdir::WalkDir::new(root)
                        .follow_links(false)
                        .max_depth(MAX_DEPTH)
                        .max_open(MAX_DEPTH + 1)
                        .into_iter(),
                );
            }
            if budget.exhausted(started.elapsed()) {
                break;
            }
            let walker = self.walker.as_mut().expect("walker initialized");
            match walker.next() {
                Some(Ok(entry)) => {
                    budget.visited += 1;
                    if entry.depth() > 0
                        && entry.file_type().is_dir()
                        && entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| name.starts_with('.'))
                    {
                        walker.skip_current_dir();
                    } else if entry.file_type().is_file()
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "jsonl")
                    {
                        return Some(entry.into_path());
                    }
                }
                Some(Err(_)) => {
                    budget.visited += 1;
                }
                None => {
                    self.walker = None;
                }
            }
        }
        None
    }

    fn finished(&self) -> bool {
        self.walker.is_none() && self.roots.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkKind {
    Changed,
    Continuation,
    Reconciliation,
}

pub struct WorkItem {
    pub path: PathBuf,
    pub kind: WorkKind,
}

#[derive(Default)]
pub struct ScanScheduler {
    roots: Vec<PathBuf>,
    changed: PathQueue,
    continuations: PathQueue,
    cursor: Option<DiscoveryCursor>,
    reconcile_requested: bool,
    work_ordinal: usize,
    background_ordinal: usize,
}

impl ScanScheduler {
    pub fn configure(&mut self, homes: &[String]) {
        let roots = ["sessions", "archived_sessions"]
            .into_iter()
            .flat_map(|directory| {
                homes
                    .iter()
                    .map(move |home| Path::new(home).join(directory))
            })
            .collect::<Vec<_>>();
        if roots != self.roots {
            self.roots = roots;
            self.changed = PathQueue::default();
            self.continuations = PathQueue::default();
            self.cursor = None;
            self.reconcile_requested = true;
        }
    }

    pub fn request_reconcile(&mut self) {
        // An already-running cursor is never reset by another wake. If a
        // change was dropped after its directory was visited, one subsequent
        // cycle is enough to reconcile that path too.
        self.reconcile_requested = true;
    }

    pub fn accept(&mut self, changes: Changes) {
        if changes.reconcile {
            self.request_reconcile();
        }
        for path in changes.files {
            if self.allowed(&path) && !self.changed.push(path) {
                self.request_reconcile();
            }
        }
    }

    pub fn begin_round(&mut self) {
        if self.cursor.is_none() && self.reconcile_requested {
            self.cursor = Some(DiscoveryCursor::new(&self.roots));
            self.reconcile_requested = false;
        }
    }

    fn allowed(&self, path: &Path) -> bool {
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            return false;
        }
        self.roots.iter().any(|root| {
            let Ok(relative) = path.strip_prefix(root) else {
                return false;
            };
            let parts = relative.components().collect::<Vec<_>>();
            !parts.is_empty()
                && parts.len() <= MAX_DEPTH
                && parts
                    .iter()
                    .all(|part| matches!(part, Component::Normal(_)))
                && parts[..parts.len() - 1].iter().all(|part| {
                    !part
                        .as_os_str()
                        .to_str()
                        .is_some_and(|name| name.starts_with('.'))
                })
        })
    }

    pub fn continue_file(&mut self, path: PathBuf) {
        if self.allowed(&path) && !self.continuations.push(path) {
            self.request_reconcile();
        }
    }

    fn history(&mut self, budget: &mut DiscoveryBudget) -> Option<WorkItem> {
        let cursor = self.cursor.as_mut()?;
        let file = cursor.next_file(budget);
        if cursor.finished() {
            self.cursor = None;
        }
        file.map(|path| WorkItem {
            path,
            kind: WorkKind::Reconciliation,
        })
    }

    fn background(&mut self, budget: &mut DiscoveryBudget) -> Option<WorkItem> {
        let prefer_continuation = self.background_ordinal % 2 == 0;
        self.background_ordinal = self.background_ordinal.wrapping_add(1);
        let continuation = prefer_continuation
            .then(|| self.continuations.pop())
            .flatten();
        if let Some(path) = continuation {
            return Some(WorkItem {
                path,
                kind: WorkKind::Continuation,
            });
        }
        self.history(budget).or_else(|| {
            self.continuations.pop().map(|path| WorkItem {
                path,
                kind: WorkKind::Continuation,
            })
        })
    }

    pub fn next(&mut self, budget: &mut DiscoveryBudget) -> Option<WorkItem> {
        // Three changed-file slots then one background slot. The ordinal
        // survives round limits, so even a one-file budget cannot starve the
        // retained historical cursor under a continuous stream of changes.
        let changed_slot = self.work_ordinal % 4 != 3;
        let next = if changed_slot && !self.changed.is_empty() {
            self.changed.pop().map(|path| WorkItem {
                path,
                kind: WorkKind::Changed,
            })
        } else {
            self.background(budget).or_else(|| {
                self.changed.pop().map(|path| WorkItem {
                    path,
                    kind: WorkKind::Changed,
                })
            })
        };
        if next.is_some() {
            self.work_ordinal = self.work_ordinal.wrapping_add(1);
        }
        next
    }

    pub fn has_work(&self) -> bool {
        !self.changed.is_empty()
            || !self.continuations.is_empty()
            || self.cursor.is_some()
            || self.reconcile_requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(scheduler: &mut ScanScheduler, entries: usize) -> Vec<PathBuf> {
        scheduler.begin_round();
        let mut budget = DiscoveryBudget::new(entries, Duration::from_secs(1));
        let mut paths = Vec::new();
        while let Some(item) = scheduler.next(&mut budget) {
            paths.push(item.path);
        }
        paths
    }

    #[test]
    fn tiny_discovery_budgets_resume_until_the_tail_is_covered() {
        let dir = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(home.join("sessions/nested")).unwrap();
        fs::create_dir_all(home.join("archived_sessions")).unwrap();
        let mut expected = HashSet::new();
        for index in 0..23 {
            let path = home.join(format!("sessions/nested/{index}.jsonl"));
            fs::write(&path, b"").unwrap();
            expected.insert(path);
        }
        let archived = home.join("archived_sessions/tail.jsonl");
        fs::write(&archived, b"").unwrap();
        expected.insert(archived);
        let mut scheduler = ScanScheduler::default();
        scheduler.configure(&[home.to_string_lossy().into_owned()]);
        let mut seen = HashSet::new();
        for _ in 0..100 {
            seen.extend(drain(&mut scheduler, 2));
            if !scheduler.has_work() {
                break;
            }
        }
        assert_eq!(seen, expected);
        assert!(!scheduler.has_work());
    }

    #[test]
    fn continuous_dirty_and_continuation_work_cannot_starve_history() {
        let dir = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir_all(home.join("archived_sessions")).unwrap();
        let current = home.join("sessions/live.jsonl");
        fs::write(&current, b"").unwrap();
        let continuing = home.join("sessions/large.jsonl");
        fs::write(&continuing, b"").unwrap();
        let mut expected = HashSet::new();
        for index in 0..12 {
            let path = home.join(format!("archived_sessions/{index}.jsonl"));
            fs::write(&path, b"").unwrap();
            expected.insert(path);
        }
        let mut scheduler = ScanScheduler::default();
        scheduler.configure(&[home.to_string_lossy().into_owned()]);
        let mut seen = HashSet::new();
        let mut saw_continuation = false;
        for _ in 0..200 {
            let mut inbox = ChangeInbox::default();
            inbox.record_file(current.clone());
            scheduler.accept(inbox.take());
            scheduler.continue_file(continuing.clone());
            scheduler.begin_round();
            let mut budget = DiscoveryBudget::new(2, Duration::from_secs(1));
            if let Some(item) = scheduler.next(&mut budget) {
                match item.kind {
                    WorkKind::Reconciliation => {
                        seen.insert(item.path);
                    }
                    WorkKind::Continuation => saw_continuation = true,
                    WorkKind::Changed => {}
                }
            }
            if expected.is_subset(&seen) {
                break;
            }
        }
        assert!(expected.is_subset(&seen));
        assert!(saw_continuation);
    }

    #[test]
    fn repeated_reconciliation_requests_do_not_reset_the_active_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(directory.path()).unwrap();
        fs::create_dir_all(home.join("sessions")).unwrap();
        let mut expected = HashSet::new();
        for index in 0..23 {
            let path = home.join(format!("sessions/{index}.jsonl"));
            fs::write(&path, b"").unwrap();
            expected.insert(path);
        }
        let mut scheduler = ScanScheduler::default();
        scheduler.configure(&[home.to_string_lossy().into_owned()]);
        let mut seen = HashSet::new();
        for _ in 0..100 {
            scheduler.request_reconcile();
            seen.extend(drain(&mut scheduler, 2));
            if expected.is_subset(&seen) {
                break;
            }
        }
        assert_eq!(seen, expected);
        for _ in 0..100 {
            drain(&mut scheduler, 2);
            if !scheduler.has_work() {
                break;
            }
        }
        assert!(!scheduler.has_work());
    }

    #[test]
    fn source_configuration_change_discards_previous_pending_work() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        let previous_home = root.join("previous");
        let next_home = root.join("next");
        fs::create_dir_all(previous_home.join("sessions")).unwrap();
        fs::create_dir_all(next_home.join("sessions")).unwrap();
        let previous_file = previous_home.join("sessions/previous.jsonl");
        let next_file = next_home.join("sessions/next.jsonl");
        fs::write(&previous_file, b"").unwrap();
        fs::write(&next_file, b"").unwrap();
        let mut scheduler = ScanScheduler::default();
        scheduler.configure(&[previous_home.to_string_lossy().into_owned()]);
        scheduler.begin_round();
        let mut inbox = ChangeInbox::default();
        inbox.record_file(previous_file.clone());
        scheduler.accept(inbox.take());
        scheduler.continue_file(previous_file.clone());
        scheduler.configure(&[next_home.to_string_lossy().into_owned()]);
        // Delayed notifications from the replaced watcher remain disallowed.
        inbox.record_file(previous_file);
        scheduler.accept(inbox.take());
        assert_eq!(drain(&mut scheduler, 100), vec![next_file]);
        assert!(!scheduler.has_work());
    }

    #[test]
    fn duplicate_events_are_bounded_and_overflow_requests_reconciliation() {
        let mut inbox = ChangeInbox::default();
        for _ in 0..10_000 {
            inbox.record_file(PathBuf::from("/fixture/repeated.jsonl"));
        }
        assert_eq!(inbox.files.paths.len(), 1);
        for index in 0..MAX_PENDING_PATHS * 2 {
            inbox.record_file(PathBuf::from(format!("/fixture/{index}.jsonl")));
        }
        let changes = inbox.take();
        assert_eq!(changes.files.len(), MAX_PENDING_PATHS);
        assert!(changes.reconcile);
        assert!(inbox.take().files.is_empty());
    }

    #[test]
    fn changed_paths_cannot_bypass_source_depth_or_hidden_directory_rules() {
        let mut scheduler = ScanScheduler::default();
        scheduler.configure(&["/fixture".into()]);
        let mut inbox = ChangeInbox::default();
        for path in [
            "/outside/file.jsonl",
            "/fixture/sessions/../outside.jsonl",
            "/fixture/sessions/.hidden/file.jsonl",
            "/fixture/sessions/file.txt",
        ] {
            inbox.record_file(PathBuf::from(path));
        }
        inbox.record_file(PathBuf::from(format!(
            "/fixture/sessions/{}/file.jsonl",
            "deep/".repeat(MAX_DEPTH)
        )));
        scheduler.accept(inbox.take());
        let mut budget = DiscoveryBudget::new(0, Duration::ZERO);
        assert!(scheduler.next(&mut budget).is_none());
    }
}
