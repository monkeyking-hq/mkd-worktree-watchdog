// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

use std::process::Command;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mkd-worktree-watchdog")
}

#[test]
fn config_init_and_scan_emit_valid_json_envelopes() {
    let tmp = TempDir::new().unwrap();
    let cfg_path = tmp.path().join("config.toml");

    // 1. config init
    let out = Command::new(bin())
        .args(["config", "init", "--output", cfg_path.to_str().unwrap()])
        .output()
        .expect("run config init");
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(json["result"], "OK");
    assert_eq!(json["data"]["created"], true);
    assert!(cfg_path.is_file());

    // 2. scan with explicit empty root
    let empty_root = tmp.path().join("empty");
    let metrics_path = tmp.path().join("metrics.json");
    std::fs::create_dir_all(&empty_root).unwrap();
    let out = Command::new(bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--metrics-path",
            metrics_path.to_str().unwrap(),
            "scan",
            "--root",
            empty_root.to_str().unwrap(),
        ])
        .output()
        .expect("run scan");
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(json["result"], "OK");
    assert_eq!(json["data"]["dry_run"], true);
    assert_eq!(json["data"]["summary"]["repos_scanned"], 0);
    assert_eq!(json["data"]["run_metrics"]["reclaimed_gb"], 0.0);
    assert_eq!(json["data"]["lifetime_metrics"]["total_runs"], 0);

    // 3. --human output includes run + lifetime metrics
    let out = Command::new(bin())
        .args([
            "--human",
            "--config",
            cfg_path.to_str().unwrap(),
            "--metrics-path",
            metrics_path.to_str().unwrap(),
            "scan",
            "--root",
            empty_root.to_str().unwrap(),
        ])
        .output()
        .expect("run human scan");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mkd-worktree-watchdog [DRY RUN]"));
    assert!(text.contains("run metrics:"));
    assert!(text.contains("lifetime metrics:"));

    // 4. metrics subcommand (JSON + human)
    let out = Command::new(bin())
        .args(["--metrics-path", metrics_path.to_str().unwrap(), "metrics"])
        .output()
        .expect("run metrics");
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(json["result"], "OK");
    assert_eq!(json["data"]["total_reclaimed_gb"], 0.0);

    let out = Command::new(bin())
        .args([
            "--human",
            "--metrics-path",
            metrics_path.to_str().unwrap(),
            "metrics",
        ])
        .output()
        .expect("run human metrics");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mkd-worktree-watchdog [LIFETIME METRICS]"));
    assert!(text.contains("total reclaimed:"));
}
