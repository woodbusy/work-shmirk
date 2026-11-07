//! Tmux 3-pane setup for the `new` flow.
//!
//! Layout: top-left shell, bottom-left claude, right vim — all cwd'd in the
//! new worktree. Pane working directories flow through tmux `-c <dir>` (argv),
//! never through a shell string. The claude binary path is the only
//! configurable value interpolated into a shell string and is
//! POSIX-single-quote-escaped. The same applies to the optional 1Password
//! token reference and the `op` binary path; the token itself is fetched by
//! the pane's own shell and never passes through this process or tmux argv.
//! See docs/contract.md for security design and trade-offs.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

fn tmux_bin() -> String {
    std::env::var("WORK_SHMIRK_TMUX_BIN").unwrap_or_else(|_| "tmux".to_string())
}

fn claude_bin() -> String {
    std::env::var("WORK_SHMIRK_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string())
}

fn op_bin() -> String {
    std::env::var("WORK_SHMIRK_OP_BIN").unwrap_or_else(|_| "op".to_string())
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

/// Build the bottom-left pane's `sh -c` script. With a token reference, the
/// script reads the secret via `op read` (ignoring any inherited service
/// account token so the user's own 1Password session is used), exports it as
/// `OP_SERVICE_ACCOUNT_TOKEN` for claude only, and drops to the user's shell
/// without launching claude if the read fails or yields nothing.
fn bottom_pane_script(claude_q: &str, op_bin: &str, op_token_ref: Option<&str>) -> String {
    let Some(reference) = op_token_ref else {
        return format!("{claude_q}; exec \"$SHELL\"");
    };
    let op_q = shell_single_quote(op_bin);
    let reference_q = shell_single_quote(reference);
    format!(
        concat!(
            "token=$(unset OP_SERVICE_ACCOUNT_TOKEN; {op_q} read {reference_q}) && [ -n \"$token\" ] || ",
            "{{ echo 'work-shmirk: could not read OP_SERVICE_ACCOUNT_TOKEN from 1Password; not starting claude' >&2; exec \"$SHELL\"; }}; ",
            "OP_SERVICE_ACCOUNT_TOKEN=$token; export OP_SERVICE_ACCOUNT_TOKEN; unset token; ",
            "{claude_q}; unset OP_SERVICE_ACCOUNT_TOKEN; exec \"$SHELL\"",
        ),
        op_q = op_q,
        reference_q = reference_q,
        claude_q = claude_q,
    )
}

/// The claude pane lives in the left column, so giving vim 30% of the width
/// leaves the claude pane about 70% of the window width.
const VIM_PANE_WIDTH: &str = "30%";
const CLAUDE_PANE_HEIGHT: &str = "70%";

/// Set up the 3-pane layout. `worktree_target` is the new worktree's absolute
/// path; `window_name` is the value to pass to `tmux rename-window`.
/// `op_token_ref`, when set, is a validated `op://` reference exported as
/// `OP_SERVICE_ACCOUNT_TOKEN` for the claude process.
pub fn setup_panes(
    worktree_target: &Path,
    window_name: &str,
    op_token_ref: Option<&str>,
) -> Result<()> {
    let target_str = worktree_target
        .to_str()
        .context("worktree path is not valid UTF-8")?;

    // Escape only the strings that live inside a `sh -c` payload. The
    // worktree path is passed via tmux `-c` (argv) and does not need escaping.
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
        "-l",
        VIM_PANE_WIDTH,
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

    // Bottom-left pane: split the top-left vertically, launch claude, then
    // exec the user's shell so the pane stays alive after claude exits (`;`
    // rather than `&&` — the user always lands in a shell).
    let bottom_payload = bottom_pane_script(&claude_q, &op_bin(), op_token_ref);
    let bottom_pane_id = run_tmux_capture(&[
        "split-window",
        "-v",
        "-l",
        CLAUDE_PANE_HEIGHT,
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

    #[test]
    fn script_without_reference_runs_claude_then_shell() {
        assert_eq!(
            bottom_pane_script("'claude'", "op", None),
            "'claude'; exec \"$SHELL\""
        );
    }

    #[test]
    fn script_with_reference_quotes_it_and_skips_claude_on_failure() {
        let script = bottom_pane_script("'claude'", "op", Some("op://v/it'em/f"));
        assert!(script.contains("read 'op://v/it'\\''em/f')"));
        assert!(script.contains("not starting claude"));
        assert!(script.contains("export OP_SERVICE_ACCOUNT_TOKEN; unset token; 'claude';"));
    }

    /// Run the generated script under `sh` with stub `op`/`claude`/shell
    /// binaries and return what claude and the final shell observed.
    fn run_script(op_body: &str) -> (String, String, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let op = write("op", op_body);
        let claude = write(
            "claude",
            "echo \"claude:${OP_SERVICE_ACCOUNT_TOKEN-unset}\"",
        );
        let shell = write("shell", "echo \"shell:${OP_SERVICE_ACCOUNT_TOKEN-unset}\"");
        let script = bottom_pane_script(
            &shell_single_quote(claude.to_str().unwrap()),
            op.to_str().unwrap(),
            Some("op://vault/item/credential"),
        );
        let output = Command::new("sh")
            .args(["-c", &script])
            .env("SHELL", &shell)
            .env("OP_SERVICE_ACCOUNT_TOKEN", "stale")
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            script,
        )
    }

    #[test]
    fn script_exports_token_to_claude_only() {
        let (stdout, _, script) = run_script(
            "[ -z \"$OP_SERVICE_ACCOUNT_TOKEN\" ] && [ \"$1\" = read ] && [ \"$2\" = op://vault/item/credential ] && echo s3cret",
        );
        assert_eq!(stdout, "claude:s3cret\nshell:unset\n");
        assert!(!script.contains("s3cret"));
    }

    #[test]
    fn script_skips_claude_when_op_fails() {
        let (stdout, stderr, _) = run_script("exit 1");
        assert_eq!(stdout, "shell:stale\n");
        assert!(stderr.contains("not starting claude"));
    }

    #[test]
    fn script_skips_claude_when_op_returns_nothing() {
        let (stdout, stderr, _) = run_script("exit 0");
        assert_eq!(stdout, "shell:stale\n");
        assert!(stderr.contains("not starting claude"));
    }
}
