//! Symlink creation and removal per `symlink_dir` + `symlink_links` config.
//!
//! Reproduces the bash one-leading-dot strip (`${link_source#.}`): a single
//! leading `.` is removed, not all of them. e.g. `.env` → `env`, `..foo` → `.foo`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::ensure_inside;

/// Strip exactly one leading `.` from `name`. Matches bash `${name#.}`.
pub fn strip_one_dot(name: &str) -> &str {
    name.strip_prefix('.').unwrap_or(name)
}

/// Create symlinks for each `link_source` in `links` under `<symlink_dir>/<name>`.
/// `link_source` is interpreted relative to `worktree_target`.
///
/// Path validation:
///   - `dest` (the link path under symlink_dir/name) is required to remain inside `symlink_dir_base`.
///   - `link_source` is required to remain inside `worktree_target`.
pub fn create_symlinks(
    worktree_target: &Path,
    symlink_dir_base: &Path,
    name: &str,
    links: &[String],
) -> Result<()> {
    let worktree_link_dir = symlink_dir_base.join(name);
    std::fs::create_dir_all(&worktree_link_dir)
        .with_context(|| format!("creating symlink dir {}", worktree_link_dir.display()))?;

    for link_source in links {
        if link_source.is_empty() {
            continue;
        }
        let stripped = strip_one_dot(link_source);
        let dest = worktree_link_dir.join(stripped);
        ensure_inside(symlink_dir_base, &dest).with_context(|| {
            format!(
                "validating symlink dest for '{link_source}' under {}",
                symlink_dir_base.display()
            )
        })?;

        let source_abs = worktree_target.join(link_source);
        ensure_inside(worktree_target, Path::new(link_source)).with_context(|| {
            format!("validating symlink source '{link_source}' inside worktree")
        })?;

        #[cfg(unix)]
        std::os::unix::fs::symlink(&source_abs, &dest).with_context(|| {
            format!(
                "creating symlink {} -> {}",
                dest.display(),
                source_abs.display()
            )
        })?;
    }
    Ok(())
}

/// Remove symlinks created by `create_symlinks`. Echoes progress to stdout.
/// Returns the directory we tried to clean up.
pub fn remove_symlinks(symlink_dir_base: &Path, name: &str, links: &[String]) -> Result<()> {
    let worktree_link_dir: PathBuf = symlink_dir_base.join(name);

    // Bash echoes only when the dir/symlink exists.
    let dir_exists = worktree_link_dir.exists() || worktree_link_dir.symlink_metadata().is_ok();
    if !dir_exists {
        return Ok(());
    }

    println!("Removing symlinks from {}", worktree_link_dir.display());

    for link_source in links {
        if link_source.is_empty() {
            continue;
        }
        let stripped = strip_one_dot(link_source);
        let candidate = worktree_link_dir.join(stripped);
        println!("Removing {}", candidate.display());
        if is_symlink(&candidate) {
            std::fs::remove_file(&candidate)
                .with_context(|| format!("removing symlink {}", candidate.display()))?;
        }
    }

    if is_symlink(&worktree_link_dir) {
        std::fs::remove_file(&worktree_link_dir)
            .with_context(|| format!("removing symlink {}", worktree_link_dir.display()))?;
    } else if worktree_link_dir.is_dir() {
        std::fs::remove_dir(&worktree_link_dir)
            .with_context(|| format!("removing directory {}", worktree_link_dir.display()))?;
    }

    Ok(())
}

fn is_symlink(p: &Path) -> bool {
    std::fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_one_leading_dot() {
        assert_eq!(strip_one_dot(".env"), "env");
        assert_eq!(strip_one_dot("env"), "env");
        assert_eq!(strip_one_dot("..env"), ".env"); // single strip only
        assert_eq!(strip_one_dot(""), "");
        assert_eq!(strip_one_dot("."), "");
    }
}
