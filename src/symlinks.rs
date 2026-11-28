//! Symlink creation, re-creation and removal per `symlink_dir` + `symlink_links` config.
//!
//! All leading dots are stripped from each entry name: `.env` → `env`,
//! `..env` → `env`, `...env` → `env`. Entries that strip to empty (`.`, `..`,
//! `...`, etc.) and raw empty entries (`""`) are rejected with an error rather
//! than silently skipped.

use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

use crate::config::ensure_inside;

/// Strip all leading `.` characters from `name`.
/// e.g. `.env` → `env`, `..env` → `env`, `env` → `env`.
pub(crate) fn strip_leading_dots(name: &str) -> &str {
    name.trim_start_matches('.')
}

/// Where `stripped` (a dot-stripped entry name) puts its link, relative to
/// `lexical_link_dir`, once `.` and `..` are resolved lexically. `new`,
/// `remove` and `relink` all act on this path, so none of them hands the
/// kernel a `..` to resolve through a symlinked component. Errors when the
/// entry escapes the link dir.
fn normalized_relative_dest(
    lexical_link_dir: &Path,
    entry: &str,
    stripped: &str,
) -> Result<PathBuf> {
    let normalized_link_dir = ensure_inside(lexical_link_dir, lexical_link_dir)?;
    let normalized_dest = ensure_inside(lexical_link_dir, &lexical_link_dir.join(stripped))
        .with_context(|| {
            format!(
                "validating symlink dest for '{entry}' under {}",
                lexical_link_dir.display()
            )
        })?;
    Ok(normalized_dest
        .strip_prefix(&normalized_link_dir)
        .map(Path::to_path_buf)
        .unwrap_or_default())
}

/// Create symlinks for each `link_source` in `links` under `<symlink_dir>/<name>`.
/// `link_source` is interpreted relative to `worktree_target`.
///
/// All leading dots are stripped from the link name (e.g. `.env` and `..env`
/// both become `env`). Entries that are empty or that strip to empty are
/// rejected with an error. A nested entry keeps its path below the link dir:
/// `.claude/skills` links at `<link_dir>/claude/skills`.
///
/// Path validation:
///   - `dest` (the link path under symlink_dir/name) is required to remain inside
///     the per-worktree link dir (`<symlink_dir_base>/<name>`). Validating against
///     the per-worktree subdir (not the top-level base) prevents `strip_leading_dots`
///     from producing a dest that escapes the per-worktree subdir while still
///     normalizing back inside `symlink_dir_base`. The link goes at the
///     normalized path, so `x/../env` links at `<link_dir>/env`.
///   - Missing intermediate directories of a dest are created one level at a
///     time; an intermediate that is a symlink or not a directory fails the
///     run, and nothing is created through it.
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
        let relative_dest = normalized_relative_dest(&worktree_link_dir, link_source, stripped)?;

        let source_abs = worktree_target.join(link_source);
        ensure_inside(worktree_target, Path::new(link_source)).with_context(|| {
            format!("validating symlink source '{link_source}' inside worktree")
        })?;

        let dest = walk_intermediates(
            &worktree_link_dir,
            &relative_dest,
            WalkMode::CreateAbsent,
            &mut Vec::new(),
        )
        .map_err(|stop| anyhow::anyhow!("creating symlink for '{link_source}': {stop}"))?;

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

/// Remove what `create_symlinks` made. Echoes progress to stdout. The config
/// may come from the worktree itself, so teardown never follows a symlink and
/// deletes only symlinks and empty real directories:
///
/// 1. Validate every entry before deleting anything. A link dir that is a
///    symlink is then unlinked, and nothing is probed through it.
/// 2. Walk each entry's intermediate components without following symlinks,
///    and remove the dest only if it is a symlink. An entry whose walk stops
///    at a symlink or non-directory is left alone and reported on stderr.
/// 3. Remove the real directories the walks verified, deepest first, when
///    they are empty.
/// 4. Remove the link dir with a non-recursive `remove_dir`, which fails when
///    anything was left in it.
pub fn remove_symlinks(symlink_dir_base: &Path, name: &str, links: &[String]) -> Result<()> {
    let worktree_link_dir: PathBuf = symlink_dir_base.join(name);

    let Ok(link_dir_meta) = std::fs::symlink_metadata(&worktree_link_dir) else {
        return Ok(());
    };

    println!("Removing symlinks from {}", worktree_link_dir.display());

    let mut planned: Vec<(&str, PathBuf)> = Vec::with_capacity(links.len());
    for link_source in links {
        if link_source.is_empty() {
            anyhow::bail!("symlink_links entry is empty");
        }
        let stripped = strip_leading_dots(link_source);
        if stripped.is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
        // Reject candidates that escape the per-worktree link dir before
        // touching the filesystem. Without this guard a hostile config could
        // drive `remove_file` against an arbitrary symlink outside the
        // configured symlink dir at remove time.
        let relative_dest = normalized_relative_dest(&worktree_link_dir, link_source, stripped)?;
        planned.push((link_source, relative_dest));
    }

    if link_dir_meta.file_type().is_symlink() {
        std::fs::remove_file(&worktree_link_dir)
            .with_context(|| format!("removing symlink {}", worktree_link_dir.display()))?;
        return Ok(());
    }
    if !link_dir_meta.file_type().is_dir() {
        return Ok(());
    }

    let mut verified_dirs: Vec<PathBuf> = Vec::new();
    for (link_source, relative_dest) in &planned {
        let candidate = match walk_intermediates(
            &worktree_link_dir,
            relative_dest,
            WalkMode::StopAtAbsent,
            &mut verified_dirs,
        ) {
            Ok(candidate) => candidate,
            Err(IntermediateWalkError::Absent(_)) => continue,
            Err(stop @ IntermediateWalkError::NotARealDirectory(_)) => {
                eprintln!("leaving '{link_source}': {stop}");
                continue;
            }
            Err(IntermediateWalkError::Io(io_error)) => {
                return Err(io_error.context(format!("removing symlink for '{link_source}'")))
            }
        };
        println!("Removing {}", candidate.display());
        if is_symlink(&candidate) {
            std::fs::remove_file(&candidate)
                .with_context(|| format!("removing symlink {}", candidate.display()))?;
        }
    }

    verified_dirs.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });
    verified_dirs.dedup();
    for directory in &verified_dirs {
        if is_empty_real_directory(directory)? {
            std::fs::remove_dir(directory)
                .with_context(|| format!("removing directory {}", directory.display()))?;
        }
    }

    std::fs::remove_dir(&worktree_link_dir)
        .with_context(|| format!("removing directory {}", worktree_link_dir.display()))?;

    Ok(())
}

/// Whether `directory` is still a real directory (not a symlink) with no
/// entries. Uses `read_dir` rather than matching `remove_dir`'s
/// `DirectoryNotEmpty`, which is newer than the crate's MSRV.
fn is_empty_real_directory(directory: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(directory) {
        Ok(meta) if meta.file_type().is_dir() => {}
        _ => return Ok(false),
    }
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("reading directory {}", directory.display()))?;
    Ok(entries.next().is_none())
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
///    in `worktree`, no `..` component in the stripped name (so an untrusted
///    name stays a plain downward path), that `symlink_dir_base`
///    already exists as a directory (it is never created), and that the per-worktree link dir and the
///    worktree do not overlap in either direction once symlinks are resolved.
/// 2. Apply. The link dir must be absent (created, one level only) or a real
///    directory; a symlink or non-directory there aborts the run. Each
///    intermediate component of a dest must be a real directory; an absent
///    one is created with a single-level `create_dir`, while a symlink or
///    non-directory skips the entry. Only an absent dest or a symlink dest is
///    written. Other obstructions skip that entry, are reported on stderr,
///    and make the run fail after the remaining entries are linked.
pub fn relink_symlinks(
    worktree: &Path,
    symlink_dir_base: &Path,
    name: &str,
    links: &[String],
) -> Result<()> {
    let lexical_link_dir = symlink_dir_base.join(name);

    let mut planned: Vec<PlannedLink> = Vec::with_capacity(links.len());
    for link_source in links {
        if link_source.is_empty() {
            anyhow::bail!("symlink_links entry is empty");
        }
        let stripped = strip_leading_dots(link_source);
        if stripped.is_empty() {
            anyhow::bail!("symlink_links entry strips to empty name: '{link_source}'");
        }
        let relative_dest = normalized_relative_dest(&lexical_link_dir, link_source, stripped)?;
        ensure_inside(worktree, Path::new(link_source)).with_context(|| {
            format!("validating symlink source '{link_source}' inside worktree")
        })?;
        if Path::new(stripped)
            .components()
            .any(|component| component == Component::ParentDir)
        {
            anyhow::bail!(
                "symlink_links entry '{link_source}' contains a '..' component; relink rejects it to keep an untrusted name a plain downward path"
            );
        }
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

/// Link one validated entry under `link_dir`, creating any absent
/// intermediate directory one level at a time, and only below components
/// verified to be real directories. An `Err` means the entry was skipped with
/// nothing written through a symlink or a non-directory; intermediates
/// already created for it stay behind as empty real directories, which
/// `remove` tears down. I/O errors from the write itself are reported the
/// same way.
fn relink_one(link_dir: &Path, link: &PlannedLink) -> Result<()> {
    let dest = match walk_intermediates(
        link_dir,
        &link.relative_dest,
        WalkMode::CreateAbsent,
        &mut Vec::new(),
    ) {
        Ok(dest) => dest,
        Err(IntermediateWalkError::Io(io_error)) => return Err(io_error),
        Err(stop) => anyhow::bail!("{stop}"),
    };

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

/// Why [`walk_intermediates`] stopped before reaching the dest. The `Display`
/// form is the short, command-neutral message; callers add their own context.
#[derive(Debug)]
enum IntermediateWalkError {
    /// An intermediate is a symlink or some other non-directory.
    NotARealDirectory(PathBuf),
    /// An intermediate does not exist.
    Absent(PathBuf),
    /// Probing an intermediate failed for a reason other than its absence,
    /// or creating an absent one failed.
    Io(anyhow::Error),
}

impl std::fmt::Display for IntermediateWalkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARealDirectory(path) => {
                write!(
                    formatter,
                    "{} is a symlink or not a directory",
                    path.display()
                )
            }
            Self::Absent(path) => write!(formatter, "{} does not exist", path.display()),
            Self::Io(io_error) => write!(formatter, "{io_error:#}"),
        }
    }
}

/// What [`walk_intermediates`] does at an absent intermediate.
#[derive(Clone, Copy)]
enum WalkMode {
    /// Create it with a single-level `create_dir` and carry on.
    CreateAbsent,
    /// Stop with [`IntermediateWalkError::Absent`], having written nothing.
    StopAtAbsent,
}

/// Walk every component of `relative_dest` except the last, starting at
/// `link_dir`, and return the full dest path once each of them has been
/// verified (or, in [`WalkMode::CreateAbsent`], created) as a real directory.
/// Uses `symlink_metadata` only, so no symlink is ever followed; the walk
/// stops at the first intermediate that is a symlink or a non-directory, and
/// at an absent one unless creating it. Every real directory verified or
/// created is appended to `verified_dirs`, shallowest first, including on a
/// stop.
fn walk_intermediates(
    link_dir: &Path,
    relative_dest: &Path,
    mode: WalkMode,
    verified_dirs: &mut Vec<PathBuf>,
) -> std::result::Result<PathBuf, IntermediateWalkError> {
    let mut dest = link_dir.to_path_buf();
    let mut components = relative_dest.components().peekable();
    while let Some(component) = components.next() {
        dest.push(component);
        if components.peek().is_none() {
            break;
        }
        match std::fs::symlink_metadata(&dest) {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(IntermediateWalkError::NotARealDirectory(dest)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match mode {
                WalkMode::StopAtAbsent => return Err(IntermediateWalkError::Absent(dest)),
                WalkMode::CreateAbsent => {
                    if let Err(e) = std::fs::create_dir(&dest) {
                        let context = format!("creating directory {}", dest.display());
                        return Err(IntermediateWalkError::Io(
                            anyhow::Error::new(e).context(context),
                        ));
                    }
                }
            },
            Err(e) => {
                let context = format!("probing {}", dest.display());
                return Err(IntermediateWalkError::Io(
                    anyhow::Error::new(e).context(context),
                ));
            }
        }
        verified_dirs.push(dest.clone());
    }
    Ok(dest)
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
    fn relink_creates_absent_intermediate_directories() {
        let fx = RelinkFixture::new();
        fx.relink(&["sub/deeper/file"]).unwrap();
        for directory in ["sub", "sub/deeper"] {
            let meta = std::fs::symlink_metadata(fx.link_dir().join(directory)).unwrap();
            assert!(
                meta.file_type().is_dir(),
                "{directory} should be a real dir"
            );
        }
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("sub/deeper/file")).unwrap(),
            fx.worktree.join("sub/deeper/file")
        );
    }

    #[test]
    fn relink_one_reports_unreadable_intermediate_component_as_probe_error() {
        use std::os::unix::fs::PermissionsExt;
        let fx = RelinkFixture::new();
        let locked = fx.link_dir().join("locked");
        std::fs::create_dir_all(locked.join("inner")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let probe_is_denied = std::fs::symlink_metadata(locked.join("inner")).is_err();
        let planned = PlannedLink {
            entry: "locked/inner/file",
            relative_dest: PathBuf::from("locked/inner/file"),
            source: fx.worktree.join("locked/inner/file"),
        };
        let result = relink_one(&fx.link_dir(), &planned);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !probe_is_denied {
            eprintln!("skipping: permission bits not enforced (running as root?)");
            return;
        }
        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("probing") && !message.contains("does not exist"),
            "unexpected error: {message}"
        );
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
    fn relink_rejects_parent_dir_component_before_writing() {
        let fx = RelinkFixture::new();
        for entry in [".a/..", "x/../env"] {
            let err = fx.relink(&[".env", entry]).unwrap_err();
            assert!(
                err.to_string().contains("contains a '..' component"),
                "unexpected error for {entry}: {err}"
            );
            assert!(std::fs::symlink_metadata(fx.link_dir()).is_err());
        }
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

    fn to_links(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn create_symlinks_creates_nested_parents() {
        let fx = RelinkFixture::new();
        create_symlinks(
            &fx.worktree,
            &fx.base,
            "wt",
            &to_links(&[".claude/skills", "a/b/c"]),
        )
        .unwrap();
        for nested_dir in ["claude", "a", "a/b"] {
            let meta = std::fs::symlink_metadata(fx.link_dir().join(nested_dir)).unwrap();
            assert!(meta.file_type().is_dir(), "{nested_dir} is not a real dir");
        }
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("claude/skills")).unwrap(),
            fx.worktree.join(".claude/skills")
        );
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("a/b/c")).unwrap(),
            fx.worktree.join("a/b/c")
        );
    }

    #[test]
    fn create_symlinks_does_not_resolve_dotdot_through_symlinked_component() {
        let fx = RelinkFixture::new();
        std::fs::create_dir(fx.outside.join("sub")).unwrap();
        std::fs::create_dir(fx.link_dir()).unwrap();
        symlink(fx.outside.join("sub"), fx.link_dir().join("a")).unwrap();
        create_symlinks(&fx.worktree, &fx.base, "wt", &to_links(&["a/../env"])).unwrap();
        assert!(std::fs::symlink_metadata(fx.outside.join("env")).is_err());
        assert_eq!(
            std::fs::read_link(fx.link_dir().join("env")).unwrap(),
            fx.worktree.join("a/../env")
        );
    }

    #[test]
    fn remove_symlinks_does_not_descend_through_symlinked_link_dir() {
        let fx = RelinkFixture::new();
        symlink(fx.worktree.join(".env"), fx.outside.join("env")).unwrap();
        symlink(&fx.outside, fx.link_dir()).unwrap();
        remove_symlinks(&fx.base, "wt", &to_links(&[".env"])).unwrap();
        assert!(is_symlink(&fx.outside.join("env")));
        assert!(std::fs::symlink_metadata(fx.link_dir()).is_err());
    }

    #[test]
    fn remove_symlinks_does_not_resolve_dotdot_through_symlinked_component() {
        let fx = RelinkFixture::new();
        std::fs::create_dir(fx.outside.join("sub")).unwrap();
        symlink(fx.worktree.join(".env"), fx.outside.join("env")).unwrap();
        std::fs::create_dir(fx.link_dir()).unwrap();
        symlink(fx.outside.join("sub"), fx.link_dir().join("a")).unwrap();
        let _ = remove_symlinks(&fx.base, "wt", &to_links(&["a/../env"]));
        assert!(is_symlink(&fx.outside.join("env")));
    }

    fn worktree_dir_placeholder() -> std::path::PathBuf {
        std::path::PathBuf::from("/dev/null")
    }
}
