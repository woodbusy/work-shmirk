mod common;

use common::TestEnv;

#[test]
fn remove_from_inside_worktree_defaults_to_basename() {
    let env = TestEnv::new();
    env.bin().args(["new", "feature-y"]).assert().success();

    let wt = env.worktrees_root().join("feature-y");
    assert!(wt.is_dir());

    // Run `remove` from inside the worktree (no arg → defaults to basename).
    let mut cmd = assert_cmd::Command::cargo_bin("work-shmirk").unwrap();
    cmd.current_dir(&wt);
    cmd.env("WORK_SHMIRK_CLAUDE_BIN", env.stubs_dir.join("claude"));
    cmd.env("WORK_SHMIRK_TMUX_BIN", env.stubs_dir.join("tmux"));
    cmd.env("WORK_SHMIRK_NO_EXEC", "1");
    cmd.env("SHELL", "/bin/sh");
    cmd.env_remove("TMUX");
    cmd.arg("remove");
    cmd.assert().success();

    assert!(!wt.exists());
}

#[test]
fn remove_from_main_worktree_without_arg_errors() {
    let env = TestEnv::new();
    // From the main worktree, no arg → error.
    env.bin()
        .arg("remove")
        .assert()
        .failure()
        .stderr(predicates::str::contains("main worktree"));
}
