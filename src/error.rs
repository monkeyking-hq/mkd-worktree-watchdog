// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Error types for `mkd-worktree-watchdog`.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Errors returned by watchdog operations.
#[derive(Debug)]
pub enum WatchdogError {
    /// Failed to read or write a file or directory.
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },
    /// Failed to parse TOML configuration.
    ConfigParse { path: PathBuf, message: String },
    /// Invalid configuration value or glob pattern.
    InvalidConfig(String),
    /// A `git` subprocess failed.
    GitCommand {
        args: Vec<String>,
        stderr: String,
        code: Option<i32>,
    },
}

impl WatchdogError {
    /// Machine-readable error code for structured CLI output.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "IO_ERROR",
            Self::ConfigParse { .. } => "CONFIG_PARSE_ERROR",
            Self::InvalidConfig(_) => "INVALID_CONFIG",
            Self::GitCommand { .. } => "GIT_COMMAND_ERROR",
        }
    }
}

impl fmt::Display for WatchdogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                path: Some(p),
                source,
            } => write!(f, "I/O error at {}: {source}", p.display()),
            Self::Io { path: None, source } => write!(f, "I/O error: {source}"),
            Self::ConfigParse { path, message } => {
                write!(f, "failed to parse config {}: {message}", path.display())
            }
            Self::InvalidConfig(msg) => write!(f, "invalid configuration: {msg}"),
            Self::GitCommand { args, stderr, code } => {
                let code_str = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                write!(
                    f,
                    "git {} failed (exit {code_str}): {}",
                    args.join(" "),
                    stderr.trim()
                )
            }
        }
    }
}

impl std::error::Error for WatchdogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
