mod common;

use common::{write_executable, TestEnv};

/// When claude exits non-zero, `work-shmirk new` must fail (not warn and
/// continue).  This guards against silently swallowing a misconfigured or
/// crashed claude invocation and dropping the user into a shell anyway.
///
/// Note: the tmux flow is unchanged — it hands off pane command execution to
/// tmux directly and never observes claude's exit code.  This test exercises
/// the inline flow only (TMUX is unset by `TestEnv::bin`).
#[test]
fn new_propagates_claude_failure() {
    let env = TestEnv::new();

    // Write a stub claude that exits with a non-zero code and emits nothing.
    let failing_claude = env.stubs_dir.join("failing-claude");
    write_executable(&failing_claude, "#!/bin/sh\nexit 7\n");

    let result = env
        .bin()
        .args(["new", "feature-broken-claude"])
        .env("WORK_SHMIRK_CLAUDE_BIN", &failing_claude)
        .output()
        .unwrap();

    assert!(
        !result.status.success(),
        "work-shmirk new should fail when claude exits non-zero"
    );

    // The worktree directory is created before claude runs, so it should exist.
    let wt = env.worktrees_root().join("feature-broken-claude");
    assert!(
        wt.is_dir(),
        "worktree dir should exist even though claude failed"
    );

    // The error message must mention the claude exit.
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("claude exited with"),
        "stderr should describe the claude failure, got: {stderr}"
    );

    // The "Launching shell" line must NOT appear — the inline flow aborts on
    // claude failure without proceeding to exec $SHELL.
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        !stdout.contains("Launching shell"),
        "should not reach the shell-launch stage after claude failure. stdout: {stdout}"
    );
}
