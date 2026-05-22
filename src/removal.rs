//! Orchestrates the `remove` flow. Mirrors the bash `remove-worktree` script.

use anyhow::{anyhow, Context, Result};
use std::path::Path;

use crate::cli::parse_worktree_name;
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
            // Run the derived basename through the CLI validator so the
            // default-name path enforces the same constraints as an explicit
            // argument (rejects `..`, metachars, control chars).
            parse_worktree_name(&basename)
                .map_err(|e| anyhow!("derived worktree name from current dir is invalid: {}", e))?
        }
    };

    let target_path = worktrees_root.join(&name);

    // Pick config dir: target's own .work-shmirk if present, else main repo's.
    // `is_dir()` (rather than `exists()`) intentionally falls back to the main
    // repo's config when the per-worktree entry is missing OR exists-but-not-
    // a-directory (file, broken symlink). This matches bash's `[ -d ... ]`.
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

    // Capture the sentinel and the porcelain branch name *before* removing
    // the worktree: the sentinel lives inside the worktree directory, and
    // `git worktree list` needs the worktree to still exist.
    let sentinel_present = target_path
        .join(".worktree-local/work-shmirk-owned-branch")
        .exists();
    let attached_branch: Option<String> =
        git::worktree_branch(git_cwd, &target_path).unwrap_or(None);

    println!("Removing worktree '{name}'");
    git::worktree_remove(git_cwd, &target_path)?;

    // Primary gate: only delete if work-shmirk created the branch.
    // Defense-in-depth: also require the porcelain-reported branch name
    // matches the worktree name (skip this extra check if lookup failed).
    let branch_name_matches = attached_branch
        .as_deref()
        .map_or(true, |b| b == name.as_str());

    if sentinel_present && branch_name_matches {
        println!("Deleting local branch '{name}'");
        git::branch_delete_force(git_cwd, &name)?;
    } else {
        eprintln!(
            "Skipping branch delete: '{name}' was not created by work-shmirk (no sentinel found)"
        );
    }

    Ok(())
}
