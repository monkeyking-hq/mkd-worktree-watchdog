#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT
#
# Install the twice-daily macOS launchd LaunchAgent for mkd-worktree-watchdog.
set -euo pipefail

BIN="${1:-$(command -v mkd-worktree-watchdog || echo "$HOME/.cargo/bin/mkd-worktree-watchdog")}"
if [[ ! -x "$BIN" ]]; then
  echo "Error: mkd-worktree-watchdog binary not found at $BIN" >&2
  exit 1
fi

"$BIN" --human service install
