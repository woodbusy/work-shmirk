//! Symlink creation, re-creation and removal per `symlink_dir` + `symlink_links` config.
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
pub(crate) fn strip_leading_dots(name: &str) -> &str {
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

    for link_source in links {
        if link_source.is_empty() {
            anyhow::bail!("symlink_links entry is empty");
        }
        if strip_leading_dots(link_source).is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
    }

    std::fs::create_dir_all(&worktree_link_dir)
        .with_context(|| format!("creating symlink dir {}", worktree_link_dir.display()))?;

    for link_source in links {
        let stripped = strip_leading_dots(link_source);
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

/// One `symlink_links` entry that passed validation: where its link goes,
/// relative to the per-worktree link dir, and what it points at.
struct PlannedLink<'a> {
    entry: &'a str,
    relative_dest: PathBuf,
    source: PathBuf,
}

/// Re-create the symlinks `create_symlinks` makes, for an existing worktree,
/// replacing any symlink already at a destination. Used by `relink`, whose
/// config may come from the worktree itself, so every write is guarded:
///
/// 1. Validate everything before writing anything: entry names, lexical
///    containment of each dest in `<symlink_dir_base>/<name>` and each source
///    in `worktree`, that `symlink_dir_base` already exists as a directory
///    (it is never created), and that the per-worktree link dir and the
///    worktree do not overlap in either direction once symlinks are resolved.
/// 2. Apply. The link dir must be absent (created, one level only) or a real
///    directory; a symlink or non-directory there aborts the run. Every
///    intermediate component of a dest must already be a real directory, and
///    only an absent dest or a symlink dest is written. Other obstructions
///    skip that entry, are reported on stderr, and make the run fail after
///    the remaining entries are linked.
pub fn relink_symlinks(
    worktree: &Path,
    symlink_dir_base: &Path,
    name: &str,
    links: &[String],
) -> Result<()> {
    let lexical_link_dir = symlink_dir_base.join(name);
    let normalized_link_dir = ensure_inside(&lexical_link_dir, &lexical_link_dir)?;

    let mut planned: Vec<PlannedLink> = Vec::with_capacity(links.len());
    for link_source in links {
        if link_source.is_empty() {
            anyhow::bail!("symlink_links entry is empty");
        }
        let stripped = strip_leading_dots(link_source);
        if stripped.is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
        let normalized_dest = ensure_inside(&lexical_link_dir, &lexical_link_dir.join(stripped))
            .with_context(|| {
                format!(
                    "validating symlink dest for '{link_source}' under {}",
                    lexical_link_dir.display()
                )
            })?;
        let relative_dest = normalized_dest
            .strip_prefix(&normalized_link_dir)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        if relative_dest.as_os_str().is_empty() {
            anyhow::bail!(
                "symlink_links entry '{link_source}' resolves to the link dir {} itself",
                lexical_link_dir.display()
            );
        }
        ensure_inside(worktree, Path::new(link_source)).with_context(|| {
            format!("validating symlink source '{link_source}' inside worktree")
        })?;
        planned.push(PlannedLink {
            entry: link_source,
            relative_dest,
            source: worktree.join(link_source),
        });
    }

    let resolved_base = symlink_dir_base.canonicalize().map_err(|_| {
        anyhow::anyhow!(
            "symlink_dir {} does not exist; create it before running relink",
            symlink_dir_base.display()
        )
    })?;
    if !resolved_base.is_dir() {
        anyhow::bail!(
            "symlink_dir {} is not a directory",
            symlink_dir_base.display()
        );
    }
    let link_dir = resolved_base.join(name);
    let resolved_worktree = worktree
        .canonicalize()
        .with_context(|| format!("resolving worktree {}", worktree.display()))?;
    if link_dir.starts_with(&resolved_worktree) || resolved_worktree.starts_with(&link_dir) {
        anyhow::bail!(
            "refusing to relink: the link dir {} overlaps the worktree {}",
            link_dir.display(),
            resolved_worktree.display()
        );
    }

    match std::fs::symlink_metadata(&link_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&link_dir)
                .with_context(|| format!("creating symlink dir {}", link_dir.display()))?;
        }
        Err(e) => {
            return Err(e).with_context(|| format!("probing link dir {}", link_dir.display()))
        }
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => anyhow::bail!(
            "refusing to relink: the link dir {} is a symlink or not a directory",
            link_dir.display()
        ),
    }

    println!("Relinking symlinks in {}", link_dir.display());

    let mut skipped: Vec<&str> = Vec::new();
    for link in &planned {
        match relink_one(&link_dir, link) {
            Ok(()) => {}
            Err(reason) => {
                eprintln!("skipping '{}': {reason:#}", link.entry);
                skipped.push(link.entry);
            }
        }
    }

    if !skipped.is_empty() {
        anyhow::bail!(
            "skipped {} symlink_links entr{}: {}",
            skipped.len(),
            if skipped.len() == 1 { "y" } else { "ies" },
            skipped.join(", ")
        );
    }
    Ok(())
}

/// Link one validated entry under `link_dir`. An `Err` means the entry was
/// skipped with nothing written; I/O errors from the write itself are
/// reported the same way.
fn relink_one(link_dir: &Path, link: &PlannedLink) -> Result<()> {
    let mut dest = link_dir.to_path_buf();
    let mut components = link.relative_dest.components().peekable();
    while let Some(component) = components.next() {
        dest.push(component);
        if components.peek().is_none() {
            break;
        }
        match std::fs::symlink_metadata(&dest) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => anyhow::bail!(
                "{} is a symlink or not a directory; relink creates no intermediate directories",
                dest.display()
            ),
            Err(_) => anyhow::bail!(
                "{} does not exist; relink creates no intermediate directories",
                dest.display()
            ),
        }
    }

    match std::fs::symlink_metadata(&dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("probing {}", dest.display())),
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::remove_file(&dest)
                .with_context(|| format!("removing stale symlink {}", dest.display()))?;
        }
        Ok(_) => anyhow::bail!("{} exists and is not a symlink; left as is", dest.display()),
    }

    if std::fs::symlink_metadata(&link.source).is_err() {
        eprintln!(
            "warning: source {} does not exist; the link will dangle",
            link.source.display()
        );
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(&link.source, &dest).with_context(|| {
        format!(
            "creating symlink {} -> {}",
            dest.display(),
            link.source.display()
        )
    })?;
    println!("Linked {} -> {}", dest.display(), link.source.display());
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
        for all_dot in &["..", "...", "."] {
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

    struct RelinkFixture {
        worktree: PathBuf,
        base: PathBuf,
        outside: PathBuf,
        _tmp: TempDir,
    }

    impl RelinkFixture {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            let root = tmp.path().canonicalize().unwrap();
            let worktree = root.join("wt");
            let base = root.join("links");
            let outside = root.join("outside");
            for dir in [&worktree, &base, &outside] {
                std::fs::create_dir(dir).unwrap();
            }
            std::fs::write(worktree.join(".env"), "env").unwrap();
            Self {
                worktree,
                base,
                outside,
                _tmp: tmp,
            }
        }

        fn link_dir(&self) -> PathBuf {
            self.base.join("wt")
        }

        fn relink(&self, links: &[&str]) -> Result<()> {
            let links: Vec<String> = links.iter().map(|s| s.to_string()).collect();
            relink_symlinks(&self.worktree, &self.base, "wt", &links)
        }
    }

    #[test]
    fn relink_creates_link_dir_and_links() {
        let fx = RelinkFixture::new();
        fx.relink(&[".env"]).unwrap();
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("env")).unwrap(),
            fx.worktree.join(".env")
        );
    }

    #[test]
    fn relink_replaces_symlink_pointing_elsewhere() {
        let fx = RelinkFixture::new();
        std::fs::create_dir(fx.link_dir()).unwrap();
        symlink(fx.outside.join("nowhere"), fx.link_dir().join("env")).unwrap();
        fx.relink(&[".env"]).unwrap();
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("env")).unwrap(),
            fx.worktree.join(".env")
        );
    }

    #[test]
    fn relink_skips_regular_file_and_links_the_rest() {
        let fx = RelinkFixture::new();
        std::fs::create_dir(fx.link_dir()).unwrap();
        std::fs::write(fx.link_dir().join("keep"), "mine").unwrap();
        let err = fx.relink(&["keep", ".env"]).unwrap_err();
        assert!(err.to_string().contains("keep"), "unexpected error: {err}");
        assert_eq!(
            std::fs::read_to_string(fx.link_dir().join("keep")).unwrap(),
            "mine"
        );
        assert!(is_symlink(&fx.link_dir().join("env")));
    }

    #[test]
    fn relink_refuses_symlinked_link_dir() {
        let fx = RelinkFixture::new();
        symlink(&fx.outside, fx.link_dir()).unwrap();
        let err = fx.relink(&[".env"]).unwrap_err();
        assert!(
            err.to_string().contains("is a symlink or not a directory"),
            "unexpected error: {err}"
        );
        assert!(!fx.outside.join("env").exists());
    }

    #[test]
    fn relink_skips_symlinked_intermediate_component() {
        let fx = RelinkFixture::new();
        std::fs::create_dir(fx.link_dir()).unwrap();
        symlink(&fx.outside, fx.link_dir().join("claude")).unwrap();
        let err = fx.relink(&[".claude/skills"]).unwrap_err();
        assert!(
            err.to_string().contains(".claude/skills"),
            "unexpected error: {err}"
        );
        assert!(std::fs::symlink_metadata(fx.outside.join("skills")).is_err());
    }

    #[test]
    fn relink_skips_absent_intermediate_component() {
        let fx = RelinkFixture::new();
        let err = fx.relink(&["sub/file"]).unwrap_err();
        assert!(
            err.to_string().contains("sub/file"),
            "unexpected error: {err}"
        );
        assert!(!fx.link_dir().join("sub").exists());
    }

    #[test]
    fn relink_rejects_escape_before_writing() {
        let fx = RelinkFixture::new();
        let err = fx.relink(&[".env", "../../escape"]).unwrap_err();
        assert!(
            format!("{err:#}").contains("validating symlink dest"),
            "unexpected error: {err:#}"
        );
        assert!(std::fs::symlink_metadata(fx.link_dir()).is_err());
    }

    #[test]
    fn relink_rejects_entry_resolving_to_link_dir() {
        let fx = RelinkFixture::new();
        let err = fx.relink(&[".a/.."]).unwrap_err();
        assert!(
            err.to_string().contains("resolves to the link dir"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn relink_refuses_missing_base_without_creating_it() {
        let fx = RelinkFixture::new();
        let missing = fx.outside.join("missing");
        let links = vec![String::from(".env")];
        let err = relink_symlinks(&fx.worktree, &missing, "wt", &links).unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "unexpected error: {err}"
        );
        assert!(!missing.exists());
    }

    #[test]
    fn relink_refuses_link_dir_overlapping_worktree() {
        let fx = RelinkFixture::new();
        let parent_of_worktree = fx.worktree.parent().unwrap().to_path_buf();
        symlink(fx.outside.join("x"), fx.worktree.join("env")).unwrap();
        let links = vec![String::from(".env")];
        let err = relink_symlinks(&fx.worktree, &parent_of_worktree, "wt", &links).unwrap_err();
        assert!(
            err.to_string().contains("overlaps the worktree"),
            "unexpected error: {err}"
        );
        assert_eq!(
            std::fs::read_link(fx.worktree.join("env")).unwrap(),
            fx.outside.join("x")
        );
    }

    fn worktree_dir_placeholder() -> std::path::PathBuf {
        std::path::PathBuf::from("/dev/null")
    }
}
