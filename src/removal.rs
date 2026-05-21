//! Orchestrates the `remove` flow. Mirrors the bash `remove-worktree` script.

use anyhow::{anyhow, Context, Result};
use std::path::Path;

use crate::config::{expand_symlink_dir, load};
use crate::git;
use crate::symlinks::remove_symlinks;

pub fn run_remove(arg: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current dir")?;

    let worktrees_root = git::worktrees_root(&cwd)?;
    let current_worktree = git::show_toplevel(&cwd)?;

    // Determine target name: explicit arg wins; otherwise basename(current).
    // When defaulting AND the current worktree IS the main worktree, refuse.
    let name: String = match arg {
        Some(s) => s.to_string(),
        None => {
            if current_worktree == worktrees_root {
                return Err(anyhow!(
                    "you're in the main worktree, not a removable worktree.\nUsage: work-shmirk remove <worktree-name>"
                ));
            }
            current_worktree
                .file_name()
                .ok_or_else(|| {
                    anyhow!(
                        "could not determine current worktree basename from {}",
                        current_worktree.display()
                    )
                })?
                .to_string_lossy()
                .to_string()
        }
    };

    let target_path = worktrees_root.join(&name);

    // Pick config dir: target's own .work-shmirk if present, else main repo's.
    let target_cfg = target_path.join(".work-shmirk");
    let main_cfg = worktrees_root.join(".work-shmirk");
    let config_dir = if target_cfg.is_dir() {
        target_cfg
    } else {
        main_cfg
    };

    let settings = load(&config_dir)?;

    // --- Remove symlinks ---
    if let Some(dir_raw) = settings.symlink_dir.as_deref() {
        if let Some(base) = expand_symlink_dir(dir_raw) {
            let links = settings.symlink_links.clone().unwrap_or_default();
            remove_symlinks(&base, &name, &links)?;
        }
    }

    // --- Remove worktree + branch ---
    //
    // We always run the git commands with cwd = worktrees_root, never via
    // `env::set_current_dir`. This avoids the "current dir got deleted under
    // us" problem when removing the worktree we're currently inside.
    let git_cwd: &Path = &worktrees_root;

    println!("Removing worktree '{name}'");
    git::worktree_remove(git_cwd, &target_path)?;

    println!("Deleting local branch '{name}'");
    git::branch_delete_force(git_cwd, &name)?;

    Ok(())
}
