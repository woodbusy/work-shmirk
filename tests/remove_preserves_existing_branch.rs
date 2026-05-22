mod common;

use common::{git, TestEnv};

/// `work-shmirk remove` must not delete a branch that was not created by
/// `work-shmirk new`.  When `-e` is used the worktree is attached to a
/// pre-existing branch; no sentinel file is written, so the remove flow
/// skips the branch delete.
#[test]
fn remove_preserves_existing_branch() {
    let env = TestEnv::new();

    // Create a long-lived branch that should survive the remove.
    git(&env.repo_dir, &["branch", "keepme"]);

    // Attach a worktree to the existing branch (no new branch created).
    env.bin().args(["new", "-e", "keepme"]).assert().success();

    let wt = env.worktrees_root().join("keepme");
    assert!(wt.is_dir(), "worktree dir should exist after `new -e`");

    // Sentinel must NOT be present because `-e` was used.
    assert!(
        !wt.join(".worktree-local/work-shmirk-owned-branch").exists(),
        "sentinel must not be written when `-e` is used"
    );

    // git worktree remove refuses if there are untracked/modified files;
    // clean the worktree first.
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    // Remove the worktree.
    env.bin().args(["remove", "keepme"]).assert().success();

    assert!(!wt.exists(), "worktree dir should be gone after remove");

    // The branch must still exist because work-shmirk did not create it.
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "keepme"])
        .current_dir(env.worktrees_root())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&branches.stdout);
    assert!(
        stdout.contains("keepme"),
        "branch 'keepme' should still exist after remove, got: {stdout}"
    );
}
