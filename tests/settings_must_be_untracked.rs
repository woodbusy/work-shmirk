mod common;

use common::{git, TestEnv};
use std::fs;

const SETTINGS: &str = r#"{"symlink_links": []}"#;

#[test]
fn committed_settings_rejected_before_creating_worktree() {
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(cfg_dir.join("settings.json"), SETTINGS).unwrap();
    git(&env.repo_dir, &["add", "-f", ".work-shmirk/settings.json"]);
    git(&env.repo_dir, &["commit", "-q", "-m", "ship settings"]);

    env.bin()
        .args(["new", "feature-x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));

    assert!(!env.worktrees_root().join("feature-x").exists());
}

#[test]
fn staged_settings_rejected() {
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(cfg_dir.join("settings.json"), SETTINGS).unwrap();
    git(&env.repo_dir, &["add", "-f", ".work-shmirk/settings.json"]);

    env.bin()
        .args(["new", "feature-x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));
}

#[test]
fn config_dir_symlink_into_tracked_dir_rejected() {
    let env = TestEnv::new();
    let shared_dir = env.repo_dir.join("shared");
    fs::create_dir_all(&shared_dir).unwrap();
    fs::write(shared_dir.join("settings.json"), SETTINGS).unwrap();
    git(&env.repo_dir, &["add", "shared/settings.json"]);
    git(&env.repo_dir, &["commit", "-q", "-m", "ship settings"]);
    std::os::unix::fs::symlink("shared", env.repo_dir.join(".work-shmirk")).unwrap();

    env.bin()
        .args(["new", "feature-x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));
}

#[test]
fn settings_symlink_to_file_outside_repo_accepted() {
    let env = TestEnv::new();
    let outside_settings = env.repo_dir.parent().unwrap().join("my-settings.json");
    fs::write(&outside_settings, SETTINGS).unwrap();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    std::os::unix::fs::symlink(&outside_settings, cfg_dir.join("settings.json")).unwrap();

    env.bin().args(["new", "feature-x"]).assert().success();
}

#[test]
fn legacy_local_settings_rejected() {
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(cfg_dir.join("settings.local.json"), SETTINGS).unwrap();

    env.bin()
        .args(["new", "feature-x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "settings.local.json is no longer supported",
        ));

    assert!(!env.worktrees_root().join("feature-x").exists());
}

#[test]
fn remove_rejects_tracked_per_worktree_settings_before_removing() {
    let env = TestEnv::new();
    env.bin().args(["new", "feature-x"]).assert().success();

    let wt = env.worktrees_root().join("feature-x");
    let wt_cfg_dir = wt.join(".work-shmirk");
    fs::create_dir_all(&wt_cfg_dir).unwrap();
    fs::write(wt_cfg_dir.join("settings.json"), SETTINGS).unwrap();
    git(&wt, &["add", "-f", ".work-shmirk/settings.json"]);

    env.bin()
        .args(["remove", "feature-x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));

    assert!(wt.is_dir(), "worktree must survive a rejected remove");
}

/// Create `feature-x` with a main-repo config that links `.env`, then point
/// its link at a stale target. Returns (worktree, link path, stale target).
fn worktree_with_stale_link(
    env: &TestEnv,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    let settings = serde_json::json!({
        "symlink_dir": base.to_string_lossy(),
        "symlink_links": [".env"]
    });
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(cfg_dir.join("settings.json"), settings.to_string()).unwrap();
    env.bin().args(["new", "feature-x"]).assert().success();

    let link = base.join("feature-x").join("env");
    let stale_target = env.repo_dir.parent().unwrap().join("stale");
    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&stale_target, &link).unwrap();
    (env.worktrees_root().join("feature-x"), link, stale_target)
}

#[test]
fn relink_rejects_tracked_per_worktree_settings_before_relinking() {
    let env = TestEnv::new();
    let (wt, link, stale_target) = worktree_with_stale_link(&env);
    let wt_cfg_dir = wt.join(".work-shmirk");
    fs::create_dir_all(&wt_cfg_dir).unwrap();
    fs::copy(
        env.repo_dir.join(".work-shmirk/settings.json"),
        wt_cfg_dir.join("settings.json"),
    )
    .unwrap();
    git(&wt, &["add", "-f", ".work-shmirk/settings.json"]);

    env.bin()
        .current_dir(&wt)
        .arg("relink")
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));

    assert_eq!(fs::read_link(&link).unwrap(), stale_target);
}

#[test]
fn relink_rejects_tracked_main_repo_settings_before_relinking() {
    let env = TestEnv::new();
    let (wt, link, stale_target) = worktree_with_stale_link(&env);
    git(&env.repo_dir, &["add", "-f", ".work-shmirk/settings.json"]);

    env.bin()
        .current_dir(&wt)
        .arg("relink")
        .assert()
        .failure()
        .stderr(predicates::str::contains("tracked by git"));

    assert_eq!(fs::read_link(&link).unwrap(), stale_target);
}
