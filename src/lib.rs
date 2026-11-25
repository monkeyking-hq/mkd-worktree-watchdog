// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! `mkd-worktree-watchdog` — rule-based watchdog for pruning stale git worktrees
//! and reclaiming disk space without external AI dependencies.

pub mod config;
pub mod error;
pub mod git;
pub mod metrics;
pub mod rules;
pub mod service;

use crate::config::Config;
use crate::error::WatchdogError;
use crate::git::{
    discover_orphans, discover_repos, find_active_process_cwds, inspect_worktree, list_worktrees,
    run_git, OrphanInspection, WorktreeInspection,
};
use crate::metrics::{append_run_log, bytes_to_gb, unix_timestamp, LifetimeMetrics, RunMetrics};
use crate::rules::{evaluate_orphan, evaluate_worktree, is_under_root, Action, Decision};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

/// Evaluated linked worktree paired with its rule decision.
#[derive(Debug, Clone, Serialize)]
pub struct EvaluatedWorktree {
    pub repo_path: PathBuf,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
    pub exists: bool,
    pub is_dirty: bool,
    pub has_unpushed_commits: bool,
    pub has_active_process: bool,
    pub age_hours: f64,
    pub size_bytes: u64,
    pub decision: Decision,
}

/// Evaluated orphan directory paired with its rule decision.
#[derive(Debug, Clone, Serialize)]
pub struct EvaluatedOrphan {
    pub path: PathBuf,
    pub age_hours: f64,
    pub size_bytes: u64,
    pub has_active_process: bool,
    pub decision: Decision,
}

/// Per-item error recorded during scan or prune without aborting the full run.
#[derive(Debug, Clone, Serialize)]
pub struct ItemError {
    pub path: PathBuf,
    pub message: String,
}

/// Summary statistics for a scan or prune pass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunSummary {
    pub repos_scanned: usize,
    pub linked_worktrees_found: usize,
    pub orphans_found: usize,
    pub worktrees_to_prune: usize,
    pub metadata_to_prune: usize,
    pub orphans_to_prune: usize,
    pub kept: usize,
    pub worktrees_pruned: usize,
    pub metadata_repos_pruned: usize,
    pub orphans_pruned: usize,
    pub branches_deleted: usize,
    pub reclaimable_bytes: u64,
    pub reclaimed_bytes: u64,
    pub error_count: usize,
}

/// Complete report emitted by `scan` or `prune`.
#[derive(Debug, Clone, Serialize)]
pub struct WatchdogReport {
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics_path: Option<PathBuf>,
    pub summary: RunSummary,
    pub run_metrics: RunMetrics,
    pub lifetime_metrics: LifetimeMetrics,
    pub worktrees: Vec<EvaluatedWorktree>,
    pub orphans: Vec<EvaluatedOrphan>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ItemError>,
}

/// Scan all configured roots and evaluate every linked worktree and orphan directory
/// without modifying disk or git state.
pub fn scan(
    config: &Config,
    config_path: Option<PathBuf>,
) -> Result<WatchdogReport, WatchdogError> {
    run_with_time(config, config_path, true, SystemTime::now())
}

/// Evaluate and prune stale worktrees, prunable metadata, and orphan directories.
/// Pass `dry_run = true` to preview actions without deleting anything.
pub fn prune(
    config: &Config,
    config_path: Option<PathBuf>,
    dry_run: bool,
) -> Result<WatchdogReport, WatchdogError> {
    run_with_time(config, config_path, dry_run, SystemTime::now())
}

/// Execute a scan or prune pass using an explicit reference timestamp (useful for tests).
pub fn run_with_time(
    config: &Config,
    config_path: Option<PathBuf>,
    dry_run: bool,
    now: SystemTime,
) -> Result<WatchdogReport, WatchdogError> {
    let started = Instant::now();
    config.validate()?;

    let active_cwds = if config.defaults.skip_active_processes {
        find_active_process_cwds()
    } else {
        HashSet::new()
    };

    let repos = discover_repos(&config.scan_roots, config.max_scan_depth);
    let mut summary = RunSummary {
        repos_scanned: repos.len(),
        ..RunSummary::default()
    };

    let mut known_worktrees = HashSet::new();
    let mut evaluated_worktrees = Vec::new();
    let mut errors = Vec::new();
    let mut repos_needing_metadata_prune = HashSet::new();

    for repo in &repos {
        let entries = match list_worktrees(repo) {
            Ok(e) => e,
            Err(err) => {
                errors.push(ItemError {
                    path: repo.clone(),
                    message: err.to_string(),
                });
                continue;
            }
        };

        for entry in entries {
            if let Ok(canon) = fs::canonicalize(&entry.path) {
                known_worktrees.insert(canon);
            } else {
                known_worktrees.insert(entry.path.clone());
            }

            if entry.is_main {
                continue;
            }

            summary.linked_worktrees_found += 1;
            let inspection: WorktreeInspection = inspect_worktree(repo, entry, &active_cwds, now);
            let decision = evaluate_worktree(config, &inspection)?;

            match decision.action {
                Action::PruneWorktree => {
                    summary.worktrees_to_prune += 1;
                    summary.reclaimable_bytes = summary
                        .reclaimable_bytes
                        .saturating_add(inspection.size_bytes);
                }
                Action::PruneMetadata => {
                    summary.metadata_to_prune += 1;
                    repos_needing_metadata_prune.insert(repo.clone());
                }
                Action::Keep | Action::PruneOrphan => {
                    summary.kept += 1;
                }
            }

            evaluated_worktrees.push(EvaluatedWorktree {
                repo_path: inspection.repo_path,
                path: inspection.entry.path,
                branch: inspection.entry.branch,
                detached: inspection.entry.detached,
                locked: inspection.entry.locked,
                prunable: inspection.entry.prunable,
                exists: inspection.exists,
                is_dirty: inspection.is_dirty,
                has_unpushed_commits: inspection.has_unpushed_commits,
                has_active_process: inspection.has_active_process,
                age_hours: (inspection.age_hours * 10.0).round() / 10.0,
                size_bytes: inspection.size_bytes,
                decision,
            });
        }
    }

    // Discover and evaluate orphan directories under configured orphan_roots.
    let orphan_inspections: Vec<OrphanInspection> =
        discover_orphans(&config.orphan_roots, &known_worktrees, &active_cwds, now);
    summary.orphans_found = orphan_inspections.len();

    let mut evaluated_orphans = Vec::new();
    for orphan in orphan_inspections {
        let decision = evaluate_orphan(config, &orphan)?;
        match decision.action {
            Action::PruneOrphan => {
                summary.orphans_to_prune += 1;
                summary.reclaimable_bytes =
                    summary.reclaimable_bytes.saturating_add(orphan.size_bytes);
            }
            _ => {
                summary.kept += 1;
            }
        }
        evaluated_orphans.push(EvaluatedOrphan {
            path: orphan.path,
            age_hours: (orphan.age_hours * 10.0).round() / 10.0,
            size_bytes: orphan.size_bytes,
            has_active_process: orphan.has_active_process,
            decision,
        });
    }

    if !dry_run {
        // 1. Remove stale linked worktrees.
        for wt in &evaluated_worktrees {
            if wt.decision.action != Action::PruneWorktree {
                continue;
            }
            let path_str = wt.path.to_string_lossy();
            let mut args = vec!["worktree", "remove"];
            if wt.is_dirty || wt.locked {
                args.push("--force");
            }
            if wt.locked {
                // git requires `--force --force` to remove a locked worktree.
                args.push("--force");
            }
            args.push(path_str.as_ref());

            match run_git(&wt.repo_path, &args) {
                Ok(_) => {
                    summary.worktrees_pruned += 1;
                    summary.reclaimed_bytes = summary.reclaimed_bytes.saturating_add(wt.size_bytes);
                    if wt.decision.delete_branch {
                        if let Some(branch) = wt.branch.as_deref() {
                            // Use lowercase `-d` so git itself refuses to delete unmerged branches.
                            if run_git(&wt.repo_path, &["branch", "-d", branch]).is_ok() {
                                summary.branches_deleted += 1;
                            }
                        }
                    }
                }
                Err(err) => {
                    errors.push(ItemError {
                        path: wt.path.clone(),
                        message: err.to_string(),
                    });
                }
            }
        }

        // 2. Run `git worktree prune` on repos with missing/prunable metadata.
        let mut sorted_repos: Vec<PathBuf> = repos_needing_metadata_prune.into_iter().collect();
        sorted_repos.sort();
        for repo in sorted_repos {
            match run_git(&repo, &["worktree", "prune"]) {
                Ok(_) => {
                    summary.metadata_repos_pruned += 1;
                }
                Err(err) => {
                    errors.push(ItemError {
                        path: repo,
                        message: err.to_string(),
                    });
                }
            }
        }

        // 3. Remove approved orphan directories (strictly verified to be under orphan_roots).
        for orphan in &evaluated_orphans {
            if orphan.decision.action != Action::PruneOrphan {
                continue;
            }
            if !is_under_root(&orphan.path, &config.orphan_roots)
                || config.orphan_roots.iter().any(|r| r == &orphan.path)
            {
                errors.push(ItemError {
                    path: orphan.path.clone(),
                    message: "refusing to delete path outside configured orphan_roots".to_string(),
                });
                continue;
            }
            match fs::remove_dir_all(&orphan.path) {
                Ok(_) => {
                    summary.orphans_pruned += 1;
                    summary.reclaimed_bytes =
                        summary.reclaimed_bytes.saturating_add(orphan.size_bytes);
                }
                Err(err) => {
                    errors.push(ItemError {
                        path: orphan.path.clone(),
                        message: format!("failed to remove orphan directory: {err}"),
                    });
                }
            }
        }
    }

    summary.error_count = errors.len();

    let timestamp_unix = unix_timestamp(now);
    let run_metrics = RunMetrics {
        timestamp_unix,
        duration_ms: started.elapsed().as_millis() as u64,
        reclaimed_bytes: summary.reclaimed_bytes,
        reclaimed_gb: bytes_to_gb(summary.reclaimed_bytes),
        reclaimable_bytes: summary.reclaimable_bytes,
        reclaimable_gb: bytes_to_gb(summary.reclaimable_bytes),
    };

    let metrics_path = config.effective_metrics_path();
    let mut lifetime_metrics = match metrics_path.as_deref() {
        Some(path) => match LifetimeMetrics::load_or_default(path) {
            Ok(m) => m,
            Err(err) => {
                errors.push(ItemError {
                    path: path.to_path_buf(),
                    message: err.to_string(),
                });
                summary.error_count = errors.len();
                LifetimeMetrics::default()
            }
        },
        None => LifetimeMetrics::default(),
    };

    if !dry_run {
        lifetime_metrics.record_run(&summary, timestamp_unix);
        if let Some(path) = metrics_path.as_deref() {
            if let Err(err) = lifetime_metrics.save(path) {
                errors.push(ItemError {
                    path: path.to_path_buf(),
                    message: err.to_string(),
                });
                summary.error_count = errors.len();
            } else if let Err(err) = append_run_log(path, &summary, &run_metrics, &lifetime_metrics)
            {
                errors.push(ItemError {
                    path: path.to_path_buf(),
                    message: err.to_string(),
                });
                summary.error_count = errors.len();
            }
        }
    }

    Ok(WatchdogReport {
        dry_run,
        config_path,
        metrics_path,
        summary,
        run_metrics,
        lifetime_metrics,
        worktrees: evaluated_worktrees,
        orphans: evaluated_orphans,
        errors,
    })
}
