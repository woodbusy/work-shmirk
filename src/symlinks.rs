//! Symlink creation and removal per `symlink_dir` + `symlink_links` config.
//!
//! All leading dots are stripped from each entry name: `.env` → `env`,
//! `..env` → `env`, `...env` → `env`. Entries that strip to empty (`.`, `..`,
//! `...`, etc.) and raw empty entries (`""`) are rejected with an error rather
//! than silently skipped.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::ensure_inside;

/// Strip all leading `.` characters from `name`.
/// e.g. `.env` → `env`, `..env` → `env`, `env` → `env`.
pub fn strip_leading_dots(name: &str) -> &str {
    name.trim_start_matches('.')
}

/// Create symlinks for each `link_source` in `links` under `<symlink_dir>/<name>`.
/// `link_source` is interpreted relative to `worktree_target`.
///
/// All leading dots are stripped from the link name (e.g. `.env` and `..env`
/// both become `env`). Entries that are empty or that strip to empty are
/// rejected with an error.
///
/// Path validation:
///   - `dest` (the link path under symlink_dir/name) is required to remain inside
///     the per-worktree link dir (`<symlink_dir_base>/<name>`). Validating against
///     the per-worktree subdir (not the top-level base) prevents `strip_leading_dots`
///     from producing a dest that escapes the per-worktree subdir while still
///     normalizing back inside `symlink_dir_base`.
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
            anyhow::bail!("symlink_links entry is empty");
        }
        let stripped = strip_leading_dots(link_source);
        if stripped.is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
        let dest = worktree_link_dir.join(stripped);
        ensure_inside(&worktree_link_dir, &dest).with_context(|| {
            format!(
                "validating symlink dest for '{link_source}' under {}",
                worktree_link_dir.display()
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
            anyhow::bail!("symlink_links entry is empty");
        }
        let stripped = strip_leading_dots(link_source);
        if stripped.is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
        let candidate = worktree_link_dir.join(stripped);
        // Reject candidates that escape the per-worktree link dir before
        // touching the filesystem. Without this guard a hostile config could
        // drive `remove_file` against an arbitrary symlink outside the
        // configured symlink dir at remove time.
        ensure_inside(&worktree_link_dir, &candidate).with_context(|| {
            format!(
                "validating symlink dest for '{link_source}' under {}",
                worktree_link_dir.display()
            )
        })?;
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
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn strip_all_leading_dots() {
        assert_eq!(strip_leading_dots(".env"), "env");
        assert_eq!(strip_leading_dots("env"), "env");
        assert_eq!(strip_leading_dots("..env"), "env");
        assert_eq!(strip_leading_dots("...env"), "env");
        assert_eq!(strip_leading_dots(""), "");
        assert_eq!(strip_leading_dots("."), "");
        assert_eq!(strip_leading_dots("..."), "");
    }

    fn make_worktree_with_file(filename: &str) -> (TempDir, TempDir) {
        let worktree_dir = TempDir::new().unwrap();
        let symlink_base = TempDir::new().unwrap();
        std::fs::write(worktree_dir.path().join(filename), b"").unwrap();
        (worktree_dir, symlink_base)
    }

    #[test]
    fn create_symlinks_rejects_empty_entry() {
        let (worktree_dir, symlink_base) = make_worktree_with_file(".env");
        let links = vec![String::from("")];
        let err = create_symlinks(
            worktree_dir.path(),
            symlink_base.path(),
            "myworktree",
            &links,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("symlink_links entry is empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn remove_symlinks_rejects_empty_entry() {
        let symlink_base = TempDir::new().unwrap();
        let worktree_link_dir = symlink_base.path().join("myworktree");
        std::fs::create_dir_all(&worktree_link_dir).unwrap();
        let links = vec![String::from("")];
        let err = remove_symlinks(symlink_base.path(), "myworktree", &links).unwrap_err();
        assert!(
            err.to_string().contains("symlink_links entry is empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_symlinks_rejects_all_dot_entry() {
        let (worktree_dir, symlink_base) = make_worktree_with_file(".env");
        for all_dot in &["..", "...", "."] {
            let links = vec![String::from(*all_dot)];
            let err = create_symlinks(
                worktree_dir.path(),
                symlink_base.path(),
                "myworktree",
                &links,
            )
            .unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("symlink_links entry strips to empty name"),
                "expected 'strips to empty name' for input '{all_dot}', got: {msg}"
            );
            assert!(
                msg.contains(*all_dot),
                "expected original input '{all_dot}' in error, got: {msg}"
            );
        }
    }

    #[test]
    fn remove_symlinks_rejects_all_dot_entry() {
        let symlink_base = TempDir::new().unwrap();
        let worktree_link_dir = symlink_base.path().join("myworktree");
        std::fs::create_dir_all(&worktree_link_dir).unwrap();
        // Place a dummy symlink so the dir-exists check passes
        symlink(worktree_dir_placeholder(), worktree_link_dir.join("env")).ok();
        for all_dot in &["..", "..."] {
            let links = vec![String::from(*all_dot)];
            let err = remove_symlinks(symlink_base.path(), "myworktree", &links).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("symlink_links entry strips to empty name"),
                "expected 'strips to empty name' for input '{all_dot}', got: {msg}"
            );
            assert!(
                msg.contains(*all_dot),
                "expected original input '{all_dot}' in error, got: {msg}"
            );
        }
    }

    fn worktree_dir_placeholder() -> std::path::PathBuf {
        std::path::PathBuf::from("/dev/null")
    }
}
