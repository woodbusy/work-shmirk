mod common;

use common::TestEnv;
use std::fs;

#[test]
fn copy_files_dest_escape_rejected() {
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(cfg_dir.join("src.txt"), "x").unwrap();

    let settings = serde_json::json!({
        "copy_files": { "src.txt": "../escape.txt" }
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    env.bin().args(["new", "escape-test"]).assert().failure();

    // Nothing escaped.
    assert!(!env.repo_dir.parent().unwrap().join("escape.txt").exists());
}

#[test]
fn remove_symlinks_dotdot_rejected() {
    // A hostile config at remove time should not be able to drive remove_file
    // against an arbitrary symlink outside the configured symlink_dir.
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    let symlink_base = env.repo_dir.join("links");
    fs::create_dir_all(&symlink_base).unwrap();

    // Plant a sentinel symlink outside the per-worktree link dir that an
    // unguarded remove_symlinks could be tricked into removing via a
    // `link_source` like `../../sentinel`.
    let sentinel_target = env.repo_dir.join("sentinel-target");
    fs::write(&sentinel_target, "x").unwrap();
    let sentinel_link = env.repo_dir.join("sentinel-link");
    std::os::unix::fs::symlink(&sentinel_target, &sentinel_link).unwrap();
    assert!(sentinel_link.exists());

    // Create the worktree first with an innocuous config so the symlink
    // base/<name> dir exists. Then rewrite the config to a hostile entry
    // before running remove. (We avoid running `new` with the hostile entry
    // since that would already fail path validation up front.)
    let benign_settings = serde_json::json!({
        "symlink_dir": symlink_base.to_string_lossy(),
        "symlink_links": []
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&benign_settings).unwrap(),
    )
    .unwrap();
    env.bin().args(["new", "escape-test"]).assert().success();

    let hostile_settings = serde_json::json!({
        "symlink_dir": symlink_base.to_string_lossy(),
        "symlink_links": ["../../sentinel-link"]
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&hostile_settings).unwrap(),
    )
    .unwrap();

    // Clean the worktree to keep `git worktree remove` happy.
    let wt = env.repo_dir.join("escape-test");
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    env.bin().args(["remove", "escape-test"]).assert().failure();

    // Sentinel symlink must still exist.
    assert!(
        sentinel_link.symlink_metadata().is_ok(),
        "sentinel symlink should not have been removed"
    );
}

#[test]
fn symlink_links_dotdot_rejected() {
    let env = TestEnv::new();
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    let symlink_base = env.repo_dir.join("links");
    fs::create_dir_all(&symlink_base).unwrap();

    let settings = serde_json::json!({
        "symlink_dir": symlink_base.to_string_lossy(),
        "symlink_links": ["../escape"]
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    env.bin().args(["new", "escape-test"]).assert().failure();

    // No bogus symlink outside symlink_base.
    assert!(!symlink_base.join("escape").exists());
}
