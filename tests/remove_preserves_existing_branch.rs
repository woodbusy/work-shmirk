mod common;

use common::{git, TestEnv};

/// `work-shmirk remove` must not delete a branch that was not created by
/// `work-shmirk new`.  When `-e` is used the worktree is attached to a
/// pre-existing branch; no ownership entry is written in git config, so the
/// remove flow skips the branch delete.
#[test]
fn remove_preserves_existing_branch() {
    let env = TestEnv::new();

    // Create a long-lived branch that should survive the remove.
    git(&env.repo_dir, &["branch", "keepme"]);

    // Attach a worktree to the existing branch (no new branch created).
    env.bin().args(["new", "-e", "keepme"]).assert().success();

    let wt = env.worktrees_root().join("keepme");
    assert!(wt.is_dir(), "worktree dir should exist after `new -e`");

    // Ownership must NOT be recorded in git config because `-e` was used.
    let config_check = std::process::Command::new("git")
        .args(["config", "--local", "work-shmirk.owned-branch"])
        .current_dir(&wt)
        .output()
        .unwrap();
    assert!(
        !config_check.status.success(),
        "work-shmirk.owned-branch must not be set in git config when `-e` is used"
    );

    // git worktree remove refuses if there are untracked/modified files;
    // clean the worktree first.
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    // Remove the worktree; capture output to assert the skip message.
    let output = env.bin().args(["remove", "keepme"]).output().unwrap();
    assert!(
        output.status.success(),
        "remove should succeed even when branch is not deleted"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Skipping branch delete"),
        "stderr should contain 'Skipping branch delete', got: {stderr}"
    );

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
