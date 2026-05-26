mod common;

use common::TestEnv;
use std::fs;

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
