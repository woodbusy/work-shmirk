mod common;

use common::TestEnv;
use std::fs;

#[test]
fn new_with_config_copies_files_and_creates_symlinks() {
    let env = TestEnv::new();

    // Set up .work-shmirk config with copy_files, symlink_dir, symlink_links,
    // and tmux window-name substitution.
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();

    // Create source files that copy_files will pick up.
    fs::write(cfg_dir.join("config.json"), "{\"k\":1}").unwrap();
    fs::write(cfg_dir.join("nested.txt"), "hello").unwrap();

    // The worktree itself should have a .env file so the symlink target exists.
    // We'll create it post-worktree-add via the copy_files mechanism using a
    // bare filename dest.
    let symlink_base = env.repo_dir.join("links");
    fs::create_dir_all(&symlink_base).unwrap();

    let settings = serde_json::json!({
        "copy_files": {
            "config.json": "config.json",
            "nested.txt": "sub/dir/nested.txt",
            ".env-src": ".env"
        },
        "symlink_dir": symlink_base.to_string_lossy(),
        "symlink_links": [".env"],
        "issues": {
            "type": "linear",
            "project": "ENG"
        },
        "tmux": {
            "project_name_substitution": "E"
        }
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
    fs::write(cfg_dir.join(".env-src"), "ENV=ok").unwrap();

    env.bin()
        .env("TMUX", "fake,0,0")
        .args(["new", "ENG-42-foo"])
        .assert()
        .success();

    let wt = env.worktrees_root().join("ENG-42-foo");
    assert!(
        wt.join("config.json").is_file(),
        "bare-filename copy missing"
    );
    assert!(
        wt.join("sub/dir/nested.txt").is_file(),
        "nested copy missing"
    );
    assert!(wt.join(".env").is_file(), ".env should have been copied");

    // Symlink: <links>/ENG-42-foo/env (single leading dot stripped).
    let link_path = symlink_base.join("ENG-42-foo").join("env");
    let meta = fs::symlink_metadata(&link_path).expect("symlink should exist");
    assert!(meta.file_type().is_symlink());

    // The window name has its first `issues.project` occurrence substituted.
    let log = fs::read_to_string(&env.tmux_log).unwrap();
    assert!(
        log.contains("rename-window\nE-42-foo\n"),
        "tmux log missing substituted window name: {log}"
    );
}
