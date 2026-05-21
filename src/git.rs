//! Thin wrappers around the `git` CLI. Stdio is inherited so the user sees
//! progress/errors directly. Each call takes a `cwd` so tests can drive it.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn git_bin() -> String {
    std::env::var("WORK_SHMIRK_GIT_BIN").unwrap_or_else(|_| "git".to_string())
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(git_bin())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .with_context(|| format!("invoking git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    let stdout = String::from_utf8(output.stdout).context("git produced non-UTF8 output")?;
    Ok(stdout.trim().to_string())
}

fn run_git_status(cwd: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new(git_bin())
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("invoking git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} exited with status {}", args.join(" "), status);
    }
    Ok(())
}

pub fn show_toplevel(cwd: &Path) -> Result<PathBuf> {
    let s = run_git_capture(cwd, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(s))
}

pub fn git_common_dir(cwd: &Path) -> Result<PathBuf> {
    let s = run_git_capture(cwd, &["rev-parse", "--git-common-dir"])?;
    let path = PathBuf::from(&s);
    // `--git-common-dir` may return a relative path; resolve it against cwd.
    Ok(if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    })
}

pub fn worktrees_root(cwd: &Path) -> Result<PathBuf> {
    let common = git_common_dir(cwd)?;
    Ok(common
        .parent()
        .ok_or_else(|| anyhow!("git common dir {} has no parent", common.display()))?
        .to_path_buf())
}

pub fn worktree_add(cwd: &Path, path: &Path, name: &str, existing: bool) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {}", path.display()))?;
    let args: Vec<&str> = if existing {
        vec!["worktree", "add", path_str, name]
    } else {
        vec!["worktree", "add", path_str, "-b", name]
    };
    run_git_status(cwd, &args)
}

pub fn worktree_remove(cwd: &Path, path: &Path) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {}", path.display()))?;
    run_git_status(cwd, &["worktree", "remove", path_str])
}

pub fn branch_delete_force(cwd: &Path, name: &str) -> Result<()> {
    run_git_status(cwd, &["branch", "-D", name])
}
