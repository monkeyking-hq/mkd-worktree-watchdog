<!--
SPDX-FileCopyrightText: 2026 Nathan A Chadwick

SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to `mkd-worktree-watchdog` are documented in this file.

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
