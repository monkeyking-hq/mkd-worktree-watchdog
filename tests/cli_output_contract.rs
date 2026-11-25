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
    std::fs::create_dir_all(&empty_root).unwrap();
    let out = Command::new(bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
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

    // 3. --human output
    let out = Command::new(bin())
        .args([
            "--human",
            "--config",
            cfg_path.to_str().unwrap(),
            "scan",
            "--root",
            empty_root.to_str().unwrap(),
        ])
        .output()
        .expect("run human scan");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mkd-worktree-watchdog [DRY RUN]"));
}
