#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT
#
# Uninstall the macOS launchd LaunchAgent for mkd-worktree-watchdog.
set -euo pipefail

BIN="${1:-$(command -v mkd-worktree-watchdog || echo "$HOME/.cargo/bin/mkd-worktree-watchdog")}"
if [[ -x "$BIN" ]]; then
  "$BIN" --human service uninstall
else
  PLIST="$HOME/Library/LaunchAgents/dev.monkeyking.worktree-watchdog.plist"
  launchctl unload -w "$PLIST" 2>/dev/null || true
  rm -f "$PLIST"
  echo "Removed $PLIST"
fi
