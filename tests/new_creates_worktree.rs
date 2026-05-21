mod common;

use common::TestEnv;

#[test]
fn new_creates_worktree_dir_and_branch_and_invokes_claude() {
    let env = TestEnv::new();

    env.bin().args(["new", "feature-x"]).assert().success();

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
    let stdout = String::from_utf8_lossy(&branches.stdout);
    assert!(stdout.contains("feature-x"), "branch missing: {stdout}");

    // Claude stub should have been called with the prompt arg.
    let log = std::fs::read_to_string(&env.claude_log).unwrap();
    assert!(
        log.contains("branch named 'feature-x'"),
        "claude prompt missing branch name. log: {log}"
    );
}
