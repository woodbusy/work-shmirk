//! `personal_issues` integration for the `new` flow: runs
//! `personal-issues set-current <issue-id>` in the new worktree.
//!
//! Must run after `copyfiles::copy_dev_workflow_config`, because
//! `personal-issues` reads its backend configuration from that file.

use anyhow::{bail, Context, Result};
use std::os::unix::io::AsFd;
use std::path::Path;
use std::process::{Command, Stdio};

fn personal_issues_bin() -> String {
    std::env::var("WORK_SHMIRK_PERSONAL_ISSUES_BIN")
        .unwrap_or_else(|_| "personal-issues".to_string())
}

/// Set up `personal-issues` in the new worktree. `issue_id` is `None` when
/// the branch name carries no issue reference; `set-current` is then skipped
/// with a warning.
pub fn setup(worktree_target: &Path, issue_id: Option<&str>) -> Result<()> {
    let Some(issue_id) = issue_id else {
        eprintln!(
            "Warning: no issue reference in branch name; skipping personal-issues set-current"
        );
        return Ok(());
    };
    set_current(worktree_target, issue_id)
}

fn set_current(worktree_target: &Path, issue_id: &str) -> Result<()> {
    // Route stdout to our stderr: the binary's stdout is reserved for the
    // worktree path the shell wrapper captures.
    let stderr_fd = std::io::stderr().as_fd().try_clone_to_owned()?;
    let status = Command::new(personal_issues_bin())
        .args(["set-current", issue_id])
        .current_dir(worktree_target)
        .stdin(Stdio::null())
        .stdout(stderr_fd)
        .stderr(Stdio::inherit())
        .status()
        .context("invoking personal-issues set-current")?;
    if !status.success() {
        bail!("personal-issues set-current {issue_id} exited with {status}");
    }
    Ok(())
}
