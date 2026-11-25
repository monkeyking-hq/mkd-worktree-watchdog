# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT
#
# Remove the Windows Task Scheduler task for mkd-worktree-watchdog.

$ErrorActionPreference = "Stop"
$taskName = "MKDWorktreeWatchdog"

$cmd = Get-Command mkd-worktree-watchdog -ErrorAction SilentlyContinue
if ($cmd) {
    & $cmd.Source --human service uninstall
} else {
    schtasks /Delete /TN $taskName /F | Out-Null
    Write-Output "Removed Windows Scheduled Task '$taskName'."
}
