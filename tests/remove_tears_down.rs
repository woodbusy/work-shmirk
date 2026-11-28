mod common;

use common::TestEnv;
use predicates::str::contains;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

#[test]
fn remove_takes_down_worktree_branch_and_symlinks() {
    let env = TestEnv::new();

    // Set up config so the new flow creates symlinks.
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    let symlink_base = env.repo_dir.join("links");
    fs::create_dir_all(&symlink_base).unwrap();
    fs::write(cfg_dir.join(".env-src"), "X=1").unwrap();

    let settings = serde_json::json!({
        "copy_files": { ".env-src": ".env" },
        "symlink_dir": symlink_base.to_string_lossy(),
        "symlink_links": [".env"]
    });
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();

    env.bin().args(["new", "feature-x"]).assert().success();

    let wt = env.worktrees_root().join("feature-x");
    assert!(wt.is_dir());
    assert!(symlink_base.join("feature-x").join("env").exists());

    // `new` (without -e) must record ownership in git config so `remove` knows
    // it is safe to delete the branch.
    let git_config_output = std::process::Command::new("git")
        .args(["config", "--local", "work-shmirk.owned-branch"])
        .current_dir(&wt)
        .output()
        .unwrap();
    assert!(
        git_config_output.status.success(),
        "git config work-shmirk.owned-branch should be set after `new`"
    );
    let config_value = String::from_utf8_lossy(&git_config_output.stdout);
    assert_eq!(
        config_value.trim(),
        "feature-x",
        "work-shmirk.owned-branch should be set to 'feature-x'"
    );

    // git worktree remove refuses if there are untracked/modified files.
    // Stage what we care about and clean the rest to keep this test focused
    // on the removal flow itself.
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    // Ownership is tracked in `.git/worktrees/feature-x/config`, which `git
    // clean -fdx` does not touch, so no recreation step is needed.

    // Now remove.
    env.bin().args(["remove", "feature-x"]).assert().success();

    assert!(!wt.exists(), "worktree dir should be gone");

    // Symlink dir gone.
    assert!(
        !symlink_base.join("feature-x").exists(),
        "per-worktree symlink dir should be gone"
    );

    // Branch gone.
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "feature-x"])
        .current_dir(env.worktrees_root())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&branches.stdout);
    assert!(
        stdout.trim().is_empty(),
        "branch should be gone, got: {stdout}"
    );
}

const WORKTREE: &str = "feature-x";

fn write_settings(cfg_dir: &std::path::Path, settings: serde_json::Value) {
    fs::create_dir_all(cfg_dir).unwrap();
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();
}

/// Creates the worktree with a main-repo config that links nothing, so the
/// link dir contents are built by the test itself. Returns (worktree, link
/// dir, links base).
fn setup_without_links(env: &TestEnv) -> (PathBuf, PathBuf, PathBuf) {
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": [] }),
    );
    env.bin().args(["new", WORKTREE]).assert().success();
    let link_dir = base.join(WORKTREE);
    (env.worktrees_root().join(WORKTREE), link_dir, base)
}

fn is_symlink(path: &std::path::Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

#[test]
fn remove_tears_down_nested_entries() {
    let env = TestEnv::new();
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({
            "symlink_dir": base.to_string_lossy(),
            "symlink_links": [".claude/skills", ".claude/commands", ".env"]
        }),
    );
    env.bin().args(["new", WORKTREE]).assert().success();
    let wt = env.worktrees_root().join(WORKTREE);
    let link_dir = base.join(WORKTREE);
    assert!(is_symlink(&link_dir.join("claude/skills")));
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    env.bin().args(["remove", WORKTREE]).assert().success();

    assert!(
        fs::symlink_metadata(&link_dir).is_err(),
        "link dir should be gone"
    );
    assert!(!wt.exists(), "worktree dir should be gone");
}

#[test]
fn remove_leaves_symlinked_intermediate_untouched() {
    let env = TestEnv::new();
    let (wt, link_dir, base) = setup_without_links(&env);
    let outside = env.repo_dir.parent().unwrap().join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(env.repo_dir.join("README.md"), outside.join("x")).unwrap();
    symlink(&outside, link_dir.join("a")).unwrap();
    write_settings(
        &wt.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": ["a/x"] }),
    );

    let outcome = env.bin().args(["remove", WORKTREE]).assert().failure();

    assert!(is_symlink(&outside.join("x")), "outside x must survive");
    assert!(is_symlink(&link_dir.join("a")));
    assert!(wt.is_dir(), "worktree must be left intact");
    outcome.stderr(contains("leaving 'a/x'"));
}

#[test]
fn remove_leaves_non_empty_intermediate() {
    let env = TestEnv::new();
    let (wt, link_dir, base) = setup_without_links(&env);
    fs::create_dir(link_dir.join("a")).unwrap();
    symlink(wt.join("a/x"), link_dir.join("a/x")).unwrap();
    fs::write(link_dir.join("a/notes"), "mine").unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": ["a/x"] }),
    );

    env.bin().args(["remove", WORKTREE]).assert().failure();

    assert!(fs::symlink_metadata(link_dir.join("a/x")).is_err());
    assert_eq!(
        fs::read_to_string(link_dir.join("a/notes")).unwrap(),
        "mine"
    );
    assert!(wt.is_dir(), "worktree must be left intact");
}

#[test]
fn remove_leaves_regular_file_at_dest() {
    let env = TestEnv::new();
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": [".env"] }),
    );
    env.bin().args(["new", WORKTREE]).assert().success();
    let wt = env.worktrees_root().join(WORKTREE);
    let dest = base.join(WORKTREE).join("env");
    fs::remove_file(&dest).unwrap();
    fs::write(&dest, "mine").unwrap();

    env.bin().args(["remove", WORKTREE]).assert().failure();

    assert_eq!(fs::read_to_string(&dest).unwrap(), "mine");
    assert!(wt.is_dir(), "worktree must be left intact");
}
