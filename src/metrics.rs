// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Per-run and persisted lifetime metrics (bytes and GB reclaimed) plus JSONL run logging.

use crate::error::WatchdogError;
use crate::RunSummary;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BYTES_PER_GB: f64 = 1_000_000_000.0;

/// Metrics for a single scan or prune execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    pub timestamp_unix: u64,
    pub duration_ms: u64,
    pub reclaimed_bytes: u64,
    pub reclaimed_gb: f64,
    pub reclaimable_bytes: u64,
    pub reclaimable_gb: f64,
}

/// Persisted cumulative lifetime metrics across all non-dry-run prune executions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LifetimeMetrics {
    pub total_runs: u64,
    pub total_worktrees_pruned: u64,
    pub total_metadata_repos_pruned: u64,
    pub total_orphans_pruned: u64,
    pub total_branches_deleted: u64,
    pub total_reclaimed_bytes: u64,
    pub total_reclaimed_gb: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_unix: Option<u64>,
    pub last_run_reclaimed_bytes: u64,
    pub last_run_reclaimed_gb: f64,
}

/// One JSONL audit record appended for each live prune run.
#[derive(Debug, Clone, Serialize)]
struct RunLogRecord {
    timestamp_unix: u64,
    duration_ms: u64,
    repos_scanned: usize,
    worktrees_pruned: usize,
    metadata_repos_pruned: usize,
    orphans_pruned: usize,
    branches_deleted: usize,
    reclaimed_bytes: u64,
    reclaimed_gb: f64,
    lifetime_runs: u64,
    lifetime_reclaimed_bytes: u64,
    lifetime_reclaimed_gb: f64,
    error_count: usize,
}

impl LifetimeMetrics {
    /// Load persisted lifetime metrics from `path`, returning zeroed defaults if the file does not exist.
    pub fn load_or_default(path: &Path) -> Result<Self, WatchdogError> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).map_err(|source| WatchdogError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
        serde_json::from_str(&raw).map_err(|err| WatchdogError::ConfigParse {
            path: path.to_path_buf(),
            message: format!("failed to parse metrics JSON: {err}"),
        })
    }

    /// Accumulate a completed non-dry-run prune pass into lifetime totals.
    pub fn record_run(&mut self, summary: &RunSummary, timestamp_unix: u64) {
        self.total_runs = self.total_runs.saturating_add(1);
        self.total_worktrees_pruned = self
            .total_worktrees_pruned
            .saturating_add(summary.worktrees_pruned as u64);
        self.total_metadata_repos_pruned = self
            .total_metadata_repos_pruned
            .saturating_add(summary.metadata_repos_pruned as u64);
        self.total_orphans_pruned = self
            .total_orphans_pruned
            .saturating_add(summary.orphans_pruned as u64);
        self.total_branches_deleted = self
            .total_branches_deleted
            .saturating_add(summary.branches_deleted as u64);
        self.total_reclaimed_bytes = self
            .total_reclaimed_bytes
            .saturating_add(summary.reclaimed_bytes);
        self.total_reclaimed_gb = bytes_to_gb(self.total_reclaimed_bytes);
        self.last_run_unix = Some(timestamp_unix);
        self.last_run_reclaimed_bytes = summary.reclaimed_bytes;
        self.last_run_reclaimed_gb = bytes_to_gb(summary.reclaimed_bytes);
    }

    /// Atomically persist lifetime metrics to `path` (write temp file + rename).
    pub fn save(&self, path: &Path) -> Result<(), WatchdogError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| WatchdogError::Io {
                path: Some(parent.to_path_buf()),
                source,
            })?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|err| WatchdogError::InvalidConfig(err.to_string()))?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, format!("{json}\n")).map_err(|source| WatchdogError::Io {
            path: Some(tmp_path.clone()),
            source,
        })?;
        #[cfg(target_os = "windows")]
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&tmp_path, path).map_err(|source| WatchdogError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
        Ok(())
    }
}

/// Append a single-line JSONL record of a live prune run to the sibling `runs.jsonl` log file.
pub fn append_run_log(
    metrics_path: &Path,
    summary: &RunSummary,
    run_metrics: &RunMetrics,
    lifetime: &LifetimeMetrics,
) -> Result<PathBuf, WatchdogError> {
    let log_path = run_log_path_for(metrics_path);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|source| WatchdogError::Io {
            path: Some(parent.to_path_buf()),
            source,
        })?;
    }
    let record = RunLogRecord {
        timestamp_unix: run_metrics.timestamp_unix,
        duration_ms: run_metrics.duration_ms,
        repos_scanned: summary.repos_scanned,
        worktrees_pruned: summary.worktrees_pruned,
        metadata_repos_pruned: summary.metadata_repos_pruned,
        orphans_pruned: summary.orphans_pruned,
        branches_deleted: summary.branches_deleted,
        reclaimed_bytes: summary.reclaimed_bytes,
        reclaimed_gb: run_metrics.reclaimed_gb,
        lifetime_runs: lifetime.total_runs,
        lifetime_reclaimed_bytes: lifetime.total_reclaimed_bytes,
        lifetime_reclaimed_gb: lifetime.total_reclaimed_gb,
        error_count: summary.error_count,
    };
    let line = serde_json::to_string(&record)
        .map_err(|err| WatchdogError::InvalidConfig(err.to_string()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|source| WatchdogError::Io {
            path: Some(log_path.clone()),
            source,
        })?;
    writeln!(file, "{line}").map_err(|source| WatchdogError::Io {
        path: Some(log_path.clone()),
        source,
    })?;
    Ok(log_path)
}

/// Derive the JSONL run log path alongside the metrics JSON file.
pub fn run_log_path_for(metrics_path: &Path) -> PathBuf {
    metrics_path
        .parent()
        .map(|p| p.join("runs.jsonl"))
        .unwrap_or_else(|| PathBuf::from("runs.jsonl"))
}

/// Default state path for persisted lifetime metrics:
/// - Linux/macOS: `$XDG_STATE_HOME/mkd-worktree-watchdog/metrics.json` or `~/.local/state/mkd-worktree-watchdog/metrics.json`
/// - Windows: `%LOCALAPPDATA%\mkd-worktree-watchdog\metrics.json`
pub fn default_metrics_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = env::var("LOCALAPPDATA") {
            if !local.trim().is_empty() {
                return Some(
                    PathBuf::from(local)
                        .join("mkd-worktree-watchdog")
                        .join("metrics.json"),
                );
            }
        }
    }
    if let Ok(xdg) = env::var("XDG_STATE_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("mkd-worktree-watchdog/metrics.json"));
        }
    }
    crate::config::home_dir().map(|h| {
        h.join(".local")
            .join("state")
            .join("mkd-worktree-watchdog")
            .join("metrics.json")
    })
}

/// Convert bytes to decimal gigabytes (GB), rounded to 4 decimal places.
pub fn bytes_to_gb(bytes: u64) -> f64 {
    let raw = bytes as f64 / BYTES_PER_GB;
    (raw * 10_000.0).round() / 10_000.0
}

/// Current unix timestamp in seconds for a `SystemTime`.
pub fn unix_timestamp(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn bytes_to_gb_rounds_cleanly() {
        assert_eq!(bytes_to_gb(0), 0.0);
        assert_eq!(bytes_to_gb(1_500_000_000), 1.5);
        assert_eq!(bytes_to_gb(250_000_000), 0.25);
    }

    #[test]
    fn lifetime_metrics_accumulates_and_persists() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("metrics.json");

        let mut lifetime = LifetimeMetrics::load_or_default(&path).unwrap();
        assert_eq!(lifetime.total_runs, 0);

        let s1 = RunSummary {
            worktrees_pruned: 2,
            metadata_repos_pruned: 1,
            orphans_pruned: 1,
            branches_deleted: 1,
            reclaimed_bytes: 1_250_000_000,
            ..RunSummary::default()
        };
        lifetime.record_run(&s1, 1_700_000_000);
        lifetime.save(&path).unwrap();

        let mut loaded = LifetimeMetrics::load_or_default(&path).unwrap();
        assert_eq!(loaded.total_runs, 1);
        assert_eq!(loaded.total_reclaimed_bytes, 1_250_000_000);
        assert_eq!(loaded.total_reclaimed_gb, 1.25);

        let s2 = RunSummary {
            worktrees_pruned: 1,
            reclaimed_bytes: 750_000_000,
            ..RunSummary::default()
        };
        loaded.record_run(&s2, 1_700_043_200);
        loaded.save(&path).unwrap();

        let reloaded = LifetimeMetrics::load_or_default(&path).unwrap();
        assert_eq!(reloaded.total_runs, 2);
        assert_eq!(reloaded.total_worktrees_pruned, 3);
        assert_eq!(reloaded.total_reclaimed_bytes, 2_000_000_000);
        assert_eq!(reloaded.total_reclaimed_gb, 2.0);
        assert_eq!(reloaded.last_run_reclaimed_gb, 0.75);
    }
}
