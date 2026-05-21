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
