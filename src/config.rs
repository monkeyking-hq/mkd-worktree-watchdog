// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Configuration schema, sane defaults, tilde expansion, and TOML loading.

use crate::error::WatchdogError;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Top-level watchdog configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Directories to scan recursively for git repositories.
    pub scan_roots: Vec<PathBuf>,
    /// Maximum directory recursion depth when searching `scan_roots` for git repos.
    pub max_scan_depth: usize,
    /// Parent directories where coding agents store standalone worktrees.
    /// Immediate children that are empty or have broken `.git` pointers are treated as orphans.
    pub orphan_roots: Vec<PathBuf>,
    /// Glob patterns for worktree or orphan paths that must never be pruned.
    pub exclude_paths: Vec<String>,
    /// Glob patterns for branch names that must never be pruned.
    pub exclude_branches: Vec<String>,
    /// Default pruning policy applied to all discovered worktrees.
    pub defaults: Policy,
    /// Per-path rule overrides, evaluated in order (first matching rule wins).
    pub rules: Vec<PathRule>,
    /// Schedule settings for the built-in daemon mode.
    pub schedule: ScheduleConfig,
    /// Optional path to persisted lifetime metrics JSON file.
    /// Defaults to `$XDG_STATE_HOME/mkd-worktree-watchdog/metrics.json`
    /// or `~/.local/state/mkd-worktree-watchdog/metrics.json`.
    #[serde(default)]
    pub metrics_path: Option<PathBuf>,
}

/// Core pruning safety and staleness policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Policy {
    /// Minimum idle age in hours before a clean, pushed/merged linked worktree is stale.
    pub max_age_hours: u64,
    /// Run `git worktree prune` on repositories that have missing/prunable worktree metadata.
    pub prune_missing: bool,
    /// Allow pruning worktrees with uncommitted changes (staged, unstaged, or untracked).
    pub allow_dirty: bool,
    /// Optional extended idle age (in hours) after which even dirty worktrees may be pruned.
    /// `None` (default) means dirty worktrees are never pruned unless `allow_dirty = true`.
    pub dirty_max_age_hours: Option<u64>,
    /// Allow pruning worktrees whose branch or detached HEAD has unpushed/unmerged commits.
    pub allow_unpushed: bool,
    /// Allow pruning locked git worktrees (`git worktree lock`).
    pub allow_locked: bool,
    /// Never prune a worktree if a running process has its current working directory inside it.
    pub skip_active_processes: bool,
    /// Delete the local branch after pruning its stale worktree (only if merged into default branch).
    pub delete_merged_branches: bool,
    /// Prune orphaned directories under `orphan_roots` (broken `.git` pointer or empty directory).
    pub prune_orphans: bool,
    /// Minimum idle age in hours before an orphaned directory under `orphan_roots` is removed.
    pub orphan_max_age_hours: u64,
}

/// Optional per-path rule override.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathRule {
    /// Human-readable name for reporting.
    pub name: String,
    /// Glob pattern matched against the worktree's full path (e.g. `**/antigravity-cli/worktrees/**`).
    pub match_path: String,
    /// If `Some(false)`, matching worktrees are always kept.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Override `max_age_hours` for matching worktrees.
    #[serde(default)]
    pub max_age_hours: Option<u64>,
    /// Override `allow_dirty` for matching worktrees.
    #[serde(default)]
    pub allow_dirty: Option<bool>,
    /// Override `dirty_max_age_hours` for matching worktrees.
    #[serde(default)]
    pub dirty_max_age_hours: Option<u64>,
    /// Override `allow_unpushed` for matching worktrees.
    #[serde(default)]
    pub allow_unpushed: Option<bool>,
    /// Override `allow_locked` for matching worktrees.
    #[serde(default)]
    pub allow_locked: Option<bool>,
    /// Override `delete_merged_branches` for matching worktrees.
    #[serde(default)]
    pub delete_merged_branches: Option<bool>,
}

/// Built-in daemon scheduling configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ScheduleConfig {
    /// Interval between automatic prune passes in hours (default: 12 = twice daily).
    pub interval_hours: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_age_hours: 72,
            prune_missing: true,
            allow_dirty: false,
            dirty_max_age_hours: None,
            allow_unpushed: false,
            allow_locked: false,
            skip_active_processes: true,
            delete_merged_branches: false,
            prune_orphans: true,
            orphan_max_age_hours: 24,
        }
    }
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self { interval_hours: 12 }
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut scan_roots = Vec::new();
        let mut orphan_roots = Vec::new();

        if let Some(home) = home_dir() {
            let workspaces = home.join("workspaces");
            if workspaces.is_dir() {
                scan_roots.push(workspaces);
            }
            for rel in [
                ".gemini/antigravity-cli/worktrees",
                ".claude/worktrees",
                ".codex/worktrees",
                ".opencode/worktrees",
            ] {
                let candidate = home.join(rel);
                if candidate.is_dir() {
                    orphan_roots.push(candidate);
                }
            }
        }

        Self {
            scan_roots,
            max_scan_depth: 3,
            orphan_roots,
            exclude_paths: Vec::new(),
            exclude_branches: vec![
                "main".to_string(),
                "master".to_string(),
                "develop".to_string(),
                "release/*".to_string(),
            ],
            defaults: Policy::default(),
            rules: Vec::new(),
            schedule: ScheduleConfig::default(),
            metrics_path: None,
        }
    }
}

impl Config {
    /// Load configuration from a TOML file, expanding leading `~` in paths.
    pub fn load_from_path(path: &Path) -> Result<Self, WatchdogError> {
        let raw = fs::read_to_string(path).map_err(|source| WatchdogError::Io {
            path: Some(path.to_path_buf()),
            source,
        })?;
        let mut cfg: Config = toml::from_str(&raw).map_err(|err| WatchdogError::ConfigParse {
            path: path.to_path_buf(),
            message: err.to_string(),
        })?;
        cfg.expand_paths();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load from explicit path if given, else from `default_config_path()` if it exists,
    /// otherwise return `Config::default()`.
    pub fn load_or_default(
        explicit_path: Option<&Path>,
    ) -> Result<(Self, Option<PathBuf>), WatchdogError> {
        if let Some(path) = explicit_path {
            let expanded = expand_tilde(path);
            let cfg = Self::load_from_path(&expanded)?;
            return Ok((cfg, Some(expanded)));
        }

        if let Some(default_path) = default_config_path() {
            if default_path.is_file() {
                let cfg = Self::load_from_path(&default_path)?;
                return Ok((cfg, Some(default_path)));
            }
        }

        let mut cfg = Self::default();
        cfg.expand_paths();
        cfg.validate()?;
        Ok((cfg, None))
    }

    /// Expand leading `~` in `scan_roots`, `orphan_roots`, and `metrics_path`.
    pub fn expand_paths(&mut self) {
        self.scan_roots = self.scan_roots.iter().map(|p| expand_tilde(p)).collect();
        self.orphan_roots = self.orphan_roots.iter().map(|p| expand_tilde(p)).collect();
        if let Some(ref p) = self.metrics_path {
            self.metrics_path = Some(expand_tilde(p));
        }
    }

    /// Resolve the effective path for persisted lifetime metrics.
    pub fn effective_metrics_path(&self) -> Option<PathBuf> {
        self.metrics_path
            .clone()
            .or_else(crate::metrics::default_metrics_path)
    }

    /// Validate glob patterns and numeric bounds.
    pub fn validate(&self) -> Result<(), WatchdogError> {
        if self.max_scan_depth == 0 {
            return Err(WatchdogError::InvalidConfig(
                "max_scan_depth must be at least 1".to_string(),
            ));
        }
        if self.schedule.interval_hours == 0 {
            return Err(WatchdogError::InvalidConfig(
                "schedule.interval_hours must be at least 1".to_string(),
            ));
        }
        for pat in &self.exclude_paths {
            globset::Glob::new(pat).map_err(|e| {
                WatchdogError::InvalidConfig(format!("invalid exclude_paths glob '{pat}': {e}"))
            })?;
        }
        for pat in &self.exclude_branches {
            globset::Glob::new(pat).map_err(|e| {
                WatchdogError::InvalidConfig(format!("invalid exclude_branches glob '{pat}': {e}"))
            })?;
        }
        for rule in &self.rules {
            globset::Glob::new(&rule.match_path).map_err(|e| {
                WatchdogError::InvalidConfig(format!(
                    "invalid match_path glob '{}' in rule '{}': {e}",
                    rule.match_path, rule.name
                ))
            })?;
        }
        Ok(())
    }

    /// Compute the effective policy for a given worktree path by applying the first matching rule.
    pub fn effective_policy_for(
        &self,
        path: &Path,
    ) -> Result<(Policy, Option<&PathRule>), WatchdogError> {
        let path_str = path.to_string_lossy();
        for rule in &self.rules {
            let matcher = globset::Glob::new(&rule.match_path)
                .map_err(|e| WatchdogError::InvalidConfig(e.to_string()))?
                .compile_matcher();
            if matcher.is_match(path_str.as_ref()) {
                let mut policy = self.defaults.clone();
                if let Some(v) = rule.max_age_hours {
                    policy.max_age_hours = v;
                }
                if let Some(v) = rule.allow_dirty {
                    policy.allow_dirty = v;
                }
                if let Some(v) = rule.dirty_max_age_hours {
                    policy.dirty_max_age_hours = Some(v);
                }
                if let Some(v) = rule.allow_unpushed {
                    policy.allow_unpushed = v;
                }
                if let Some(v) = rule.allow_locked {
                    policy.allow_locked = v;
                }
                if let Some(v) = rule.delete_merged_branches {
                    policy.delete_merged_branches = v;
                }
                return Ok((policy, Some(rule)));
            }
        }
        Ok((self.defaults.clone(), None))
    }
}

/// Resolve the XDG/default config file path: `~/.config/mkd-worktree-watchdog/config.toml`.
pub fn default_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("mkd-worktree-watchdog/config.toml"));
        }
    }
    home_dir().map(|h| h.join(".config/mkd-worktree-watchdog/config.toml"))
}

/// Resolve the user's home directory from `HOME`.
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// Expand a leading `~` or `~/` to the user's home directory.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw == "~" {
        return home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Render a well-commented default configuration TOML file for `config init`.
pub fn render_default_toml() -> String {
    r#"# mkd-worktree-watchdog configuration
# SPDX-License-Identifier: MIT

# Directories scanned recursively for git repositories.
scan_roots = [
  "~/workspaces",
]

# Maximum directory depth under each scan_root when discovering git repos.
max_scan_depth = 3

# Parent directories where AI agents create standalone worktrees.
# Immediate child directories that are empty or have broken .git pointers are pruned as orphans.
orphan_roots = [
  "~/.gemini/antigravity-cli/worktrees",
  "~/.claude/worktrees",
  "~/.codex/worktrees",
  "~/.opencode/worktrees",
]

# Glob patterns for worktree or orphan paths that must NEVER be pruned.
exclude_paths = []

# Glob patterns for branch names that must NEVER be pruned.
exclude_branches = [
  "main",
  "master",
  "develop",
  "release/*",
]

[defaults]
# Minimum idle age (hours) before a clean, pushed/merged linked worktree is considered stale.
max_age_hours = 72

# Run `git worktree prune` on repositories with missing/prunable worktree metadata.
prune_missing = true

# Never prune worktrees with uncommitted changes by default.
allow_dirty = false

# Optional: uncomment to allow pruning dirty worktrees after a much longer idle period (e.g., 14 days).
# dirty_max_age_hours = 336

# Never prune worktrees with unpushed/unmerged commits by default.
allow_unpushed = false

# Never prune locked worktrees (`git worktree lock`) by default.
allow_locked = false

# Never prune a worktree while a running process has its cwd inside it.
skip_active_processes = true

# Delete local branches after pruning their stale worktree (only when already merged).
delete_merged_branches = false

# Remove orphaned directories under orphan_roots (empty or broken .git pointer).
prune_orphans = true

# Minimum idle age (hours) before removing an orphaned directory.
orphan_max_age_hours = 24

[schedule]
# Interval (hours) for built-in `daemon` mode (12 = twice daily).
interval_hours = 12

# Optional per-path rule overrides (first matching rule wins):
# [[rules]]
# name = "agy-ephemeral"
# match_path = "**/.gemini/antigravity-cli/worktrees/**"
# max_age_hours = 24
# allow_dirty = false
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses_and_validates() {
        let cfg: Config = toml::from_str(&render_default_toml()).expect("valid default toml");
        cfg.validate().expect("valid default config");
        assert_eq!(cfg.defaults.max_age_hours, 72);
        assert!(!cfg.defaults.allow_dirty);
        assert!(!cfg.defaults.allow_unpushed);
        assert_eq!(cfg.schedule.interval_hours, 12);
    }

    #[test]
    fn path_rule_override_applies_first_match() {
        let mut cfg = Config::default();
        cfg.rules.push(PathRule {
            name: "agent-fast".to_string(),
            match_path: "**/antigravity-cli/worktrees/**".to_string(),
            enabled: None,
            max_age_hours: Some(12),
            allow_dirty: None,
            dirty_max_age_hours: Some(168),
            allow_unpushed: None,
            allow_locked: None,
            delete_merged_branches: Some(true),
        });

        let (policy, matched) = cfg
            .effective_policy_for(Path::new(
                "/home/alice/.gemini/antigravity-cli/worktrees/abc123",
            ))
            .unwrap();
        assert_eq!(matched.map(|r| r.name.as_str()), Some("agent-fast"));
        assert_eq!(policy.max_age_hours, 12);
        assert_eq!(policy.dirty_max_age_hours, Some(168));
        assert!(policy.delete_merged_branches);
        assert!(!policy.allow_dirty);
    }
}
