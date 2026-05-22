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

    // `new` (without -e) must write the sentinel so `remove` knows it is safe
    // to delete the branch.
    let sentinel = wt.join(".worktree-local/work-shmirk-owned-branch");
    assert!(
        sentinel.exists(),
        "sentinel file should have been written by `new`"
    );

    // git worktree remove refuses if there are untracked/modified files (this
    // matches bash behavior too — the user normally commits or .gitignores
    // .worktree-local/ etc.). Stage what we care about and remove the rest
    // to keep this test focused on the removal flow itself.
    std::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(&wt)
        .status()
        .unwrap();

    // `git clean -fdx` removed the sentinel along with other untracked content.
    // Recreate it so the remove flow sees it — this mirrors what happens in real
    // usage where `.worktree-local/` is gitignored (so `git clean` would not
    // touch it).
    fs::create_dir_all(wt.join(".worktree-local")).unwrap();
    fs::write(&sentinel, "").unwrap();

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
