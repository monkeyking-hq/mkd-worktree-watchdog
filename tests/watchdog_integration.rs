// SPDX-FileCopyrightText: 2026 Nathan A Chadwick
//
// SPDX-License-Identifier: MIT

use mkd_worktree_watchdog::config::{Config, PathRule};
use mkd_worktree_watchdog::git::list_worktrees;
use mkd_worktree_watchdog::rules::Action;
use mkd_worktree_watchdog::run_with_time;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
    fs::write(dir.join("README.md"), "hello world\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", "initial commit"]);
}

fn base_config(scan_root: PathBuf, orphan_root: PathBuf) -> Config {
    let mut cfg = Config {
        scan_roots: vec![scan_root],
        orphan_roots: vec![orphan_root],
        ..Config::default()
    };
    cfg.defaults.skip_active_processes = false;
    cfg
}

#[test]
fn prunes_stale_clean_merged_worktree_and_keeps_dirty_unpushed_locked() {
    let tmp = TempDir::new().unwrap();
    let workspaces = tmp.path().join("workspaces");
    let orphans = tmp.path().join("orphans");
    fs::create_dir_all(&orphans).unwrap();

    let repo = workspaces.join("demo-repo");
    init_repo(&repo);

    // 1. Clean merged worktree (should be pruned when stale).
    let wt_stale = tmp.path().join("wt-stale");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/stale",
            wt_stale.to_str().unwrap(),
        ],
    );

    // 2. Dirty worktree (should be kept by default).
    let wt_dirty = tmp.path().join("wt-dirty");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/dirty",
            wt_dirty.to_str().unwrap(),
        ],
    );
    fs::write(wt_dirty.join("uncommitted.txt"), "wip\n").unwrap();

    // 3. Unpushed/unmerged commit worktree (should be kept by default).
    let wt_unpushed = tmp.path().join("wt-unpushed");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/unpushed",
            wt_unpushed.to_str().unwrap(),
        ],
    );
    fs::write(wt_unpushed.join("new.txt"), "commit me\n").unwrap();
    git(&wt_unpushed, &["add", "."]);
    git(&wt_unpushed, &["commit", "-m", "local unpushed commit"]);

    // 4. Locked worktree (should be kept by default).
    let wt_locked = tmp.path().join("wt-locked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/locked",
            wt_locked.to_str().unwrap(),
        ],
    );
    git(&repo, &["worktree", "lock", wt_locked.to_str().unwrap()]);

    // 5. Missing worktree directory (metadata should be pruned).
    let wt_missing = tmp.path().join("wt-missing");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/missing",
            wt_missing.to_str().unwrap(),
        ],
    );
    fs::remove_dir_all(&wt_missing).unwrap();

    // 6. Orphaned directories under orphan_roots (empty dir + broken .git pointer).
    let orphan_empty = orphans.join("empty-agent-dir");
    fs::create_dir_all(&orphan_empty).unwrap();
    let orphan_broken = orphans.join("broken-agent-dir");
    fs::create_dir_all(&orphan_broken).unwrap();
    fs::write(
        orphan_broken.join(".git"),
        "gitdir: /nonexistent/repo/.git/worktrees/dead\n",
    )
    .unwrap();
    fs::write(orphan_broken.join("artifact.bin"), vec![0u8; 4096]).unwrap();

    // Simulate 100 hours in the future so everything exceeds default 72h / 24h thresholds.
    let future = SystemTime::now() + Duration::from_secs(100 * 3600);
    let mut cfg = base_config(workspaces, orphans);
    cfg.defaults.delete_merged_branches = true;

    // Dry run first: nothing deleted.
    let dry = run_with_time(&cfg, None, true, future).unwrap();
    assert_eq!(dry.summary.repos_scanned, 1);
    assert_eq!(dry.summary.linked_worktrees_found, 5);
    assert_eq!(dry.summary.worktrees_to_prune, 1);
    assert_eq!(dry.summary.metadata_to_prune, 1);
    assert_eq!(dry.summary.orphans_to_prune, 2);
    assert!(wt_stale.exists());
    assert!(orphan_empty.exists());
    assert!(orphan_broken.exists());

    // Live prune.
    let live = run_with_time(&cfg, None, false, future).unwrap();
    assert_eq!(live.summary.error_count, 0);
    assert_eq!(live.summary.worktrees_pruned, 1);
    assert_eq!(live.summary.metadata_repos_pruned, 1);
    assert_eq!(live.summary.orphans_pruned, 2);
    assert_eq!(live.summary.branches_deleted, 1);
    assert!(live.summary.reclaimed_bytes >= 4096);

    // Verify disk state.
    assert!(repo.exists(), "main repo must never be deleted");
    assert!(!wt_stale.exists(), "stale clean worktree should be removed");
    assert!(wt_dirty.exists(), "dirty worktree must be preserved");
    assert!(wt_unpushed.exists(), "unpushed worktree must be preserved");
    assert!(wt_locked.exists(), "locked worktree must be preserved");
    assert!(!orphan_empty.exists(), "empty orphan should be removed");
    assert!(!orphan_broken.exists(), "broken orphan should be removed");

    // Verify git worktree list no longer contains stale or missing entries.
    let remaining = list_worktrees(&repo).unwrap();
    assert_eq!(remaining.len(), 4); // main + dirty + unpushed + locked
    assert!(remaining.iter().all(|w| !w.prunable));
}

#[test]
fn per_path_rule_override_respects_custom_age_and_disable() {
    let tmp = TempDir::new().unwrap();
    let workspaces = tmp.path().join("workspaces");
    let orphans = tmp.path().join("orphans");
    fs::create_dir_all(&orphans).unwrap();

    let repo = workspaces.join("app");
    init_repo(&repo);

    let wt_fast = tmp.path().join("agent-fast/wt1");
    let wt_protected = tmp.path().join("protected/wt2");
    fs::create_dir_all(wt_fast.parent().unwrap()).unwrap();
    fs::create_dir_all(wt_protected.parent().unwrap()).unwrap();

    git(
        &repo,
        &["worktree", "add", "--detach", wt_fast.to_str().unwrap()],
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            wt_protected.to_str().unwrap(),
        ],
    );

    let mut cfg = base_config(workspaces, orphans);
    cfg.defaults.max_age_hours = 72;
    cfg.rules = vec![
        PathRule {
            name: "fast-agent".to_string(),
            match_path: "**/agent-fast/**".to_string(),
            enabled: None,
            max_age_hours: Some(6),
            allow_dirty: None,
            dirty_max_age_hours: None,
            allow_unpushed: None,
            allow_locked: None,
            delete_merged_branches: None,
        },
        PathRule {
            name: "never-prune-protected".to_string(),
            match_path: "**/protected/**".to_string(),
            enabled: Some(false),
            max_age_hours: Some(1),
            allow_dirty: None,
            dirty_max_age_hours: None,
            allow_unpushed: None,
            allow_locked: None,
            delete_merged_branches: None,
        },
    ];

    // 10 hours later: wt_fast (>= 6h) should be pruned, wt_protected (disabled) kept.
    let future = SystemTime::now() + Duration::from_secs(10 * 3600);
    let report = run_with_time(&cfg, None, true, future).unwrap();

    let fast_eval = report
        .worktrees
        .iter()
        .find(|w| w.path.ends_with("agent-fast/wt1"))
        .unwrap();
    assert_eq!(fast_eval.decision.action, Action::PruneWorktree);
    assert_eq!(
        fast_eval.decision.matched_rule.as_deref(),
        Some("fast-agent")
    );

    let prot_eval = report
        .worktrees
        .iter()
        .find(|w| w.path.ends_with("protected/wt2"))
        .unwrap();
    assert_eq!(prot_eval.decision.action, Action::Keep);
    assert_eq!(
        prot_eval.decision.matched_rule.as_deref(),
        Some("never-prune-protected")
    );
}
