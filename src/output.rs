// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Compact JSON envelopes and human-readable CLI output.

use mkd_worktree_watchdog::metrics::LifetimeMetrics;
use mkd_worktree_watchdog::rules::Action;
use mkd_worktree_watchdog::WatchdogReport;
use serde::Serialize;
use std::io::{self, Write};
use std::path::Path;

/// Output format for command results (default: compact single-line JSON envelope).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Canonical single-line JSON envelope (default). Best for agents and scripting with `jq`.
    #[default]
    Compact,
    /// Pretty-printed JSON envelope.
    Json,
    /// Human-readable text (no JSON).
    Human,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "UPPERCASE")]
enum ResultCode {
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum ErrorLevel {
    Error,
}

#[derive(Debug, Clone, Serialize)]
struct CliResponse<T: Serialize> {
    result: ResultCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<ErrorLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

pub fn print_data<T: Serialize>(
    message: impl Into<String>,
    data: &T,
    format: OutputFormat,
) -> io::Result<()> {
    let response = CliResponse {
        result: ResultCode::Ok,
        message: message.into(),
        level: None,
        data: Some(data),
        error_code: None,
    };
    emit_json(&response, format)
}

pub fn print_report(report: &WatchdogReport, format: OutputFormat) -> io::Result<()> {
    match format {
        OutputFormat::Human => print_report_human(report),
        OutputFormat::Compact | OutputFormat::Json => {
            let msg = if report.dry_run {
                "Worktree scan complete"
            } else {
                "Worktree prune complete"
            };
            print_data(msg, report, format)
        }
    }
}

pub fn print_error(
    message: impl Into<String>,
    error_code: &str,
    format: OutputFormat,
) -> io::Result<()> {
    let message = message.into();
    match format {
        OutputFormat::Human => writeln!(io::stderr(), "Error: {message}"),
        OutputFormat::Compact | OutputFormat::Json => {
            let response: CliResponse<()> = CliResponse {
                result: ResultCode::Error,
                message,
                level: Some(ErrorLevel::Error),
                data: None,
                error_code: Some(error_code.to_string()),
            };
            emit_json(&response, format)
        }
    }
}

fn emit_json<T: Serialize>(value: &T, format: OutputFormat) -> io::Result<()> {
    let line = match format {
        OutputFormat::Compact => {
            serde_json::to_string(value).map_err(|e| io::Error::other(e.to_string()))?
        }
        OutputFormat::Json => {
            serde_json::to_string_pretty(value).map_err(|e| io::Error::other(e.to_string()))?
        }
        OutputFormat::Human => unreachable!("human handled separately"),
    };
    writeln!(io::stdout(), "{line}")
}

fn print_report_human(report: &WatchdogReport) -> io::Result<()> {
    let mut out = io::stdout().lock();
    let mode = if report.dry_run { "DRY RUN" } else { "PRUNE" };
    let s = &report.summary;

    writeln!(out, "mkd-worktree-watchdog [{mode}]")?;
    if let Some(cfg) = &report.config_path {
        writeln!(out, "  config: {}", cfg.display())?;
    } else {
        writeln!(out, "  config: built-in defaults")?;
    }
    writeln!(
        out,
        "  repos scanned: {} | linked worktrees: {} | orphans: {}",
        s.repos_scanned, s.linked_worktrees_found, s.orphans_found
    )?;
    if report.dry_run {
        writeln!(
            out,
            "  candidates: {} worktree(s), {} metadata entry(ies), {} orphan(s) | kept: {} | reclaimable: {:.4} GB ({})",
            s.worktrees_to_prune,
            s.metadata_to_prune,
            s.orphans_to_prune,
            s.kept,
            report.run_metrics.reclaimable_gb,
            format_bytes(s.reclaimable_bytes)
        )?;
    } else {
        writeln!(
            out,
            "  pruned: {} worktree(s), {} repo metadata, {} orphan(s), {} branch(es) | kept: {} | reclaimed: {:.4} GB ({})",
            s.worktrees_pruned,
            s.metadata_repos_pruned,
            s.orphans_pruned,
            s.branches_deleted,
            s.kept,
            report.run_metrics.reclaimed_gb,
            format_bytes(s.reclaimed_bytes)
        )?;
    }
    writeln!(
        out,
        "  run metrics: duration {} ms | {} {:.4} GB",
        report.run_metrics.duration_ms,
        if report.dry_run {
            "reclaimable"
        } else {
            "reclaimed"
        },
        if report.dry_run {
            report.run_metrics.reclaimable_gb
        } else {
            report.run_metrics.reclaimed_gb
        }
    )?;
    let lt = &report.lifetime_metrics;
    writeln!(
        out,
        "  lifetime metrics: runs {} | total reclaimed {:.4} GB ({}) | worktrees {} | metadata repos {} | orphans {} | branches {}",
        lt.total_runs,
        lt.total_reclaimed_gb,
        format_bytes(lt.total_reclaimed_bytes),
        lt.total_worktrees_pruned,
        lt.total_metadata_repos_pruned,
        lt.total_orphans_pruned,
        lt.total_branches_deleted
    )?;

    if !report.worktrees.is_empty() {
        writeln!(out, "\nLinked worktrees:")?;
        for wt in &report.worktrees {
            let tag = match wt.decision.action {
                Action::PruneWorktree => {
                    if report.dry_run {
                        "WOULD PRUNE"
                    } else {
                        "PRUNED"
                    }
                }
                Action::PruneMetadata => {
                    if report.dry_run {
                        "WOULD PRUNE-META"
                    } else {
                        "PRUNED-META"
                    }
                }
                Action::Keep | Action::PruneOrphan => "KEEP",
            };
            let branch = wt
                .branch
                .as_deref()
                .unwrap_or(if wt.detached { "detached" } else { "-" });
            writeln!(
                out,
                "  [{tag:16}] {} ({branch}, {:.1}h, {}) — {}",
                wt.path.display(),
                wt.age_hours,
                format_bytes(wt.size_bytes),
                wt.decision.reason
            )?;
        }
    }

    if !report.orphans.is_empty() {
        writeln!(out, "\nOrphan directories:")?;
        for orphan in &report.orphans {
            let tag = match orphan.decision.action {
                Action::PruneOrphan => {
                    if report.dry_run {
                        "WOULD PRUNE"
                    } else {
                        "PRUNED"
                    }
                }
                _ => "KEEP",
            };
            writeln!(
                out,
                "  [{tag:16}] {} ({:.1}h, {}) — {}",
                orphan.path.display(),
                orphan.age_hours,
                format_bytes(orphan.size_bytes),
                orphan.decision.reason
            )?;
        }
    }

    if !report.errors.is_empty() {
        writeln!(out, "\nErrors ({}):", report.errors.len())?;
        for err in &report.errors {
            writeln!(out, "  [ERROR] {}: {}", err.path.display(), err.message)?;
        }
    }

    Ok(())
}

pub fn print_lifetime_metrics(
    metrics: &LifetimeMetrics,
    path: Option<&Path>,
    format: OutputFormat,
) -> io::Result<()> {
    match format {
        OutputFormat::Human => {
            let mut out = io::stdout().lock();
            writeln!(out, "mkd-worktree-watchdog [LIFETIME METRICS]")?;
            if let Some(p) = path {
                writeln!(out, "  metrics file: {}", p.display())?;
            }
            writeln!(
                out,
                "  total reclaimed: {:.4} GB ({})",
                metrics.total_reclaimed_gb,
                format_bytes(metrics.total_reclaimed_bytes)
            )?;
            writeln!(
                out,
                "  total runs: {} | worktrees: {} | metadata repos: {} | orphans: {} | branches: {}",
                metrics.total_runs,
                metrics.total_worktrees_pruned,
                metrics.total_metadata_repos_pruned,
                metrics.total_orphans_pruned,
                metrics.total_branches_deleted
            )?;
            if let Some(ts) = metrics.last_run_unix {
                writeln!(
                    out,
                    "  last run (unix): {} | last run reclaimed: {:.4} GB ({})",
                    ts,
                    metrics.last_run_reclaimed_gb,
                    format_bytes(metrics.last_run_reclaimed_bytes)
                )?;
            }
            Ok(())
        }
        OutputFormat::Compact | OutputFormat::Json => {
            print_data("Lifetime metrics", metrics, format)
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}
