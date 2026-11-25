<!--
SPDX-FileCopyrightText: 2026 Nathan A Chadwick

SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to `mkd-worktree-watchdog` are documented in this file.

## [0.3.0] - 2026-11-25

### Documentation (2026-11-25)
- Documented Windows default configuration path (`%APPDATA%\mkd-worktree-watchdog\config.toml`, fallback `%USERPROFILE%\.config\mkd-worktree-watchdog\config.toml`) in `README.md`.

### Added
- Native cross-platform service deployment via `mkd-worktree-watchdog service install|uninstall|status`:
  - **Linux**: `systemd` user service + twice-daily timer (`06:00` and `18:00`, `Persistent=true`).
  - **macOS (OS X)**: `launchd` LaunchAgent plist (`StartCalendarInterval` at `06:00` and `18:00`, logs to `~/Library/Logs/mkd-worktree-watchdog.log`).
  - **Windows**: Task Scheduler task `MKDWorktreeWatchdog` (twice daily at `06:00` and `18:00`, `StartWhenAvailable=true`, runs hidden).
- Standalone deployment scripts under `contrib/launchd/` (`install.sh`, `uninstall.sh`) and `contrib/windows/` (`install-task.ps1`, `uninstall-task.ps1`).
- Full Windows & macOS runtime compatibility: `%APPDATA%` / `%LOCALAPPDATA%` / `%USERPROFILE%` path resolution, cross-platform `~/` and `~\` tilde expansion, normalized glob matching for Windows backslashes, Windows-safe atomic metrics file replacement, and cross-platform signal handling via `ctrlc`.

## [0.2.0] - 2026-11-25

### Added
- Per-run metrics (`run_metrics`: duration, bytes and GB reclaimed/reclaimable) and persisted lifetime metrics (`lifetime_metrics`: cumulative runs, GB/bytes reclaimed, and pruned totals).
- Automatic atomic persistence to `$XDG_STATE_HOME/mkd-worktree-watchdog/metrics.json` (default `~/.local/state/mkd-worktree-watchdog/metrics.json`, overridable via `metrics_path` in config or `--metrics-path`) on live prune runs; dry-runs and `scan` remain read-only.
- Append-only JSONL run log (`runs.jsonl` alongside `metrics.json`) recording every live/automated run's metrics alongside `journald` output.
- New `metrics` CLI subcommand to inspect lifetime GB reclaimed in JSON or `--human` format.

## [0.1.0] - 2026-11-25

### Added
- Initial open-source release of `mkd-worktree-watchdog` (MIT licensed, zero external AI dependencies).
- Deterministic, configurable TOML ruleset with sane safety defaults (`max_age_hours = 72`, protects dirty, unpushed/unmerged, locked, active-process, and main/release worktrees).
- Automatic discovery of git repositories under `scan_roots` and orphaned agent worktree directories under `orphan_roots` (empty directories or broken `.git` pointers).
- Per-path glob rule overrides (`[[rules]]`) and branch/path exclusions (`exclude_paths`, `exclude_branches`).
- CLI subcommands: `scan`, `prune` (with `--dry-run`), `daemon` (default 12h / twice-daily schedule), `config init`, and `config show`.
- Machine-first JSON output contract (`--format compact` default, `--pretty` / `--format json`, `--human` / `--format human`).
- Twice-daily `systemd` user timer/service units (`contrib/systemd/`) and macOS `launchd` plist (`contrib/launchd/`).
- Unit tests and end-to-end integration tests with real git repositories.
