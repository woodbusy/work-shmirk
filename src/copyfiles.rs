//! Copy files from `<config_dir>/<src>` into `<worktree_target>/<dest>` per
//! `copy_files` config, with path-traversal validation on destinations.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::config::ensure_inside;

pub fn copy_files(
    config_dir: &Path,
    worktree_target: &Path,
    files: &BTreeMap<String, String>,
) -> Result<()> {
    for (src, dest) in files {
        let src_path = config_dir.join(src);
        if !src_path.is_file() {
            eprintln!(
                "Warning: copy_files source '{}' not found in {}",
                src,
                config_dir.display()
            );
            continue;
        }
        let dest_path = worktree_target.join(dest);
        ensure_inside(worktree_target, &dest_path)
            .with_context(|| format!("copy_files dest '{dest}' escapes worktree root"))?;

        // `Path::parent` of a bare filename returns `Some("")`, which would
        // trip `create_dir_all`. Skip if the parent component is empty.
        if let Some(parent) = Path::new(dest).parent() {
            if !parent.as_os_str().is_empty() {
                let abs_parent = worktree_target.join(parent);
                std::fs::create_dir_all(&abs_parent)
                    .with_context(|| format!("creating dest parent {}", abs_parent.display()))?;
            }
        }

        std::fs::copy(&src_path, &dest_path)
            .with_context(|| format!("copying {} → {}", src_path.display(), dest_path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn bare_filename_dest_skips_create_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("cfg");
        let target = tmp.path().join("wt");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(config_dir.join("config.json"), "{}").unwrap();

        let mut files = BTreeMap::new();
        files.insert("config.json".to_string(), "config.json".to_string());
        copy_files(&config_dir, &target, &files).unwrap();

        assert!(target.join("config.json").is_file());
    }

    #[test]
    fn nested_dest_creates_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("cfg");
        let target = tmp.path().join("wt");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(config_dir.join("a"), "x").unwrap();

        let mut files = BTreeMap::new();
        files.insert("a".to_string(), "sub/dir/a".to_string());
        copy_files(&config_dir, &target, &files).unwrap();

        assert!(target.join("sub/dir/a").is_file());
    }

    #[test]
    fn missing_source_warns_continues() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("cfg");
        let target = tmp.path().join("wt");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&target).unwrap();

        let mut files = BTreeMap::new();
        files.insert("missing".to_string(), "out".to_string());
        // Should not error.
        copy_files(&config_dir, &target, &files).unwrap();
        assert!(!target.join("out").exists());
    }

    #[test]
    fn rejects_escape_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let config_dir = tmp.path().join("cfg");
        let target = tmp.path().join("wt");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(config_dir.join("a"), "x").unwrap();

        let mut files = BTreeMap::new();
        files.insert("a".to_string(), "../escape.txt".to_string());
        assert!(copy_files(&config_dir, &target, &files).is_err());
        assert!(!tmp.path().join("escape.txt").exists());
    }
}
