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

/// Record in the worktree's local git config that `work-shmirk` created the
/// branch.  Writes `work-shmirk.owned-branch = <name>` into
/// `.git/worktrees/<name>/config` (the per-worktree config file), which is
/// not touched by `git clean` and requires git-level access to forge.
///
/// `worktree_path` must be the absolute path to the worktree directory.
pub fn set_worktree_owned_branch(worktree_path: &Path, name: &str) -> Result<()> {
    run_git_status(
        worktree_path,
        &["config", "--local", "work-shmirk.owned-branch", name],
    )
}

/// Read `work-shmirk.owned-branch` from the worktree-local git config.
/// Returns `Ok(Some(name))` when the key is set, `Ok(None)` when absent,
/// and `Err` only for unexpected failures (git binary missing, etc.).
///
/// `worktree_path` must be the absolute path to the worktree directory.
pub fn get_worktree_owned_branch(worktree_path: &Path) -> Result<Option<String>> {
    let output = Command::new(git_bin())
        .args(["config", "--local", "work-shmirk.owned-branch"])
        .current_dir(worktree_path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .with_context(|| "invoking git config --local work-shmirk.owned-branch".to_string())?;

    if output.status.success() {
        let value = String::from_utf8(output.stdout)
            .context("git config produced non-UTF8 output")?
            .trim()
            .to_string();
        Ok(Some(value))
    } else {
        // Exit code 1 means the key is not set; any other code is an error.
        match output.status.code() {
            Some(1) => Ok(None),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!(
                    "git config --local work-shmirk.owned-branch failed: {}",
                    stderr.trim()
                )
            }
        }
    }
}

/// Return the branch name attached to a given worktree path, with the
/// `refs/heads/` prefix stripped.  Returns `Ok(None)` for detached HEAD,
/// bare worktrees, or when no matching record is found.  If
/// `worktree_path` cannot be canonicalized (e.g. the directory has already
/// been removed), returns `Ok(None)` so callers can safely fall back to the
/// sentinel gate alone.
pub fn worktree_branch(cwd: &Path, worktree_path: &Path) -> Result<Option<String>> {
    let canonical_target = match worktree_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };

    let output = run_git_capture(cwd, &["worktree", "list", "--porcelain"])?;

    // Records are separated by blank lines.  Each record has one or more
    // key-value lines.  We look for the record whose `worktree <path>`
    // canonicalizes to our target, then return its `branch` value.
    for record in output.split("\n\n") {
        let mut record_path: Option<std::path::PathBuf> = None;
        let mut record_branch: Option<String> = None;

        for line in record.lines() {
            if let Some(path_str) = line.strip_prefix("worktree ") {
                record_path = Some(std::path::PathBuf::from(path_str));
            } else if let Some(branch_ref) = line.strip_prefix("branch ") {
                if let Some(name) = branch_ref.strip_prefix("refs/heads/") {
                    record_branch = Some(name.to_string());
                }
                // `detached` lines (no "branch" key) leave record_branch as None.
            }
        }

        if let Some(rp) = record_path {
            if let Ok(canonical_rp) = rp.canonicalize() {
                if canonical_rp == canonical_target {
                    return Ok(record_branch);
                }
            }
        }
    }

    Ok(None)
}
