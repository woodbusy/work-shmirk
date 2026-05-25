mod common;

use common::TestEnv;
use std::fs;

/// Verify that `remove` fails when the per-worktree `.work-shmirk` entry
/// exists as a regular file rather than a directory.
///
/// The worktree is intentionally left dirty (no `git clean -fdx`) so that if a
/// future refactor accidentally moves the directory check below the destructive
/// git operations, `git worktree remove` will fail for a different reason
/// (untracked files) and the test will misbehave in a way that alerts the
/// implementer that the guard order changed.
#[test]
fn remove_fails_when_target_cfg_is_regular_file() {
    let env = TestEnv::new();

    env.bin().args(["new", "feature-y"]).assert().success();

    let wt = env.worktrees_root().join("feature-y");
    assert!(wt.is_dir(), "worktree should have been created");

    // Plant a regular file where the config dir would be.
    let bad_cfg = wt.join(".work-shmirk");
    fs::write(&bad_cfg, "").unwrap();

    env.bin().args(["remove", "feature-y"]).assert().failure();

    // The worktree must still exist: the error must fire before any destructive
    // git operation (worktree remove, branch delete).
    assert!(
        wt.is_dir(),
        "worktree dir must still exist after the expected failure"
    );
}

/// Same guard, but with a broken symlink instead of a regular file.
#[test]
fn remove_fails_when_target_cfg_is_broken_symlink() {
    let env = TestEnv::new();

    env.bin().args(["new", "feature-z"]).assert().success();

    let wt = env.worktrees_root().join("feature-z");
    assert!(wt.is_dir(), "worktree should have been created");

    // Plant a broken symlink where the config dir would be.
    let bad_cfg = wt.join(".work-shmirk");
    std::os::unix::fs::symlink("/nonexistent/path", &bad_cfg).unwrap();

    env.bin().args(["remove", "feature-z"]).assert().failure();

    assert!(
        wt.is_dir(),
        "worktree dir must still exist after the expected failure"
    );
}
