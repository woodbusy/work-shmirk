//! Tmux 3-pane setup for the `new` flow.
//!
//! Deliberate divergences from the bash script:
//!   1. Pane working directories are set via tmux `-c <dir>` (passed as an
//!      argv element), so the worktree path never traverses a shell and
//!      requires no shell escaping.
//!   2. Pane commands are launched via `sh -c '<script>'` after `--`, not
//!      typed as keystrokes via `send-keys`. This decouples behavior from the
//!      user's interactive shell and eliminates the pane-payload injection
//!      surface entirely.
//!   3. The prompt is passed to `claude` as a single shell-escaped argument
//!      inside the `sh -c` script (POSIX single-quote escaping). The claude
//!      binary path is also shell-escaped for the same reason. These are the
//!      only two user-derived strings that are interpolated into a shell
//!      string.
//!   4. The starting pane id is captured via `display-message -p '#{pane_id}'`
//!      before the first split so we can target `respawn-pane` and `select-pane`
//!      by id rather than by layout-relative names (`-L`, `-U`, `-D`).
//!   5. The top-left pane is respawned via `respawn-pane -k` rather than
//!      receiving `cd <dir>` keystrokes. Cost: the original pane's scrollback
//!      is discarded; the new shell does not inherit in-process env mutations
//!      from the original shell. See docs/contract.md.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

fn tmux_bin() -> String {
    std::env::var("WORK_SHMIRK_TMUX_BIN").unwrap_or_else(|_| "tmux".to_string())
}

fn claude_bin() -> String {
    std::env::var("WORK_SHMIRK_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string())
}

/// POSIX single-quote escape: surround in `'...'`, with each interior `'`
/// rewritten as `'\''`.
pub fn shell_single_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn run_tmux(args: &[&str]) -> Result<()> {
    let status = Command::new(tmux_bin())
        .args(args)
        .status()
        .with_context(|| format!("invoking tmux {}", args.join(" ")))?;
    if !status.success() {
        bail!("tmux {} exited with {}", args.join(" "), status);
    }
    Ok(())
}

fn run_tmux_capture(args: &[&str]) -> Result<String> {
    let output = Command::new(tmux_bin())
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .with_context(|| format!("invoking tmux {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("tmux {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Set up the 3-pane layout. `worktree_target` is the new worktree's absolute
/// path; `prompt` is the full Claude prompt; `window_name` is the value to
/// pass to `tmux rename-window`.
pub fn setup_panes(worktree_target: &Path, prompt: &str, window_name: &str) -> Result<()> {
    let target_str = worktree_target
        .to_str()
        .context("worktree path is not valid UTF-8")?;

    // Escape only the strings that live inside a `sh -c` payload. The
    // worktree path is passed via tmux `-c` (argv) and does not need escaping.
    let prompt_q = shell_single_quote(prompt);
    let claude_q = shell_single_quote(&claude_bin());

    run_tmux(&["rename-window", window_name])?;

    // Capture the id of the starting pane so we can target it later by id
    // rather than by a layout-relative name.
    let top_left_pane_id = run_tmux_capture(&["display-message", "-p", "#{pane_id}"])?;
    if top_left_pane_id.is_empty() {
        bail!("tmux display-message returned empty pane id");
    }

    // Right pane: split horizontally, launch vim directly in the worktree.
    // `-P -F '#{pane_id}'` prints the new pane's id so we can refer to it
    // later (not strictly needed for vim, but consistent with the pattern).
    let right_pane_id = run_tmux_capture(&[
        "split-window",
        "-h",
        "-P",
        "-F",
        "#{pane_id}",
        "-c",
        target_str,
        "--",
        "sh",
        "-c",
        "exec vim",
    ])?;
    if right_pane_id.is_empty() {
        bail!("tmux split-window -h -P returned empty pane id");
    }

    // Return focus to the top-left pane by id before the vertical split.
    // After the horizontal split above, focus is in the right pane. Without
    // this select-pane, split-window -v would split the right pane instead.
    run_tmux(&["select-pane", "-t", &top_left_pane_id])?;

    // Bottom-left pane: split the top-left vertically, launch claude with the
    // prompt, then exec the user's shell so the pane stays alive after claude
    // exits (`;` rather than `&&` — the user always lands in a shell).
    let bottom_payload = format!("{claude_q} {prompt_q}; exec \"$SHELL\"");
    let bottom_pane_id = run_tmux_capture(&[
        "split-window",
        "-v",
        "-P",
        "-F",
        "#{pane_id}",
        "-c",
        target_str,
        "--",
        "sh",
        "-c",
        &bottom_payload,
    ])?;
    if bottom_pane_id.is_empty() {
        bail!("tmux split-window -v -P returned empty pane id");
    }

    // Top-left pane: respawn with a fresh shell cwd'd in the worktree.
    // `-k` kills the existing command (the shell work-shmirk was invoked from)
    // and starts a new one. The user gets a clean shell prompt; no `cd`
    // keystrokes appear in shell history. Trade-off: original scrollback is
    // discarded and the new shell does not inherit the original's in-process
    // env mutations. See docs/contract.md.
    run_tmux(&[
        "respawn-pane",
        "-k",
        "-t",
        &top_left_pane_id,
        "-c",
        target_str,
        "--",
        "sh",
        "-c",
        "exec \"$SHELL\"",
    ])?;

    // Return focus to the bottom-left (claude) pane.
    run_tmux(&["select-pane", "-t", &bottom_pane_id])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_simple() {
        assert_eq!(shell_single_quote("hello"), "'hello'");
    }

    #[test]
    fn quote_with_single_quote() {
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn quote_empty() {
        assert_eq!(shell_single_quote(""), "''");
    }
}
