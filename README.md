<!--
SPDX-FileCopyrightText: 2026 Nathan A Chadwick

SPDX-License-Identifier: MIT
-->

# mkd-worktree-watchdog

A lightweight, zero-AI, rule-based watchdog service and CLI that safely prunes stale `git` worktrees, missing worktree metadata, and orphaned agent worktree directories to reclaim disk space automatically.

Built for developer machines and CI/agent hosts where coding agents (Antigravity CLI / `agy`, Claude Code, Codex, Kilo, etc.) create ephemeral git worktrees that accumulate build artifacts (`target/`, `node_modules/`, `.venv/`) and exhaust disk space.

This project is open source under the [MIT License](LICENSE) and follows the [Universal Code v1.0.0](https://github.com/monkeyking-hq/universal-code/blob/main/UC-v1.0.0.md).

## Safety first (sane defaults)

By default, `mkd-worktree-watchdog` **never**:

- Touches the main repository checkout (only *linked* worktrees).
- Prunes a worktree with uncommitted changes (staged, unstaged, or untracked).
- Prunes a worktree with unpushed or unmerged commits.
- Prunes a locked worktree (`git worktree lock`).
- Prunes a worktree while a running process has its working directory inside it (Linux `/proc`).
- Prunes worktrees on protected branches (`main`, `master`, `develop`, `release/*`).
- Prunes worktrees younger than `72` hours (3 days).

What it **does** clean by default:

1. **Stale clean worktrees**: Linked worktrees idle for `>= 72h` with no uncommitted changes, no unpushed/unmerged commits, not locked, and no active process inside them.
2. **Missing/prunable git metadata**: Runs `git worktree prune` on repos whose linked worktree directories were already deleted (e.g. after `/tmp` cleanup or reboot).
3. **Orphaned agent worktree directories**: Empty directories or directories with broken `.git` pointers under configured `orphan_roots` (such as `~/.gemini/antigravity-cli/worktrees`) idle for `>= 24h`.

## Installation

```bash
cargo install --path .
```

## Quick start

```bash
# 1. Generate a commented default configuration at ~/.config/mkd-worktree-watchdog/config.toml
mkd-worktree-watchdog --human config init

# 2. Preview what would be pruned (read-only)
mkd-worktree-watchdog --human scan

# 3. Prune stale worktrees and reclaim disk space (outputs run + lifetime GB reclaimed)
mkd-worktree-watchdog --human prune

# Or preview prune explicitly
mkd-worktree-watchdog --human prune --dry-run

# 4. Inspect lifetime metrics anytime
mkd-worktree-watchdog --human metrics
```

## Run & lifetime metrics

- Every `scan` and `prune` output includes both **run metrics** (duration, GB/bytes reclaimed or reclaimable) and **lifetime metrics** (cumulative runs, total GB reclaimed, and total pruned counts).
- Live `prune` runs atomically update `~/.local/state/mkd-worktree-watchdog/metrics.json` (respecting `$XDG_STATE_HOME` or `metrics_path` / `--metrics-path`) and append a structured JSONL audit entry to `runs.jsonl` in the same directory.
- Automated `systemd` runs also log run and lifetime metrics directly to `journald` (`journalctl --user -u mkd-worktree-watchdog.service`).
- `scan` and `--dry-run` read lifetime totals for display without modifying state.

## Running on a schedule (twice daily)

### Quickest: built-in cross-platform service command (Linux, macOS, Windows)

```bash
# Install and enable the native twice-daily service (06:00 & 18:00)
mkd-worktree-watchdog --human service install

# Check service status
mkd-worktree-watchdog --human service status

# Uninstall
mkd-worktree-watchdog --human service uninstall
```

What `service install` configures on each OS:

| Platform | Service Manager | Schedule | Logs / State |
| --- | --- | --- | --- |
| **Linux** | `systemd` user timer (`mkd-worktree-watchdog.timer`) | `06:00` & `18:00` (`Persistent=true`) | `journalctl --user -u mkd-worktree-watchdog.service` + `~/.local/state/mkd-worktree-watchdog/` |
| **macOS (OS X)** | `launchd` LaunchAgent (`dev.monkeyking.worktree-watchdog`) | `06:00` & `18:00` (`StartCalendarInterval`) | `~/Library/Logs/mkd-worktree-watchdog.log` + `~/.local/state/mkd-worktree-watchdog/` |
| **Windows** | Task Scheduler (`MKDWorktreeWatchdog`) | `06:00` & `18:00` (`StartWhenAvailable=true`, hidden) | `%LOCALAPPDATA%\mkd-worktree-watchdog\` (`metrics.json` + `runs.jsonl`) |

### Manual / script-based deployment

- **Linux (`systemd`)**: unit templates in `contrib/systemd/`
- **macOS (`launchd`)**: plist + helper scripts in `contrib/launchd/` (`install.sh` / `uninstall.sh`)
- **Windows (Task Scheduler)**: PowerShell scripts in `contrib/windows/` (`install-task.ps1` / `uninstall-task.ps1`)
- **Any OS / container (built-in daemon)**: `mkd-worktree-watchdog daemon --interval-hours 12`

## Configuration

Default configuration file locations (or pass `--config <path>`):

- **Linux / macOS**: `$XDG_CONFIG_HOME/mkd-worktree-watchdog/config.toml` (default: `~/.config/mkd-worktree-watchdog/config.toml`)
- **Windows**: `%APPDATA%\mkd-worktree-watchdog\config.toml` (fallback: `%USERPROFILE%\.config\mkd-worktree-watchdog\config.toml`)

Run `mkd-worktree-watchdog config init` to generate the full commented template, or `mkd-worktree-watchdog --human config show` to inspect the active resolved config.

### Per-path rule overrides

You can apply tighter or looser policies to specific agent worktree locations using `[[rules]]` (first matching glob wins):

```toml
[[rules]]
name = "agy-ephemeral"
match_path = "**/.gemini/antigravity-cli/worktrees/**"
max_age_hours = 24
allow_dirty = false

[[rules]]
name = "keep-long-running"
match_path = "**/workspaces/critical/**"
enabled = false
```

## Machine-first CLI output

Following the MKD CLI output contract, commands emit a single-line JSON envelope by default (`result: "OK" | "ERROR"`, `message`, `data`):

- Default: compact single-line JSON (`--format compact`)
- Pretty JSON: `--pretty` or `--format json`
- Human-readable: `--human` or `--format human`

## License

MIT — see [LICENSE](LICENSE).
