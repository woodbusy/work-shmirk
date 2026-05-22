mod common;

use common::TestEnv;

/// Verify the tmux branch of `work-shmirk new` emits direct tmux primitives
/// (split-window -c, respawn-pane, select-pane by id) rather than send-keys
/// shell strings. TMUX must be set to enter the tmux code path.
#[test]
fn tmux_flow_uses_direct_primitives() {
    let env = TestEnv::new();

    env.bin()
        .env("TMUX", "fake,0,0")
        .args(["new", "feature-tmux"])
        .assert()
        .success();

    let wt = env.worktrees_root().join("feature-tmux");
    assert!(wt.is_dir(), "worktree dir should exist");

    let tmux_log = std::fs::read_to_string(&env.tmux_log)
        .unwrap_or_else(|e| panic!("failed to read tmux log: {e}"));

    // Window is renamed to the branch name.
    assert!(
        tmux_log.contains("rename-window"),
        "rename-window missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("feature-tmux"),
        "window name missing. log:\n{tmux_log}"
    );

    // Starting pane id is captured via display-message.
    assert!(
        tmux_log.contains("display-message"),
        "display-message missing. log:\n{tmux_log}"
    );

    // Right pane: horizontal split with -c and exec vim.
    assert!(
        tmux_log.contains("split-window"),
        "split-window missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("-h"),
        "horizontal split flag missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("-c"),
        "split-window -c flag missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("exec vim"),
        "right-pane 'exec vim' payload missing. log:\n{tmux_log}"
    );

    // The worktree path appears in the log (passed as -c argument).
    let wt_str = wt.to_str().expect("worktree path is valid UTF-8");
    assert!(
        tmux_log.contains(wt_str),
        "worktree path missing from log. log:\n{tmux_log}"
    );

    // Bottom-left pane: vertical split with -c. Prompt text is present.
    assert!(
        tmux_log.contains("-v"),
        "vertical split flag missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("feature-tmux"),
        "prompt text missing from log. log:\n{tmux_log}"
    );

    // Top-left pane: respawned via respawn-pane -k.
    assert!(
        tmux_log.contains("respawn-pane"),
        "respawn-pane missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains("-k"),
        "respawn-pane -k flag missing. log:\n{tmux_log}"
    );

    // The stub derives each pane id from the count of --end-- markers already
    // in the log before that invocation. The call sequence in setup_panes is:
    //   (0) rename-window         → n=0, no output
    //   (1) display-message       → n=1, outputs %100  (top-left id)
    //   (2) split-window -h -P    → n=2, outputs %101  (right pane id)
    //   (3) select-pane -t %100   → n=3, no output
    //   (4) split-window -v -P    → n=4, outputs %103  (bottom pane id)
    //   (5) respawn-pane -t %100  → n=5, no output
    //   (6) select-pane -t %103   → n=6, no output
    // display-message and the two split-window -P calls all get distinct ids.
    let top_left_id = "%100";
    let bottom_id = "%103";
    assert!(
        tmux_log.contains(top_left_id),
        "{top_left_id} (top-left pane id) missing. log:\n{tmux_log}"
    );
    assert!(
        tmux_log.contains(bottom_id),
        "{bottom_id} (bottom pane id) missing. log:\n{tmux_log}"
    );

    // respawn-pane targets the top-left pane id and the final select-pane
    // targets the bottom pane id — proves two distinct ids are wired through
    // the Rust code.
    let respawn_idx = tmux_log.find("respawn-pane").expect("respawn-pane in log");
    let select_final_idx = tmux_log.rfind("select-pane").expect("select-pane in log");
    let respawn_region = &tmux_log[respawn_idx..];
    let select_final_region = &tmux_log[select_final_idx..];
    assert!(
        respawn_region.contains(top_left_id),
        "respawn-pane should reference {top_left_id}. region:\n{respawn_region}"
    );
    assert!(
        select_final_region.contains(bottom_id),
        "final select-pane should reference {bottom_id}. region:\n{select_final_region}"
    );

    // Confirm send-keys is NOT used (the whole point of the rewrite).
    assert!(
        !tmux_log.contains("send-keys"),
        "send-keys should not appear in the new flow. log:\n{tmux_log}"
    );
}
