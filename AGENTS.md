---
hashseal: "blake3:bfc9c6ebb385e8eac24ef53e9c4fc50ab9193d4f7813216328ec46ebbabf441b"
---
<!--
SPDX-FileCopyrightText: 2026 Nathan A Chadwick
SPDX-License-Identifier: MIT
-->

# Agent guide: mkd-worktree-watchdog

## Read first

Read `README.md`, `Cargo.toml`, and the source relevant to the task. Follow `AGENTS.local.md` in the repository root if present; it contains local-only instructions and must remain untracked.

This project follows the [Universal Code](https://github.com/monkeyking-hq/universal-code/blob/main/UC-v1.0.0.md). Keep shared instructions self-contained; do not require private services, external AI APIs, sibling repositories, or machine-specific paths.

## Purpose and source map

`mkd-worktree-watchdog` is a standalone open-source Rust CLI, library, and scheduled watchdog service that safely prunes stale git worktrees, prunable git metadata, and orphaned agent worktree directories to reclaim disk space.

| Path | Responsibility |
| --- | --- |
| `src/config.rs` | TOML configuration schema, sane defaults, tilde expansion, glob validation, and per-path rule resolution |
| `src/git.rs` | Repo discovery, `git worktree list --porcelain` parsing, dirty/unpushed/age/size inspection, orphan discovery, and `/proc` active-process detection |
| `src/rules.rs` | Pure deterministic rule evaluation engine (`PruneWorktree`, `PruneMetadata`, `PruneOrphan`, `Keep`) |
| `src/lib.rs` | High-level `scan`, `prune`, and `run_with_time` orchestration and safe execution |
| `src/output.rs` | Compact/pretty JSON envelopes and human-readable CLI output |
| `src/main.rs` | CLI subcommands (`scan`, `prune`, `daemon`, `config init/show`) and signal handling |
| `contrib/systemd/` | Twice-daily systemd user service and timer units |
| `contrib/launchd/` | macOS launchd plist (12-hour interval) |
| `tests/watchdog_integration.rs` | End-to-end integration tests against real temporary git repositories |
| `tests/cli_output_contract.rs` | CLI JSON/human output contract verification |

## Safety invariants

- **Never** delete or modify a repository's main worktree (`is_main == true`).
- **Never** prune dirty, unpushed/unmerged, locked, or active-process worktrees unless explicitly opted into by config.
- **Never** delete branches with `git branch -D`; only `git branch -d` (merged-only) when `delete_merged_branches = true`.
- **Never** delete an orphan path outside the configured `orphan_roots` or equal to an orphan root itself.
- **Zero external AI dependencies**: all decisions must remain deterministic and rule-based.

## Build and verification

All Cargo compile, check, lint, test, and run commands **must use `--release`**. Do not create `target/debug`; remove it immediately if accidentally created.

```bash
cargo fmt --all -- --check
cargo clippy --release --workspace --all-features --all-targets -- -D warnings
cargo check --release --workspace --all-targets
cargo test --release --workspace -- --quiet
```

## Change workflow

Work on a feature branch, keep changes focused, and preserve unrelated edits. Code-producing changes require a SemVer bump in `Cargo.toml`, a matching `Cargo.lock` update, and a versioned `CHANGELOG.md` entry.
