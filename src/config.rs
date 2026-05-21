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

/// Expand a `symlink_dir` config string, matching bash semantics:
///   - empty input returns None (feature disabled)
///   - leading `~` is replaced with `$HOME`
///   - every literal `$HOME` substring is replaced with the value of `$HOME`
///
/// Note that mid-string `~` is NOT expanded (matches bash `${var/#\~/...}`).
pub fn expand_symlink_dir(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let mut expanded = if let Some(rest) = raw.strip_prefix('~') {
        format!("{home}{rest}")
    } else {
        raw.to_string()
    };
    if expanded.contains("$HOME") {
        expanded = expanded.replace("$HOME", &home);
    }
    Some(PathBuf::from(expanded))
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
    fn merge_arrays_replace() {
        let base = json!({"symlink_links": [".a", ".b"]});
        let overlay = json!({"symlink_links": [".c"]});
        let merged = merge_values(base, overlay);
        assert_eq!(merged, json!({"symlink_links": [".c"]}));
    }

    #[test]
    fn expand_tilde_prefix() {
        std::env::set_var("HOME", "/home/u");
        assert_eq!(
            expand_symlink_dir("~/foo").unwrap(),
            PathBuf::from("/home/u/foo")
        );
    }

    #[test]
    fn expand_home_var() {
        std::env::set_var("HOME", "/home/u");
        assert_eq!(
            expand_symlink_dir("$HOME/foo").unwrap(),
            PathBuf::from("/home/u/foo")
        );
        assert_eq!(
            expand_symlink_dir("/x/$HOME/y").unwrap(),
            PathBuf::from("/x//home/u/y")
        );
    }

    #[test]
    fn expand_no_mid_string_tilde() {
        std::env::set_var("HOME", "/home/u");
        assert_eq!(
            expand_symlink_dir("/x/~/y").unwrap(),
            PathBuf::from("/x/~/y")
        );
    }

    #[test]
    fn expand_empty_returns_none() {
        assert!(expand_symlink_dir("").is_none());
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
