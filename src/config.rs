//! Config loading and path-containment helpers.
//!
//! Config lives in `<worktree>/.work-shmirk/settings.json`. It is personal
//! configuration and must not be tracked by git, so a repository cannot ship
//! settings that drive work-shmirk on a user's machine.

use crate::git;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct Settings {
    #[serde(default)]
    pub issues: Option<IssuesConfig>,
    #[serde(default)]
    pub copy_files: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub symlink_dir: Option<String>,
    #[serde(default)]
    pub symlink_links: Option<Vec<String>>,
    #[serde(default)]
    pub tmux: Option<TmuxConfig>,
    #[serde(default)]
    pub personal_issues: bool,
    #[serde(default)]
    pub claude: Option<ClaudeConfig>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct ClaudeConfig {
    /// 1Password secret reference (`op://vault/item/field`) whose value is
    /// exported as `OP_SERVICE_ACCOUNT_TOKEN` in the pane that runs claude.
    #[serde(default)]
    pub op_token_ref: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct IssuesConfig {
    /// Rename `type` (a Rust keyword) to `type_` on the Rust side.
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct TmuxConfig {
    #[serde(default)]
    pub project_name_substitution: Option<String>,
}

/// Validate a 1Password secret reference. It must start with `op://` and
/// contain no control characters (which includes newlines).
pub fn validate_op_token_ref(reference: &str) -> Result<()> {
    if !reference.starts_with("op://") || reference.len() == "op://".len() {
        return Err(anyhow!(
            "claude.op_token_ref must be a 1Password secret reference starting with 'op://'"
        ));
    }
    if reference.chars().any(char::is_control) {
        return Err(anyhow!(
            "claude.op_token_ref must not contain control characters"
        ));
    }
    Ok(())
}

const SETTINGS_FILE: &str = "settings.json";
const LEGACY_LOCAL_SETTINGS_FILE: &str = "settings.local.json";

/// Load settings from `<config_dir>/settings.json`, where `config_dir` is
/// `<worktree>/.work-shmirk`.
///
/// Returns the default settings (all None) when the directory or file is
/// absent. Errors when the file is tracked by git, or when a legacy
/// `settings.local.json` is present.
pub fn load(config_dir: &Path) -> Result<Settings> {
    if !config_dir.exists() {
        return Ok(Settings::default());
    }
    let legacy_path = config_dir.join(LEGACY_LOCAL_SETTINGS_FILE);
    if legacy_path.exists() {
        bail!(
            "{} is no longer supported; work-shmirk now reads a single, untracked {}. \
             Move its contents into {} and delete it",
            legacy_path.display(),
            SETTINGS_FILE,
            config_dir.join(SETTINGS_FILE).display()
        );
    }
    let settings_path = config_dir.join(SETTINGS_FILE);
    if !settings_path.exists() {
        return Ok(Settings::default());
    }
    ensure_untracked(config_dir, &settings_path)?;
    let content = std::fs::read_to_string(&settings_path)
        .with_context(|| format!("reading {}", settings_path.display()))?;
    let settings: Settings = serde_json::from_str(&content)
        .with_context(|| format!("parsing settings in {}", settings_path.display()))?;
    Ok(settings)
}

/// Refuse settings that the worktree's repository tracks. Checks both the
/// path as named and the path after resolving symlinks, so a tracked symlink
/// (or a `.work-shmirk` symlink into a tracked directory) cannot smuggle in
/// repository-supplied settings. A resolved path outside the repository is
/// not checked: it cannot be content the repository supplies.
fn ensure_untracked(config_dir: &Path, settings_path: &Path) -> Result<()> {
    let worktree_dir = config_dir
        .parent()
        .ok_or_else(|| anyhow!("config dir {} has no parent", config_dir.display()))?;
    let repo_root = git::show_toplevel(worktree_dir)?
        .canonicalize()
        .context("resolving repository root")?;
    let as_named = worktree_dir
        .canonicalize()
        .with_context(|| format!("resolving {}", worktree_dir.display()))?
        .join(config_dir.file_name().unwrap_or_default())
        .join(SETTINGS_FILE);
    let resolved = settings_path
        .canonicalize()
        .with_context(|| format!("resolving {}", settings_path.display()))?;

    for candidate in [&as_named, &resolved] {
        let Ok(relative) = candidate.strip_prefix(&repo_root) else {
            continue;
        };
        if git::is_tracked(&repo_root, relative)? {
            bail!(
                "refusing to load {}: it is tracked by git. work-shmirk settings are \
                 personal and must stay untracked so a repository cannot configure \
                 work-shmirk on your machine. Untrack it with `git rm --cached` or \
                 delete it",
                settings_path.display()
            );
        }
    }
    Ok(())
}

/// Expand a `symlink_dir` config string. The following leading tokens are
/// recognized and replaced with the value of `$HOME`:
///   - `~/...` or bare `~`
///   - `$HOME/...` or bare `$HOME`
///   - `${HOME}/...` or bare `${HOME}`
///
/// Any `~` not immediately followed by `/` or end-of-string (e.g. `~alice/`)
/// is an error — `~user`-style home-dir lookup is not supported.
///
/// Mid-string occurrences of `~`, `$HOME`, or `${HOME}` are left literal.
/// The returned path is lexically normalized: redundant slashes and `./`
/// segments are collapsed, but `..` is preserved as-is.
///
/// Returns `Ok(None)` when `raw` is empty (feature disabled).
pub fn expand_symlink_dir(raw: &str) -> Result<Option<PathBuf>> {
    let home = std::env::var("HOME").unwrap_or_default();
    expand_symlink_dir_with_home(raw, &home)
}

/// Same as `expand_symlink_dir` but takes the home directory as an argument,
/// for use in tests that run in parallel (avoids mutating the process-global
/// `HOME` env var).
pub fn expand_symlink_dir_with_home(raw: &str, home: &str) -> Result<Option<PathBuf>> {
    if raw.is_empty() {
        return Ok(None);
    }
    let expanded = if let Some(after_tilde) = raw.strip_prefix('~') {
        // `~` must be followed by `/` or end-of-string; anything else (e.g.
        // `~alice`) is unsupported and would silently produce a broken path.
        if after_tilde.is_empty() || after_tilde.starts_with('/') {
            format!("{home}{after_tilde}")
        } else {
            return Err(anyhow!(
                "unsupported symlink_dir value {raw:?}: `~` must be followed by `/` or \
                 end-of-string; `~user`-style home-dir expansion is not supported"
            ));
        }
    } else if let Some(after_home) = raw.strip_prefix("${HOME}") {
        // `${HOME}` is only recognized when followed by `/` or end-of-string.
        if after_home.is_empty() || after_home.starts_with('/') {
            format!("{home}{after_home}")
        } else {
            raw.to_string()
        }
    } else if let Some(after_home) = raw.strip_prefix("$HOME") {
        // `$HOME` is only recognized when followed by `/` or end-of-string.
        if after_home.is_empty() || after_home.starts_with('/') {
            format!("{home}{after_home}")
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };

    // Lexically normalize: drop CurDir ("./") segments and collapse redundant
    // separators, but keep ParentDir ("..") components in place — resolving ".."
    // is the job of `ensure_inside`, not this function.
    let normalized: PathBuf = PathBuf::from(&expanded)
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect();

    Ok(Some(normalized))
}

/// Validate that `candidate` resolves to a path inside `base` (lexically;
/// the path need not exist yet). Returns the normalized absolute-ish path on
/// success; errors with a clear message on escape.
///
/// Path-traversal guard for `copy_files` destinations and `symlink_links`.
pub fn ensure_inside(base: &Path, candidate: &Path) -> Result<PathBuf> {
    let combined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };

    // Lexically resolve `.` and `..` against the components of `combined`,
    // without touching the filesystem.
    let mut stack: Vec<Component> = Vec::new();
    for comp in combined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop the last normal component if any; otherwise we'd escape.
                match stack.last() {
                    Some(Component::Normal(_)) => {
                        stack.pop();
                    }
                    Some(Component::RootDir) | Some(Component::Prefix(_)) => {
                        // Cannot ascend past root.
                    }
                    _ => {
                        // Leading `..` with no normal ancestor → escape.
                        return Err(anyhow!(
                            "path escapes containing directory: {}",
                            candidate.display()
                        ));
                    }
                }
            }
            other => stack.push(other),
        }
    }
    let normalized: PathBuf = stack.iter().collect();

    // Likewise normalize `base`.
    let mut base_stack: Vec<Component> = Vec::new();
    for comp in base.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(base_stack.last(), Some(Component::Normal(_))) {
                    base_stack.pop();
                }
            }
            other => base_stack.push(other),
        }
    }
    let base_normalized: PathBuf = base_stack.iter().collect();

    if !normalized.starts_with(&base_normalized) {
        return Err(anyhow!(
            "path '{}' escapes base '{}'",
            candidate.display(),
            base.display()
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expand_tilde_prefix() {
        assert_eq!(
            expand_symlink_dir_with_home("~/foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_home_var() {
        assert_eq!(
            expand_symlink_dir_with_home("$HOME/foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
        // Mid-string $HOME is NOT expanded — only leading $HOME is a token.
        assert_eq!(
            expand_symlink_dir_with_home("/x/$HOME/y", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/x/$HOME/y")
        );
    }

    #[test]
    fn expand_leading_home_var_only_at_start() {
        // $HOME with no trailing path component still expands correctly.
        assert_eq!(
            expand_symlink_dir_with_home("$HOME", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u")
        );
    }

    #[test]
    fn expand_home_var_no_separator_is_literal() {
        // "$HOMEfoo" must NOT expand — there is no separator after $HOME, so
        // the token is not a leading $HOME variable reference.
        assert_eq!(
            expand_symlink_dir_with_home("$HOMEshared", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("$HOMEshared")
        );
    }

    #[test]
    fn expand_tilde_with_trailing_slash_in_home() {
        // A home value that itself has a trailing slash is handled correctly;
        // PathBuf::components strips the redundant separator.
        assert_eq!(
            expand_symlink_dir_with_home("~/foo", "/home/u/")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_collapses_double_slash() {
        // Redundant slashes in the resulting path are normalized away.
        assert_eq!(
            expand_symlink_dir_with_home("~//foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
        assert_eq!(
            expand_symlink_dir_with_home("$HOME//foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
        assert_eq!(
            expand_symlink_dir_with_home("${HOME}//foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_collapses_dot_segments() {
        // CurDir (".") segments are dropped during normalization.
        assert_eq!(
            expand_symlink_dir_with_home("~/./foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_preserves_parent_dir_segments() {
        // ParentDir ("..") is preserved as-is; resolving it is not our job.
        assert_eq!(
            expand_symlink_dir_with_home("~/a/../b", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/a/../b")
        );
    }

    #[test]
    fn expand_no_mid_string_tilde() {
        assert_eq!(
            expand_symlink_dir_with_home("/x/~/y", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/x/~/y")
        );
    }

    #[test]
    fn expand_empty_returns_none() {
        assert!(expand_symlink_dir_with_home("", "/home/u")
            .unwrap()
            .is_none());
    }

    // --- new tests for ${HOME} form and ~user error ---

    #[test]
    fn expand_brace_home_with_path() {
        assert_eq!(
            expand_symlink_dir_with_home("${HOME}/foo", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_brace_home_bare() {
        assert_eq!(
            expand_symlink_dir_with_home("${HOME}", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u")
        );
    }

    #[test]
    fn expand_brace_home_no_separator_is_literal() {
        // `${HOME}shared` — no `/` after `}`, so the token is left literal.
        assert_eq!(
            expand_symlink_dir_with_home("${HOME}shared", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("${HOME}shared")
        );
    }

    #[test]
    fn expand_brace_home_longer_var_is_literal() {
        // `${HOMEshared}` — the closing `}` is not in the `${HOME}` position,
        // so the naive "strip `${HOME`, find `}`" path is guarded against.
        assert_eq!(
            expand_symlink_dir_with_home("${HOMEshared}", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("${HOMEshared}")
        );
    }

    #[test]
    fn expand_tilde_user_returns_error() {
        // `~alice/foo` is not supported and must produce a clear error.
        assert!(expand_symlink_dir_with_home("~alice/foo", "/home/u").is_err());
    }

    #[test]
    fn expand_tilde_dot_returns_error() {
        // `~.gitconfig` also hits the "not `/` or end" arm — rule is about the
        // character after `~`, not specifically user names.
        assert!(expand_symlink_dir_with_home("~.gitconfig", "/home/u").is_err());
    }

    #[test]
    fn expand_tilde_bare_still_expands() {
        // A bare `~` (no trailing bytes) must still expand to `$HOME`.
        assert_eq!(
            expand_symlink_dir_with_home("~", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/home/u")
        );
    }

    #[test]
    fn expand_mid_string_brace_home_is_literal() {
        // Mid-string `${HOME}` must not be expanded.
        assert_eq!(
            expand_symlink_dir_with_home("/x/${HOME}/y", "/home/u")
                .unwrap()
                .unwrap(),
            PathBuf::from("/x/${HOME}/y")
        );
    }

    #[test]
    fn ensure_inside_accepts_in_tree() {
        let base = Path::new("/tmp/wt");
        let result = ensure_inside(base, Path::new("foo/bar.txt")).unwrap();
        assert!(result.starts_with("/tmp/wt"));
    }

    #[test]
    fn ensure_inside_rejects_dotdot_escape() {
        let base = Path::new("/tmp/wt");
        assert!(ensure_inside(base, Path::new("../escape.txt")).is_err());
        assert!(ensure_inside(base, Path::new("a/../../escape.txt")).is_err());
    }

    #[test]
    fn ensure_inside_rejects_absolute_outside() {
        let base = Path::new("/tmp/wt");
        assert!(ensure_inside(base, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn ensure_inside_accepts_dot_in_path() {
        let base = Path::new("/tmp/wt");
        let result = ensure_inside(base, Path::new("./foo/./bar")).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/wt/foo/bar"));
    }

    #[test]
    fn settings_unknown_fields_ignored() {
        let value = json!({"unknown": 1, "issues": {"type": "github"}});
        let s: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(s.issues.unwrap().type_.unwrap(), "github");
    }

    #[test]
    fn settings_fully_populated() {
        let value = json!({
            "issues": {"type": "linear", "project": "ENG"},
            "copy_files": {"src": "dest"},
            "symlink_dir": "~/links",
            "symlink_links": [".env"],
            "tmux": {"project_name_substitution": "E"},
            "personal_issues": true
        });
        let s: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(s.issues.as_ref().unwrap().type_.as_deref(), Some("linear"));
        assert_eq!(s.copy_files.unwrap().get("src").unwrap(), "dest");
        assert_eq!(s.symlink_links.unwrap()[0], ".env");
        assert_eq!(
            s.tmux.unwrap().project_name_substitution.as_deref(),
            Some("E")
        );
        assert!(s.personal_issues);
    }

    #[test]
    fn personal_issues_defaults_to_false() {
        let s: Settings = serde_json::from_value(json!({})).unwrap();
        assert!(!s.personal_issues);
    }
}
