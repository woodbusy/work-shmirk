//! Tmux 3-pane setup for the `new` flow.
//!
//! Deliberate divergences from the bash script:
//!   1. Right pane id is captured from `split-window -h -P -F '#{pane_id}'`
//!      and used to target subsequent send-keys, instead of the bash script's
//!      `-t right` (a non-standard reference that silently misfires on default
//!      tmux configs).
//!   2. The prompt is passed to `claude` as a single argument literal (with
//!      POSIX single-quote escaping), eliminating the `"$(cat <tmpfile>)"`
//!      indirection and the tempfile entirely.

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
    let target_q = shell_single_quote(target_str);
    let prompt_q = shell_single_quote(prompt);

    run_tmux(&["rename-window", window_name])?;

    // Capture the right pane id from split-window -P -F.
    let right_pane_id = run_tmux_capture(&["split-window", "-h", "-P", "-F", "#{pane_id}"])?;
    if right_pane_id.is_empty() {
        bail!("tmux split-window -P returned empty pane id");
    }

    // Right pane: cd + vim.
    let right_payload = format!("cd {target_q} && vim");
    run_tmux(&["send-keys", "-t", &right_pane_id, &right_payload, "Enter"])?;

    // Select left pane (which is the original) and split vertically.
    run_tmux(&["select-pane", "-L"])?;
    run_tmux(&["split-window", "-v"])?;

    // Bottom-left pane is now active: launch claude with the prompt as a
    // single arg, then exec $SHELL. Honor WORK_SHMIRK_CLAUDE_BIN so the
    // override propagates into the pane (single-quote escaped).
    let claude_q = shell_single_quote(&claude_bin());
    let bottom_payload = format!("cd {target_q} && {claude_q} {prompt_q} && exec $SHELL");
    run_tmux(&["send-keys", &bottom_payload, "Enter"])?;

    // Top-left: cd into the worktree.
    run_tmux(&["select-pane", "-U"])?;
    let top_payload = format!("cd {target_q}");
    run_tmux(&["send-keys", &top_payload, "Enter"])?;

    // Return focus to bottom-left.
    run_tmux(&["select-pane", "-D"])?;

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
