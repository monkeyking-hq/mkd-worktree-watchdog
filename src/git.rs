// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Git repository discovery, worktree inspection, orphan detection, and active-process checks.

use crate::error::WatchdogError;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    "vendor",
    ".cargo",
];

/// Parsed entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub prunable: bool,
    pub prunable_reason: Option<String>,
    pub is_main: bool,
}

/// Full inspection result for a linked worktree.
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeInspection {
    pub repo_path: PathBuf,
    pub entry: WorktreeEntry,
    pub exists: bool,
    pub age_hours: f64,
    pub size_bytes: u64,
    pub is_dirty: bool,
    pub has_unpushed_commits: bool,
    pub has_active_process: bool,
}

/// An orphaned directory discovered under `orphan_roots`.
#[derive(Debug, Clone, Serialize)]
pub struct OrphanInspection {
    pub path: PathBuf,
    pub reason: String,
    pub age_hours: f64,
    pub size_bytes: u64,
    pub has_active_process: bool,
}

/// Discover main git repositories (directories containing a `.git/` directory) under `roots`.
pub fn discover_repos(roots: &[PathBuf], max_depth: usize) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        walk_for_repos(root, 0, max_depth, &mut seen, &mut repos);
    }
    repos.sort();
    repos
}

fn walk_for_repos(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    seen: &mut HashSet<PathBuf>,
    repos: &mut Vec<PathBuf>,
) {
    let dot_git = dir.join(".git");
    if dot_git.is_dir() {
        let canonical = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if seen.insert(canonical.clone()) {
            repos.push(canonical);
        }
        // Still allow nested repos only if depth < max_depth? Usually stop at repo root to avoid
        // scanning submodules/build trees.
        return;
    }

    if depth >= max_depth {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if SKIP_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        walk_for_repos(&entry.path(), depth + 1, max_depth, seen, repos);
    }
}

/// Run `git -C <dir> <args...>` and return stdout on success.
pub fn run_git(dir: &Path, args: &[&str]) -> Result<String, WatchdogError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|source| WatchdogError::Io {
            path: Some(dir.to_path_buf()),
            source,
        })?;

    if !output.status.success() {
        return Err(WatchdogError::GitCommand {
            args: args.iter().map(|s| (*s).to_string()).collect(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            code: output.status.code(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// List all worktrees of a repository via `git worktree list --porcelain`.
pub fn list_worktrees(repo: &Path) -> Result<Vec<WorktreeEntry>, WatchdogError> {
    let stdout = run_git(repo, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_porcelain(&stdout))
}

/// Parse `git worktree list --porcelain` output into structured entries.
pub fn parse_worktree_porcelain(output: &str) -> Vec<WorktreeEntry> {
    let mut result = Vec::new();
    let mut current: Option<WorktreeEntry> = None;

    for raw_line in output.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            if let Some(entry) = current.take() {
                result.push(entry);
            }
            continue;
        }

        if let Some(path_str) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                result.push(entry);
            }
            let is_main = result.is_empty();
            current = Some(WorktreeEntry {
                path: PathBuf::from(path_str),
                head: None,
                branch: None,
                detached: false,
                bare: false,
                locked: false,
                lock_reason: None,
                prunable: false,
                prunable_reason: None,
                is_main,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = Some(head.to_string());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                entry.branch = Some(
                    branch
                        .strip_prefix("refs/heads/")
                        .unwrap_or(branch)
                        .to_string(),
                );
            } else if line == "detached" {
                entry.detached = true;
            } else if line == "bare" {
                entry.bare = true;
            } else if line == "locked" {
                entry.locked = true;
            } else if let Some(reason) = line.strip_prefix("locked ") {
                entry.locked = true;
                entry.lock_reason = Some(reason.to_string());
            } else if line == "prunable" {
                entry.prunable = true;
            } else if let Some(reason) = line.strip_prefix("prunable ") {
                entry.prunable = true;
                entry.prunable_reason = Some(reason.to_string());
            }
        }
    }

    if let Some(entry) = current {
        result.push(entry);
    }

    result
}

/// Inspect a linked worktree for age, disk usage, dirty state, unpushed commits, and active processes.
pub fn inspect_worktree(
    repo_path: &Path,
    entry: WorktreeEntry,
    active_cwds: &HashSet<PathBuf>,
    now: SystemTime,
) -> WorktreeInspection {
    let exists = entry.path.exists();
    if !exists || entry.prunable {
        return WorktreeInspection {
            repo_path: repo_path.to_path_buf(),
            entry,
            exists,
            age_hours: 0.0,
            size_bytes: 0,
            is_dirty: false,
            has_unpushed_commits: false,
            has_active_process: false,
        };
    }

    let has_active_process = path_has_active_process(&entry.path, active_cwds);
    let age_hours = compute_worktree_age_hours(&entry.path, now);
    let size_bytes = compute_dir_size_bytes(&entry.path);
    let is_dirty = check_worktree_dirty(&entry.path);
    let has_unpushed_commits = check_unpushed_commits(repo_path, &entry);

    WorktreeInspection {
        repo_path: repo_path.to_path_buf(),
        entry,
        exists,
        age_hours,
        size_bytes,
        is_dirty,
        has_unpushed_commits,
        has_active_process,
    }
}

/// Check if a worktree has any uncommitted changes (staged, unstaged, or untracked).
/// Defaults to `true` (safe) if `git status` fails.
fn check_worktree_dirty(wt_path: &Path) -> bool {
    match run_git(wt_path, &["status", "--porcelain"]) {
        Ok(stdout) => !stdout.trim().is_empty(),
        Err(_) => true,
    }
}

/// Check if a worktree has commits that are not pushed to upstream and not reachable from
/// the repository's default/other branches.
fn check_unpushed_commits(repo_path: &Path, entry: &WorktreeEntry) -> bool {
    let Some(head) = entry.head.as_deref() else {
        return false;
    };

    if entry.detached {
        // Detached HEAD is safe if reachable from any local or remote branch.
        if let Ok(branches) = run_git(repo_path, &["branch", "-a", "--contains", head]) {
            if !branches.trim().is_empty() {
                return false;
            }
        }
        return true;
    }

    // 1. If the branch has an upstream, check if there are commits ahead of @{u}.
    if let Ok(ahead) = run_git(&entry.path, &["rev-list", "--count", "@{u}..HEAD"]) {
        if let Ok(count) = ahead.trim().parse::<u64>() {
            return count > 0;
        }
    }

    // 2. Otherwise, check if HEAD is already merged into the main repo's HEAD or default branch.
    for target in [
        "HEAD",
        "origin/HEAD",
        "refs/heads/main",
        "refs/heads/master",
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .args(["merge-base", "--is-ancestor", head, target])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                return false;
            }
        }
    }

    true
}

/// Compute idle age in hours from the most recent modification time of the worktree's gitdir
/// metadata and a bounded walk of working files (excluding heavy build artifact directories).
pub fn compute_worktree_age_hours(wt_path: &Path, now: SystemTime) -> f64 {
    let mut latest = SystemTime::UNIX_EPOCH;

    // Check worktree root mtime itself.
    if let Ok(meta) = fs::metadata(wt_path) {
        if let Ok(mtime) = meta.modified() {
            if mtime > latest {
                latest = mtime;
            }
        }
    }

    // Check gitdir metadata referenced by `<wt>/.git` file.
    if let Some(gitdir) = resolve_worktree_gitdir(wt_path) {
        for rel in ["index", "HEAD", "logs/HEAD", "gitdir"] {
            if let Ok(meta) = fs::metadata(gitdir.join(rel)) {
                if let Ok(mtime) = meta.modified() {
                    if mtime > latest {
                        latest = mtime;
                    }
                }
            }
        }
    }

    // Bounded walk of working tree (depth 3, max 500 entries) to detect recent file edits.
    let mut visited = 0usize;
    latest_mtime_walk(wt_path, 0, 3, 500, &mut visited, &mut latest);

    elapsed_hours(latest, now)
}

fn latest_mtime_walk(
    dir: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    visited: &mut usize,
    latest: &mut SystemTime,
) {
    if depth > max_depth || *visited >= max_entries {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *visited >= max_entries {
            break;
        }
        *visited += 1;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if SKIP_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(mtime) = meta.modified() {
                if mtime > *latest {
                    *latest = mtime;
                }
            }
            if meta.is_dir() && depth < max_depth {
                latest_mtime_walk(
                    &entry.path(),
                    depth + 1,
                    max_depth,
                    max_entries,
                    visited,
                    latest,
                );
            }
        }
    }
}

/// Resolve the `gitdir:` target from a linked worktree's `.git` file.
pub fn resolve_worktree_gitdir(wt_path: &Path) -> Option<PathBuf> {
    let dot_git = wt_path.join(".git");
    if !dot_git.is_file() {
        return None;
    }
    let content = fs::read_to_string(&dot_git).ok()?;
    let target = content.trim().strip_prefix("gitdir:")?.trim();
    let path = PathBuf::from(target);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(wt_path.join(path))
    }
}

/// Compute total logical file size in bytes of a directory (ignoring symlinks).
pub fn compute_dir_size_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }

    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
    }
    total
}

/// Discover orphaned directories under `orphan_roots`.
/// An immediate child directory is considered an orphan if it is NOT in `known_worktrees` and:
/// - It is completely empty, OR
/// - Its `.git` file points to a non-existent `gitdir:`.
pub fn discover_orphans(
    orphan_roots: &[PathBuf],
    known_worktrees: &HashSet<PathBuf>,
    active_cwds: &HashSet<PathBuf>,
    now: SystemTime,
) -> Vec<OrphanInspection> {
    let mut orphans = Vec::new();
    for root in orphan_roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() || ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if known_worktrees.contains(&canonical) {
                continue;
            }

            let reason = if is_dir_empty(&path) {
                Some("empty abandoned worktree directory".to_string())
            } else if let Some(gitdir) = resolve_worktree_gitdir(&path) {
                if !gitdir.exists() {
                    Some(format!(
                        "broken .git pointer (missing gitdir {})",
                        gitdir.display()
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(reason) = reason {
                let age_hours = compute_worktree_age_hours(&path, now);
                let size_bytes = compute_dir_size_bytes(&path);
                let has_active_process = path_has_active_process(&path, active_cwds);
                orphans.push(OrphanInspection {
                    path: canonical,
                    reason,
                    age_hours,
                    size_bytes,
                    has_active_process,
                });
            }
        }
    }
    orphans.sort_by(|a, b| a.path.cmp(&b.path));
    orphans
}

fn is_dir_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut rd| rd.next().is_none())
        .unwrap_or(false)
}

/// Collect active process current working directories from `/proc/<pid>/cwd` on Linux.
pub fn find_active_process_cwds() -> HashSet<PathBuf> {
    let mut cwds = HashSet::new();
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                if let Ok(target) = fs::read_link(entry.path().join("cwd")) {
                    cwds.insert(target);
                }
            }
        }
    }
    cwds
}

fn path_has_active_process(wt_path: &Path, active_cwds: &HashSet<PathBuf>) -> bool {
    let canonical = fs::canonicalize(wt_path).unwrap_or_else(|_| wt_path.to_path_buf());
    active_cwds.iter().any(|cwd| cwd.starts_with(&canonical))
}

fn elapsed_hours(earlier: SystemTime, now: SystemTime) -> f64 {
    if earlier == UNIX_EPOCH {
        return 0.0;
    }
    match now.duration_since(earlier) {
        Ok(dur) => dur.as_secs_f64() / 3600.0,
        Err(_) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worktree_porcelain_output() {
        let sample = "\
worktree /home/user/repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /tmp/wt-1
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feat/test
locked waiting for review

worktree /tmp/wt-2
HEAD 3333333333333333333333333333333333333333
detached
prunable gitdir file points to non-existent location
";
        let entries = parse_worktree_porcelain(sample);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_main);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));

        assert!(!entries[1].is_main);
        assert!(entries[1].locked);
        assert_eq!(
            entries[1].lock_reason.as_deref(),
            Some("waiting for review")
        );

        assert!(!entries[2].is_main);
        assert!(entries[2].detached);
        assert!(entries[2].prunable);
        assert_eq!(
            entries[2].prunable_reason.as_deref(),
            Some("gitdir file points to non-existent location")
        );
    }
}
