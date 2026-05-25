//! Config loading, deep-merge, and path-containment helpers.
//!
//! Config lives in `<repo>/.work-shmirk/`:
//!   - `settings.json`        — committed defaults
//!   - `settings.local.json`  — gitignored per-user overrides
//!
//! Merge semantics match jq's `*` operator: recursive object merge, scalars
//! and arrays in the local file replace those in the base.

use anyhow::{anyhow, Context, Result};
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
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct IssuesConfig {
    /// Rename `type` (a Rust keyword) to `type_` on the Rust side.
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub cli: Option<String>,
    #[serde(default)]
    pub skill: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, PartialEq, Eq)]
pub struct TmuxConfig {
    #[serde(default)]
    pub project_name_substitution: Option<String>,
}

/// Load and merge settings from `<config_dir>/settings.json` and
/// `<config_dir>/settings.local.json`. Either or both may be absent.
///
/// If `config_dir` does not exist, returns the default settings (all None).
pub fn load(config_dir: &Path) -> Result<Settings> {
    if !config_dir.exists() {
        return Ok(Settings::default());
    }
    let base = read_optional_json(&config_dir.join("settings.json"))?;
    let local = read_optional_json(&config_dir.join("settings.local.json"))?;
    let merged = merge_values(base, local);
    let settings: Settings = serde_json::from_value(merged)
        .with_context(|| format!("parsing settings under {}", config_dir.display()))?;
    Ok(settings)
}

fn read_optional_json(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("parsing JSON in {}", path.display()))?;
    Ok(value)
}

/// Recursive deep-merge: objects merge key-wise, everything else (scalars,
/// arrays, mismatched types) is replaced by the rhs value.
pub fn merge_values(base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match (base, overlay) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v_overlay) in b {
                let v_base = a.remove(&k).unwrap_or(Value::Null);
                a.insert(k, merge_values(v_base, v_overlay));
            }
            Value::Object(a)
        }
        // Treat overlay-null specially: if the overlay key is explicitly null,
        // we still want it to overwrite (matches jq's `*` operator).
        (_, overlay) => overlay,
    }
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
/// Mid-string occurrences of `~`, `$HOME`, or `${HOME}` are left literal
/// (matches bash `${var/#\~/...}` semantics). The returned path is lexically
/// normalized: redundant slashes and `./` segments are collapsed, but `..` is
/// preserved as-is.
///
/// Returns `Ok(None)` when `raw` is empty (feature disabled).
pub fn expand_symlink_dir(raw: &str) -> Result<Option<PathBuf>> {
    let home = std::env::var("HOME").unwrap_or_default();
    expand_symlink_dir_with_home(raw, &home)
}

/// Same as `expand_symlink_dir` but takes the home directory as an argument.
/// Used by tests to avoid mutating the process-global `HOME` env var
/// (which is shared across parallel tests in a single test binary).
///
/// Recognized leading tokens: `~`, `$HOME`, `${HOME}`. A `~` not followed by
/// `/` or end-of-string is rejected with an error. Mid-string occurrences are
/// left literal. The returned path is lexically normalized (no `//` or `./`
/// segments) but `..` components are preserved.
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
/// This is the path-traversal guard for `copy_files` destinations and
/// `symlink_links` entries. Bash performs neither check.
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
    fn merge_base_only() {
        let base = json!({"a": 1, "b": {"c": 2}});
        let overlay = json!({});
        assert_eq!(merge_values(base.clone(), overlay), base);
    }

    #[test]
    fn merge_local_only() {
        let base = json!({});
        let overlay = json!({"a": 1});
        assert_eq!(merge_values(base, overlay.clone()), overlay);
    }

    #[test]
    fn merge_deep() {
        let base = json!({"issues": {"type": "linear", "project": "ENG"}, "x": 1});
        let overlay = json!({"issues": {"cli": "linear"}, "x": 2});
        let merged = merge_values(base, overlay);
        assert_eq!(
            merged,
            json!({"issues": {"type": "linear", "project": "ENG", "cli": "linear"}, "x": 2})
        );
    }

    #[test]
    fn merge_null_overlay_overwrites_base() {
        // Documented quirk: an explicit `null` in the overlay overwrites the
        // base value (matches jq's `*` operator).
        let base = json!({"a": 1});
        let overlay = json!({"a": null});
        assert_eq!(merge_values(base, overlay), json!({"a": null}));
    }

    #[test]
    fn merge_arrays_replace() {
        let base = json!({"symlink_links": [".a", ".b"]});
        let overlay = json!({"symlink_links": [".c"]});
        let merged = merge_values(base, overlay);
        assert_eq!(merged, json!({"symlink_links": [".c"]}));
    }

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
            "issues": {"type": "linear", "project": "ENG", "cli": "linear", "skill": "linear-cli"},
            "copy_files": {"src": "dest"},
            "symlink_dir": "~/links",
            "symlink_links": [".env"],
            "tmux": {"project_name_substitution": "E"}
        });
        let s: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(s.issues.as_ref().unwrap().type_.as_deref(), Some("linear"));
        assert_eq!(s.copy_files.unwrap().get("src").unwrap(), "dest");
        assert_eq!(s.symlink_links.unwrap()[0], ".env");
        assert_eq!(
            s.tmux.unwrap().project_name_substitution.as_deref(),
            Some("E")
        );
    }
}
