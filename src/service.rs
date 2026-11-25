// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

//! Cross-platform native service deployment for Linux (systemd), macOS (launchd),
//! and Windows (Task Scheduler).

use crate::error::WatchdogError;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// macOS launchd service label.
pub const LAUNCHD_LABEL: &str = "dev.monkeyking.worktree-watchdog";
/// Windows Task Scheduler task name.
pub const WINDOWS_TASK_NAME: &str = "MKDWorktreeWatchdog";
/// Linux systemd unit base name.
pub const SYSTEMD_UNIT_NAME: &str = "mkd-worktree-watchdog";

/// Structured result for `service install`, `service uninstall`, and `service status`.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatus {
    pub platform: &'static str,
    pub manager: &'static str,
    pub installed: bool,
    pub active: bool,
    pub schedule: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unit_paths: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Render the Linux systemd user `.service` unit file.
pub fn render_systemd_service(exe: &Path, config: Option<&Path>) -> String {
    let mut exec = format!("{} --human prune", exe.display());
    if let Some(cfg) = config {
        exec = format!("{} --human --config {} prune", exe.display(), cfg.display());
    }
    format!(
        "\
# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT

[Unit]
Description=Prune stale git worktrees and reclaim disk space (mkd-worktree-watchdog)

[Service]
Type=oneshot
ExecStart={exec}
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
"
    )
}

/// Render the Linux systemd user `.timer` unit file (twice daily: 06:00 and 18:00).
pub fn render_systemd_timer() -> String {
    "\
# SPDX-FileCopyrightText: 2026 Nathan A Chadwick
# SPDX-License-Identifier: MIT

[Unit]
Description=Run mkd-worktree-watchdog twice daily to prune stale git worktrees

[Timer]
OnCalendar=*-*-* 06,18:00:00
RandomizedDelaySec=15m
Persistent=true

[Install]
WantedBy=timers.target
"
    .to_string()
}

/// Render a macOS `launchd` LaunchAgent plist scheduled twice daily (06:00 and 18:00).
pub fn render_launchd_plist(exe: &Path, config: Option<&Path>, log_path: &Path) -> String {
    let exe_xml = xml_escape(&exe.to_string_lossy());
    let log_xml = xml_escape(&log_path.to_string_lossy());
    let config_args = match config {
        Some(cfg) => format!(
            "    <string>--config</string>\n    <string>{}</string>\n",
            xml_escape(&cfg.to_string_lossy())
        ),
        None => String::new(),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_xml}</string>
    <string>--human</string>
{config_args}    <string>prune</string>
  </array>
  <key>StartCalendarInterval</key>
  <array>
    <dict>
      <key>Hour</key>
      <integer>6</integer>
      <key>Minute</key>
      <integer>0</integer>
    </dict>
    <dict>
      <key>Hour</key>
      <integer>18</integer>
      <key>Minute</key>
      <integer>0</integer>
    </dict>
  </array>
  <key>StandardOutPath</key>
  <string>{log_xml}</string>
  <key>StandardErrorPath</key>
  <string>{log_xml}</string>
  <key>ProcessType</key>
  <string>Background</string>
  <key>LowPriorityIO</key>
  <true/>
</dict>
</plist>
"#
    )
}

/// Render a Windows Task Scheduler XML definition scheduled twice daily (06:00 and 18:00),
/// with `StartWhenAvailable` enabled so missed runs catch up after wake/login.
pub fn render_windows_task_xml(exe: &Path, config: Option<&Path>) -> String {
    let exe_xml = xml_escape(&exe.to_string_lossy());
    let args = match config {
        Some(cfg) => format!(
            "--human --config \"{}\" prune",
            xml_escape(&cfg.to_string_lossy())
        ),
        None => "--human prune".to_string(),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Prune stale git worktrees and reclaim disk space (mkd-worktree-watchdog)</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2026-01-01T06:00:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
    <CalendarTrigger>
      <StartBoundary>2026-01-01T18:00:00</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
  </Triggers>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <ExecutionTimeLimit>PT30M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe_xml}</Command>
      <Arguments>{args}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Install and enable the native twice-daily scheduled service for the current OS.
pub fn install_service(exe: &Path, config: Option<&Path>) -> Result<ServiceStatus, WatchdogError> {
    #[cfg(target_os = "linux")]
    {
        linux::install(exe, config)
    }
    #[cfg(target_os = "macos")]
    {
        macos::install(exe, config)
    }
    #[cfg(target_os = "windows")]
    {
        windows::install(exe, config)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (exe, config);
        Err(WatchdogError::InvalidConfig(
            "native service installation is supported on Linux, macOS, and Windows; use `daemon` mode on other platforms".to_string(),
        ))
    }
}

/// Disable and remove the native scheduled service for the current OS.
pub fn uninstall_service() -> Result<ServiceStatus, WatchdogError> {
    #[cfg(target_os = "linux")]
    {
        linux::uninstall()
    }
    #[cfg(target_os = "macos")]
    {
        macos::uninstall()
    }
    #[cfg(target_os = "windows")]
    {
        windows::uninstall()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(WatchdogError::InvalidConfig(
            "native service management is supported on Linux, macOS, and Windows".to_string(),
        ))
    }
}

/// Query installation and active status of the native scheduled service for the current OS.
pub fn service_status() -> Result<ServiceStatus, WatchdogError> {
    #[cfg(target_os = "linux")]
    {
        linux::status()
    }
    #[cfg(target_os = "macos")]
    {
        macos::status()
    }
    #[cfg(target_os = "windows")]
    {
        windows::status()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(WatchdogError::InvalidConfig(
            "native service management is supported on Linux, macOS, and Windows".to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::config::home_dir;
    use std::fs;
    use std::process::Command;

    fn unit_paths() -> Result<(PathBuf, PathBuf), WatchdogError> {
        let base = home_dir()
            .ok_or_else(|| WatchdogError::InvalidConfig("HOME is not set".to_string()))?
            .join(".config/systemd/user");
        Ok((
            base.join(format!("{SYSTEMD_UNIT_NAME}.service")),
            base.join(format!("{SYSTEMD_UNIT_NAME}.timer")),
        ))
    }

    pub fn install(exe: &Path, config: Option<&Path>) -> Result<ServiceStatus, WatchdogError> {
        let (service_path, timer_path) = unit_paths()?;
        if let Some(parent) = service_path.parent() {
            fs::create_dir_all(parent).map_err(|source| WatchdogError::Io {
                path: Some(parent.to_path_buf()),
                source,
            })?;
        }
        fs::write(&service_path, render_systemd_service(exe, config)).map_err(|source| {
            WatchdogError::Io {
                path: Some(service_path.clone()),
                source,
            }
        })?;
        fs::write(&timer_path, render_systemd_timer()).map_err(|source| WatchdogError::Io {
            path: Some(timer_path.clone()),
            source,
        })?;

        run_systemctl(&["--user", "daemon-reload"])?;
        run_systemctl(&[
            "--user",
            "enable",
            "--now",
            &format!("{SYSTEMD_UNIT_NAME}.timer"),
        ])?;

        status()
    }

    pub fn uninstall() -> Result<ServiceStatus, WatchdogError> {
        let (service_path, timer_path) = unit_paths()?;
        let _ = run_systemctl(&[
            "--user",
            "disable",
            "--now",
            &format!("{SYSTEMD_UNIT_NAME}.timer"),
        ]);
        if timer_path.exists() {
            let _ = fs::remove_file(&timer_path);
        }
        if service_path.exists() {
            let _ = fs::remove_file(&service_path);
        }
        let _ = run_systemctl(&["--user", "daemon-reload"]);

        Ok(ServiceStatus {
            platform: "linux",
            manager: "systemd-user",
            installed: false,
            active: false,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: vec![service_path, timer_path],
            detail: Some("uninstalled".to_string()),
        })
    }

    pub fn status() -> Result<ServiceStatus, WatchdogError> {
        let (service_path, timer_path) = unit_paths()?;
        let installed = service_path.is_file() && timer_path.is_file();
        let active = Command::new("systemctl")
            .args(["--user", "is-active", &format!("{SYSTEMD_UNIT_NAME}.timer")])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let detail = Command::new("systemctl")
            .args([
                "--user",
                "list-timers",
                "--no-legend",
                "--all",
                &format!("{SYSTEMD_UNIT_NAME}.timer"),
            ])
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                (!s.is_empty()).then_some(s)
            });

        Ok(ServiceStatus {
            platform: "linux",
            manager: "systemd-user",
            installed,
            active,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: vec![service_path, timer_path],
            detail,
        })
    }

    fn run_systemctl(args: &[&str]) -> Result<(), WatchdogError> {
        let out = Command::new("systemctl")
            .args(args)
            .output()
            .map_err(|source| WatchdogError::Io { path: None, source })?;
        if !out.status.success() {
            return Err(WatchdogError::InvalidConfig(format!(
                "systemctl {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use crate::config::home_dir;
    use std::fs;
    use std::process::Command;

    fn paths() -> Result<(PathBuf, PathBuf), WatchdogError> {
        let home = home_dir()
            .ok_or_else(|| WatchdogError::InvalidConfig("HOME is not set".to_string()))?;
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"));
        let log = home.join("Library/Logs/mkd-worktree-watchdog.log");
        Ok((plist, log))
    }

    pub fn install(exe: &Path, config: Option<&Path>) -> Result<ServiceStatus, WatchdogError> {
        let (plist_path, log_path) = paths()?;
        if let Some(parent) = plist_path.parent() {
            fs::create_dir_all(parent).map_err(|source| WatchdogError::Io {
                path: Some(parent.to_path_buf()),
                source,
            })?;
        }
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&plist_path, render_launchd_plist(exe, config, &log_path)).map_err(|source| {
            WatchdogError::Io {
                path: Some(plist_path.clone()),
                source,
            }
        })?;

        let _ = Command::new("launchctl")
            .args(["unload", "-w", &plist_path.to_string_lossy()])
            .output();
        let out = Command::new("launchctl")
            .args(["load", "-w", &plist_path.to_string_lossy()])
            .output()
            .map_err(|source| WatchdogError::Io { path: None, source })?;
        if !out.status.success() {
            return Err(WatchdogError::InvalidConfig(format!(
                "launchctl load failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        status()
    }

    pub fn uninstall() -> Result<ServiceStatus, WatchdogError> {
        let (plist_path, _) = paths()?;
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &plist_path.to_string_lossy()])
            .output();
        if plist_path.exists() {
            let _ = fs::remove_file(&plist_path);
        }
        Ok(ServiceStatus {
            platform: "macos",
            manager: "launchd",
            installed: false,
            active: false,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: vec![plist_path],
            detail: Some("uninstalled".to_string()),
        })
    }

    pub fn status() -> Result<ServiceStatus, WatchdogError> {
        let (plist_path, _) = paths()?;
        let installed = plist_path.is_file();
        let out = Command::new("launchctl")
            .args(["list", LAUNCHD_LABEL])
            .output();
        let active = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
        let detail = out.ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });
        Ok(ServiceStatus {
            platform: "macos",
            manager: "launchd",
            installed,
            active,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: vec![plist_path],
            detail,
        })
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::env;
    use std::fs;
    use std::process::Command;

    fn task_xml_path() -> PathBuf {
        env::temp_dir().join("mkd-worktree-watchdog-task.xml")
    }

    pub fn install(exe: &Path, config: Option<&Path>) -> Result<ServiceStatus, WatchdogError> {
        let xml_path = task_xml_path();
        let xml = render_windows_task_xml(exe, config);
        // Windows schtasks /XML expects UTF-16LE with BOM when encoding="UTF-16".
        let mut bytes = vec![0xFFu8, 0xFEu8];
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&xml_path, bytes).map_err(|source| WatchdogError::Io {
            path: Some(xml_path.clone()),
            source,
        })?;

        let out = Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                WINDOWS_TASK_NAME,
                "/XML",
                &xml_path.to_string_lossy(),
                "/F",
            ])
            .output()
            .map_err(|source| WatchdogError::Io { path: None, source })?;
        let _ = fs::remove_file(&xml_path);

        if !out.status.success() {
            return Err(WatchdogError::InvalidConfig(format!(
                "schtasks /Create failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        status()
    }

    pub fn uninstall() -> Result<ServiceStatus, WatchdogError> {
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"])
            .output();
        Ok(ServiceStatus {
            platform: "windows",
            manager: "task-scheduler",
            installed: false,
            active: false,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: Vec::new(),
            detail: Some("uninstalled".to_string()),
        })
    }

    pub fn status() -> Result<ServiceStatus, WatchdogError> {
        let out = Command::new("schtasks")
            .args(["/Query", "/TN", WINDOWS_TASK_NAME, "/FO", "LIST"])
            .output();
        let installed = out.as_ref().map(|o| o.status.success()).unwrap_or(false);
        let detail = out.ok().and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });
        let active = detail
            .as_deref()
            .map(|d| d.contains("Ready") || d.contains("Running"))
            .unwrap_or(false);
        Ok(ServiceStatus {
            platform: "windows",
            manager: "task-scheduler",
            installed,
            active,
            schedule: "twice daily (06:00, 18:00)".to_string(),
            unit_paths: Vec::new(),
            detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_systemd_units_with_valid_calendar() {
        let svc = render_systemd_service(
            Path::new("/home/alice/.cargo/bin/mkd-worktree-watchdog"),
            None,
        );
        assert!(
            svc.contains("ExecStart=/home/alice/.cargo/bin/mkd-worktree-watchdog --human prune")
        );
        let timer = render_systemd_timer();
        assert!(timer.contains("OnCalendar=*-*-* 06,18:00:00"));
        assert!(timer.contains("Persistent=true"));
    }

    #[test]
    fn renders_macos_launchd_plist_twice_daily() {
        let plist = render_launchd_plist(
            Path::new("/usr/local/bin/mkd-worktree-watchdog"),
            Some(Path::new(
                "/Users/alice/.config/mkd-worktree-watchdog/config.toml",
            )),
            Path::new("/Users/alice/Library/Logs/mkd-worktree-watchdog.log"),
        );
        assert!(plist.contains("<string>dev.monkeyking.worktree-watchdog</string>"));
        assert!(plist.contains("<integer>6</integer>"));
        assert!(plist.contains("<integer>18</integer>"));
        assert!(plist.contains("<string>--config</string>"));
        assert!(plist.contains("StandardOutPath"));
    }

    #[test]
    fn renders_windows_task_xml_twice_daily() {
        let xml = render_windows_task_xml(
            Path::new(r"C:\Users\alice\.cargo\bin\mkd-worktree-watchdog.exe"),
            None,
        );
        assert!(xml.contains("2026-01-01T06:00:00"));
        assert!(xml.contains("2026-01-01T18:00:00"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains(r"C:\Users\alice\.cargo\bin\mkd-worktree-watchdog.exe"));
    }
}
