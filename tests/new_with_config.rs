mod common;

use common::TestEnv;
use std::fs;

#[test]
fn new_with_config_copies_files_creates_symlinks_and_renders_linear_prompt() {
    let env = TestEnv::new();

    // Set up .work-shmirk config with copy_files, symlink_dir, symlink_links,
    // and linear issue type.
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
            "project": "ENG",
            "cli": "linear"
        }
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
    fs::write(cfg_dir.join(".env-src"), "ENV=ok").unwrap();

    env.bin().args(["new", "ENG-42-foo"]).assert().success();

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

    // Claude prompt should contain "ENG-42" and the linear fetch command.
    let log = fs::read_to_string(&env.claude_log).unwrap();
    assert!(log.contains("ENG-42"), "log missing ENG-42: {log}");
    assert!(
        log.contains("linear issue view ENG-42"),
        "log missing fetch cmd: {log}"
    );
}
