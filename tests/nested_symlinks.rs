mod common;

use common::{git, TestEnv};
use std::fs;
use std::path::{Path, PathBuf};

const WORKTREE: &str = "feature-x";

fn write_settings(cfg_dir: &Path, settings: serde_json::Value) {
    fs::create_dir_all(cfg_dir).unwrap();
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();
}

/// Main-repo config linking `links` under a links base outside the repo.
/// Returns the links base.
fn configure_links(env: &TestEnv, links: &[&str]) -> PathBuf {
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": links }),
    );
    base
}

#[test]
fn new_links_nested_entry() {
    let env = TestEnv::new();
    let base = configure_links(&env, &[".claude/skills", ".env"]);

    env.bin().args(["new", WORKTREE]).assert().success();

    let wt = env.worktrees_root().join(WORKTREE);
    let link_dir = base.join(WORKTREE);
    let claude_meta = fs::symlink_metadata(link_dir.join("claude")).unwrap();
    assert!(
        claude_meta.file_type().is_dir(),
        "claude should be a real dir"
    );
    assert_eq!(
        fs::read_link(link_dir.join("claude/skills")).unwrap(),
        wt.canonicalize().unwrap().join(".claude/skills")
    );
    assert!(fs::symlink_metadata(link_dir.join("env"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn new_refuses_nested_entry_through_symlinked_intermediate() {
    let env = TestEnv::new();
    fs::create_dir_all(env.repo_dir.join("a")).unwrap();
    fs::write(env.repo_dir.join("a/keep"), "k").unwrap();
    git(&env.repo_dir, &["add", "a/keep"]);
    git(&env.repo_dir, &["commit", "-q", "-m", "add a"]);
    configure_links(&env, &["a", "a/b"]);

    env.bin().args(["new", WORKTREE]).assert().failure();

    let wt = env.worktrees_root().join(WORKTREE);
    assert!(wt.join("a").is_dir(), "worktree should have a/");
    assert!(
        fs::symlink_metadata(wt.join("a/b")).is_err(),
        "nothing may be created through the symlinked intermediate"
    );
}
