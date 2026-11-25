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

# 3. Prune stale worktrees and reclaim disk space
mkd-worktree-watchdog --human prune

# Or preview prune explicitly
mkd-worktree-watchdog --human prune --dry-run
```

## Running on a schedule (twice daily)

### Option A: systemd user timer (Linux, recommended)

```bash
mkdir -p ~/.config/systemd/user
cp contrib/systemd/mkd-worktree-watchdog.service ~/.config/systemd/user/
cp contrib/systemd/mkd-worktree-watchdog.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now mkd-worktree-watchdog.timer
systemctl --user list-timers mkd-worktree-watchdog.timer
```

The default timer runs twice daily at **06:00** and **18:00** (`RandomizedDelaySec=15m`, `Persistent=true`).

### Option B: Built-in daemon mode (any OS / container)

```bash
mkd-worktree-watchdog daemon --interval-hours 12
```

### Option C: macOS launchd

Copy `contrib/launchd/dev.monkeyking.worktree-watchdog.plist` to `~/Library/LaunchAgents/`, adjust the binary path if needed, and load it with `launchctl load`.

## Configuration

Configuration lives at `~/.config/mkd-worktree-watchdog/config.toml` (or pass `--config <path>`). Run `mkd-worktree-watchdog config init` to generate the full commented template, or `mkd-worktree-watchdog --human config show` to inspect the active resolved config.

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
