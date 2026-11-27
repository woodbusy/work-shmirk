//! Orchestrates the `relink` flow: re-create the current worktree's symlinks
//! under `<symlink_dir>/<worktree-name>/`.
//!
//! Config is selected as `remove` selects it (per-worktree `.work-shmirk/`
//! first, main repo's as fallback), so a worktree can supply its own config.
//! That config is untrusted; `symlinks::relink_symlinks` carries the guards.

use anyhow::{anyhow, Context, Result};

use crate::cli::parse_worktree_name;
use crate::config::{expand_symlink_dir, load, select_config_dir};
use crate::git;
use crate::symlinks::relink_symlinks;

pub fn run_relink() -> Result<()> {
    let cwd = std::env::current_dir().context("getting current dir")?;

    let current_worktree = git::show_toplevel(&cwd)?;
    let worktrees_root = git::worktrees_root(&cwd)?;

    if same_path(&current_worktree, &worktrees_root) {
        return Err(anyhow!(
            "you're in the main worktree; relink acts on a linked worktree.\nRun `work-shmirk relink` from inside the worktree whose symlinks you want to re-create"
        ));
    }

    let basename = current_worktree
        .file_name()
        .ok_or_else(|| {
            anyhow!(
                "could not determine current worktree basename from {}",
                current_worktree.display()
            )
        })?
        .to_string_lossy()
        .to_string();
    let name = parse_worktree_name(&basename)
        .map_err(|e| anyhow!("derived worktree name from current dir is invalid: {}", e))?;

    let config_dir = select_config_dir(&current_worktree, &worktrees_root)?;
    let settings = load(&config_dir)?;

    let base = match settings.symlink_dir.as_deref() {
        Some(dir_raw) => expand_symlink_dir(dir_raw)?,
        None => None,
    };
    let Some(base) = base else {
        eprintln!("symlink_dir is not set; nothing to relink");
        return Ok(());
    };

    let links = settings.symlink_links.clone().unwrap_or_default();
    relink_symlinks(&current_worktree, &base, &name, &links)
}

fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
