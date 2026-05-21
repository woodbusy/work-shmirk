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

    // Claude must have been launched with cwd == worktree target, matching
    // the bash flow's `cd <wt> && claude ...`. The stub logs `PWD=<dir>`.
    // On macOS `/var` is a symlink to `/private/var`, so canonicalize before
    // comparing.
    let expected_pwd = std::fs::canonicalize(&wt).unwrap();
    let pwd_line = log
        .lines()
        .find(|l| l.starts_with("PWD="))
        .unwrap_or_else(|| panic!("no PWD line in claude log: {log}"));
    let logged_pwd = std::fs::canonicalize(pwd_line.trim_start_matches("PWD=")).unwrap();
    assert_eq!(
        logged_pwd, expected_pwd,
        "claude was not launched from worktree target. log: {log}"
    );
}
