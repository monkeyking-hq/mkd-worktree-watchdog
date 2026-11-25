// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

mod output;

use clap::{Parser, Subcommand};
use mkd_worktree_watchdog::config::{default_config_path, render_default_toml, Config};
use mkd_worktree_watchdog::error::WatchdogError;
use mkd_worktree_watchdog::metrics::LifetimeMetrics;
use mkd_worktree_watchdog::service::{install_service, service_status, uninstall_service};
use mkd_worktree_watchdog::{prune, scan};
use output::{
    print_data, print_error, print_lifetime_metrics, print_report, print_service_status,
    OutputFormat,
};
use serde::Serialize;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(
    name = "mkd-worktree-watchdog",
    version,
    about = "Rule-based watchdog service that safely prunes stale git worktrees and reclaims disk space"
)]
struct Cli {
    /// Path to TOML configuration file (defaults to ~/.config/mkd-worktree-watchdog/config.toml).
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Override path to persisted lifetime metrics JSON file.
    #[arg(long, global = true)]
    metrics_path: Option<PathBuf>,

    /// Output format (compact JSON by default).
    #[arg(short = 'F', long, value_enum, global = true, default_value_t = OutputFormat::Compact)]
    format: OutputFormat,

    /// Shorthand for --format human.
    #[arg(long, global = true, conflicts_with = "format")]
    human: bool,

    /// Shorthand for --format json (pretty JSON).
    #[arg(
        long,
        global = true,
        conflicts_with = "format",
        conflicts_with = "human"
    )]
    pretty: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scan repositories and orphan roots and report what would be pruned (read-only).
    Scan {
        /// Additional directories to scan for git repositories (overrides config scan_roots if given).
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
    },
    /// Prune stale linked worktrees, missing worktree metadata, and orphan directories.
    Prune {
        /// Preview actions without deleting anything.
        #[arg(long)]
        dry_run: bool,
        /// Additional directories to scan for git repositories (overrides config scan_roots if given).
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
    },
    /// Run continuously on a schedule (default: every 12 hours, i.e. twice daily).
    Daemon {
        /// Override schedule interval in hours.
        #[arg(long)]
        interval_hours: Option<u64>,
        /// Run in dry-run mode (log what would be pruned without deleting).
        #[arg(long)]
        dry_run: bool,
    },
    /// Show persisted lifetime metrics (GB reclaimed, run count, pruned totals).
    Metrics,
    /// Install, uninstall, or inspect the native OS scheduled service (systemd / launchd / Windows Task Scheduler).
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Manage watchdog configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    /// Install and enable the twice-daily scheduled service for this OS.
    Install,
    /// Disable and remove the scheduled service for this OS.
    Uninstall,
    /// Show scheduled service status for this OS.
    Status,
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Write a commented default config.toml file.
    Init {
        /// Output path (defaults to ~/.config/mkd-worktree-watchdog/config.toml).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite existing file if present.
        #[arg(long)]
        force: bool,
    },
    /// Show the effective resolved configuration.
    Show,
}

#[derive(Serialize)]
struct ConfigInitResult {
    path: PathBuf,
    created: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = if cli.human {
        OutputFormat::Human
    } else if cli.pretty {
        OutputFormat::Json
    } else {
        cli.format
    };

    match run(cli, format) {
        Ok(code) => code,
        Err(err) => {
            let _ = print_error(err.to_string(), err.error_code(), format);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, format: OutputFormat) -> Result<ExitCode, WatchdogError> {
    match cli.command {
        Commands::Scan { roots } => {
            let (mut cfg, cfg_path) = Config::load_or_default(cli.config.as_deref())?;
            if let Some(mp) = cli.metrics_path {
                cfg.metrics_path = Some(mp);
            }
            if !roots.is_empty() {
                cfg.scan_roots = roots;
            }
            cfg.expand_paths();
            let report = scan(&cfg, cfg_path)?;
            print_report(&report, format)
                .map_err(|source| WatchdogError::Io { path: None, source })?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Prune { dry_run, roots } => {
            let (mut cfg, cfg_path) = Config::load_or_default(cli.config.as_deref())?;
            if let Some(mp) = cli.metrics_path {
                cfg.metrics_path = Some(mp);
            }
            if !roots.is_empty() {
                cfg.scan_roots = roots;
            }
            cfg.expand_paths();
            let report = prune(&cfg, cfg_path, dry_run)?;
            let has_errors = !report.errors.is_empty();
            print_report(&report, format)
                .map_err(|source| WatchdogError::Io { path: None, source })?;
            Ok(if has_errors {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Daemon {
            interval_hours,
            dry_run,
        } => {
            let (mut cfg, cfg_path) = Config::load_or_default(cli.config.as_deref())?;
            if let Some(ref mp) = cli.metrics_path {
                cfg.metrics_path = Some(mp.clone());
                cfg.expand_paths();
            }
            let hours = interval_hours.unwrap_or(cfg.schedule.interval_hours);
            if hours == 0 {
                return Err(WatchdogError::InvalidConfig(
                    "interval_hours must be at least 1".to_string(),
                ));
            }
            let interval = Duration::from_secs(hours * 3600);
            let shutdown = Arc::new(AtomicBool::new(false));
            let shutdown_flag = Arc::clone(&shutdown);
            let _ = ctrlc::set_handler(move || {
                shutdown_flag.store(true, Ordering::Relaxed);
            });

            loop {
                // Reload config each cycle so rule changes take effect without restarting.
                let (mut live_cfg, live_path) = match Config::load_or_default(cli.config.as_deref())
                {
                    Ok(pair) => pair,
                    Err(_) => (cfg.clone(), cfg_path.clone()),
                };
                if let Some(ref mp) = cli.metrics_path {
                    live_cfg.metrics_path = Some(mp.clone());
                    live_cfg.expand_paths();
                }
                match prune(&live_cfg, live_path, dry_run) {
                    Ok(report) => {
                        let _ = print_report(&report, format);
                    }
                    Err(err) => {
                        let _ = print_error(err.to_string(), err.error_code(), format);
                    }
                }
                let _ = io::stdout().flush();

                let deadline = Instant::now() + interval;
                while Instant::now() < deadline {
                    if shutdown.load(Ordering::Relaxed) {
                        return Ok(ExitCode::SUCCESS);
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                if shutdown.load(Ordering::Relaxed) {
                    return Ok(ExitCode::SUCCESS);
                }
            }
        }
        Commands::Metrics => {
            let (mut cfg, _) = Config::load_or_default(cli.config.as_deref())?;
            if let Some(mp) = cli.metrics_path {
                cfg.metrics_path = Some(mp);
                cfg.expand_paths();
            }
            let metrics_path = cfg.effective_metrics_path();
            let metrics = match metrics_path.as_deref() {
                Some(path) => LifetimeMetrics::load_or_default(path)?,
                None => LifetimeMetrics::default(),
            };
            print_lifetime_metrics(&metrics, metrics_path.as_deref(), format)
                .map_err(|source| WatchdogError::Io { path: None, source })?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Service { action } => match action {
            ServiceAction::Install => {
                let exe = env::current_exe()
                    .map_err(|source| WatchdogError::Io { path: None, source })?;
                let status = install_service(&exe, cli.config.as_deref())?;
                print_service_status("Service installed", &status, format)
                    .map_err(|source| WatchdogError::Io { path: None, source })?;
                Ok(ExitCode::SUCCESS)
            }
            ServiceAction::Uninstall => {
                let status = uninstall_service()?;
                print_service_status("Service uninstalled", &status, format)
                    .map_err(|source| WatchdogError::Io { path: None, source })?;
                Ok(ExitCode::SUCCESS)
            }
            ServiceAction::Status => {
                let status = service_status()?;
                print_service_status("Service status", &status, format)
                    .map_err(|source| WatchdogError::Io { path: None, source })?;
                Ok(ExitCode::SUCCESS)
            }
        },
        Commands::Config { action } => match action {
            ConfigAction::Init { output, force } => {
                let target = output.or_else(default_config_path).ok_or_else(|| {
                    WatchdogError::InvalidConfig(
                        "could not determine default config path (HOME unset); pass --output"
                            .to_string(),
                    )
                })?;
                if target.exists() && !force {
                    return Err(WatchdogError::InvalidConfig(format!(
                        "config file already exists at {} (pass --force to overwrite)",
                        target.display()
                    )));
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|source| WatchdogError::Io {
                        path: Some(parent.to_path_buf()),
                        source,
                    })?;
                }
                fs::write(&target, render_default_toml()).map_err(|source| WatchdogError::Io {
                    path: Some(target.clone()),
                    source,
                })?;

                match format {
                    OutputFormat::Human => {
                        writeln!(io::stdout(), "Wrote default config to {}", target.display())
                            .map_err(|source| WatchdogError::Io { path: None, source })?;
                    }
                    OutputFormat::Compact | OutputFormat::Json => {
                        let res = ConfigInitResult {
                            path: target,
                            created: true,
                        };
                        print_data("Config initialized", &res, format)
                            .map_err(|source| WatchdogError::Io { path: None, source })?;
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            ConfigAction::Show => {
                let (cfg, _) = Config::load_or_default(cli.config.as_deref())?;
                match format {
                    OutputFormat::Human => {
                        let rendered = toml::to_string_pretty(&cfg)
                            .map_err(|e| WatchdogError::InvalidConfig(e.to_string()))?;
                        write!(io::stdout(), "{rendered}")
                            .map_err(|source| WatchdogError::Io { path: None, source })?;
                    }
                    OutputFormat::Compact | OutputFormat::Json => {
                        print_data("Effective configuration", &cfg, format)
                            .map_err(|source| WatchdogError::Io { path: None, source })?;
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
        },
    }
}
