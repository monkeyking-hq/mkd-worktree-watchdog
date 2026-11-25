# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT
#
# Install the twice-daily Windows Task Scheduler task (06:00 & 18:00) for mkd-worktree-watchdog.

param(
    [string]$BinaryPath = "",
    [string]$ConfigPath = ""
)

$ErrorActionPreference = "Stop"

if (-not $BinaryPath) {
    $cmd = Get-Command mkd-worktree-watchdog -ErrorAction SilentlyContinue
    if ($cmd) {
        $BinaryPath = $cmd.Source
    } else {
        $BinaryPath = Join-Path $env:USERPROFILE ".cargo\bin\mkd-worktree-watchdog.exe"
    }
}

if (-not (Test-Path $BinaryPath)) {
    throw "mkd-worktree-watchdog binary not found at $BinaryPath. Run 'cargo install --path .' first."
}

$argsList = @("--human")
if ($ConfigPath) {
    $argsList += @("--config", $ConfigPath)
}
$argsList += @("service", "install")

& $BinaryPath @argsList
