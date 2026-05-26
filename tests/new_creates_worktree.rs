mod common;

use common::TestEnv;

#[test]
fn new_creates_worktree_dir_and_branch_and_prints_path() {
    let env = TestEnv::new();

    let output = env.bin().args(["new", "feature-x"]).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();

    let wt = env.worktrees_root().join("feature-x");
    assert!(wt.is_dir(), "worktree dir should exist");
    assert!(wt.join(".worktree-local").is_dir());
    assert!(wt.join(".worktree-local/tmp").is_dir());

    // Branch should exist.
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "feature-x"])
        .current_dir(env.worktrees_root())
        .output()
        .unwrap();
    let branch_out = String::from_utf8_lossy(&branches.stdout);
    assert!(branch_out.contains("feature-x"), "branch missing: {branch_out}");

    // Worktree path must be printed to stdout for the shell wrapper to capture.
    assert!(
        stdout.trim().ends_with("feature-x"),
        "stdout should end with worktree name: {stdout}"
    );
}
