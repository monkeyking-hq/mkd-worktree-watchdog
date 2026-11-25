// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Deterministic, rule-based evaluation of worktrees and orphan directories.

use crate::config::Config;
use crate::error::WatchdogError;
use crate::git::{OrphanInspection, WorktreeInspection};
use serde::Serialize;
use std::path::Path;

/// Recommended action for a worktree or orphan directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Remove the linked worktree from disk and git metadata.
    PruneWorktree,
    /// Run `git worktree prune` to clean missing/prunable metadata.
    PruneMetadata,
    /// Remove an orphaned directory under `orphan_roots`.
    PruneOrphan,
    /// Keep untouched.
    Keep,
}

/// Evaluation outcome for a single worktree or orphan.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub action: Action,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule: Option<String>,
    pub delete_branch: bool,
}

/// Evaluate a linked (or main) worktree against the configured ruleset.
pub fn evaluate_worktree(
    config: &Config,
    wt: &WorktreeInspection,
) -> Result<Decision, WatchdogError> {
    if wt.entry.is_main {
        return Ok(keep("main repository worktree", None));
    }

    if matches_any_glob(&wt.entry.path.to_string_lossy(), &config.exclude_paths)? {
        return Ok(keep("matched exclude_paths", None));
    }

    let (policy, matched_rule) = config.effective_policy_for(&wt.entry.path)?;
    let rule_name = matched_rule.map(|r| r.name.clone());

    if let Some(rule) = matched_rule {
        if rule.enabled == Some(false) {
            return Ok(keep("disabled by matching rule", rule_name));
        }
    }

    if wt.entry.prunable || !wt.exists {
        return if policy.prune_missing {
            Ok(Decision {
                action: Action::PruneMetadata,
                reason: wt
                    .entry
                    .prunable_reason
                    .clone()
                    .unwrap_or_else(|| "worktree directory missing on disk".to_string()),
                matched_rule: rule_name,
                delete_branch: false,
            })
        } else {
            Ok(keep("prune_missing is disabled", rule_name))
        };
    }

    if let Some(branch) = wt.entry.branch.as_deref() {
        if matches_any_glob(branch, &config.exclude_branches)? {
            return Ok(keep(
                format!("branch '{branch}' matched exclude_branches"),
                rule_name,
            ));
        }
    }

    if wt.entry.locked && !policy.allow_locked {
        return Ok(keep("worktree is locked", rule_name));
    }

    if wt.has_active_process && policy.skip_active_processes {
        return Ok(keep("active process using worktree", rule_name));
    }

    if wt.is_dirty && !policy.allow_dirty {
        match policy.dirty_max_age_hours {
            Some(limit) if wt.age_hours >= limit as f64 => {}
            Some(limit) => {
                return Ok(keep(
                    format!(
                        "dirty worktree younger than dirty_max_age_hours ({:.1}h < {}h)",
                        wt.age_hours, limit
                    ),
                    rule_name,
                ));
            }
            None => {
                return Ok(keep("worktree has uncommitted changes", rule_name));
            }
        }
    }

    if wt.has_unpushed_commits && !policy.allow_unpushed {
        return Ok(keep("worktree has unpushed or unmerged commits", rule_name));
    }

    if wt.age_hours < policy.max_age_hours as f64 {
        return Ok(keep(
            format!(
                "idle {:.1}h < max_age_hours ({}h)",
                wt.age_hours, policy.max_age_hours
            ),
            rule_name,
        ));
    }

    Ok(Decision {
        action: Action::PruneWorktree,
        reason: format!(
            "stale worktree (idle {:.1}h >= {}h)",
            wt.age_hours, policy.max_age_hours
        ),
        matched_rule: rule_name,
        delete_branch: policy.delete_merged_branches && wt.entry.branch.is_some(),
    })
}

/// Evaluate an orphaned directory discovered under `orphan_roots`.
pub fn evaluate_orphan(
    config: &Config,
    orphan: &OrphanInspection,
) -> Result<Decision, WatchdogError> {
    if !config.defaults.prune_orphans {
        return Ok(keep("prune_orphans is disabled", None));
    }

    if matches_any_glob(&orphan.path.to_string_lossy(), &config.exclude_paths)? {
        return Ok(keep("matched exclude_paths", None));
    }

    if orphan.has_active_process && config.defaults.skip_active_processes {
        return Ok(keep("active process using orphan directory", None));
    }

    if orphan.age_hours < config.defaults.orphan_max_age_hours as f64 {
        return Ok(keep(
            format!(
                "orphan idle {:.1}h < orphan_max_age_hours ({}h)",
                orphan.age_hours, config.defaults.orphan_max_age_hours
            ),
            None,
        ));
    }

    Ok(Decision {
        action: Action::PruneOrphan,
        reason: format!("{} (idle {:.1}h)", orphan.reason, orphan.age_hours),
        matched_rule: None,
        delete_branch: false,
    })
}

fn keep(reason: impl Into<String>, matched_rule: Option<String>) -> Decision {
    Decision {
        action: Action::Keep,
        reason: reason.into(),
        matched_rule,
        delete_branch: false,
    }
}

fn matches_any_glob(value: &str, patterns: &[String]) -> Result<bool, WatchdogError> {
    for pat in patterns {
        let matcher = globset::Glob::new(pat)
            .map_err(|e| WatchdogError::InvalidConfig(format!("invalid glob '{pat}': {e}")))?
            .compile_matcher();
        if matcher.is_match(value) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Helper to check whether a path is inside any configured `orphan_roots`.
pub fn is_under_root(path: &Path, roots: &[std::path::PathBuf]) -> bool {
    roots.iter().any(|r| path.starts_with(r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::WorktreeEntry;
    use std::path::PathBuf;

    fn sample_wt() -> WorktreeInspection {
        WorktreeInspection {
            repo_path: PathBuf::from("/home/user/workspaces/app"),
            entry: WorktreeEntry {
                path: PathBuf::from("/tmp/wt-feature"),
                head: Some("abcdef123456".to_string()),
                branch: Some("feat/old".to_string()),
                detached: false,
                bare: false,
                locked: false,
                lock_reason: None,
                prunable: false,
                prunable_reason: None,
                is_main: false,
            },
            exists: true,
            age_hours: 96.0,
            size_bytes: 1024 * 1024,
            is_dirty: false,
            has_unpushed_commits: false,
            has_active_process: false,
        }
    }

    #[test]
    fn prunes_stale_clean_pushed_worktree() {
        let cfg = Config::default();
        let d = evaluate_worktree(&cfg, &sample_wt()).unwrap();
        assert_eq!(d.action, Action::PruneWorktree);
    }

    #[test]
    fn keeps_dirty_worktree_by_default() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.is_dirty = true;
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::Keep);
        assert!(d.reason.contains("uncommitted"));
    }

    #[test]
    fn keeps_unpushed_worktree_by_default() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.has_unpushed_commits = true;
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::Keep);
        assert!(d.reason.contains("unpushed"));
    }

    #[test]
    fn keeps_active_process_worktree() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.has_active_process = true;
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::Keep);
        assert!(d.reason.contains("active process"));
    }

    #[test]
    fn keeps_locked_worktree_by_default() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.entry.locked = true;
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::Keep);
        assert!(d.reason.contains("locked"));
    }

    #[test]
    fn keeps_excluded_branch() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.entry.branch = Some("release/v1.0".to_string());
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::Keep);
        assert!(d.reason.contains("exclude_branches"));
    }

    #[test]
    fn prunes_missing_metadata() {
        let cfg = Config::default();
        let mut wt = sample_wt();
        wt.exists = false;
        wt.entry.prunable = true;
        let d = evaluate_worktree(&cfg, &wt).unwrap();
        assert_eq!(d.action, Action::PruneMetadata);
    }
}
