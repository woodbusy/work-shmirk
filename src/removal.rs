//! Orchestrates the `remove` flow.

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

    // Pick config dir: three-way check.
    //
    // 1. `target_cfg.is_dir()` — follows symlinks, so symlinks-to-dirs work.
    //    Use the per-worktree config.
    // 2. `symlink_metadata` fails (no directory entry at all, not even a broken
    //    symlink) — silently fall back to the main repo's config. This is the
    //    normal case for worktrees that have no per-worktree override.
    // 3. Everything else (regular file, broken symlink, socket, fifo, …) — the
    //    entry exists but cannot be used as a config directory. This is almost
    //    certainly a user mistake; fail loudly rather than silently loading a
    //    different config that may remove the wrong symlinks.
    let target_cfg = target_path.join(".work-shmirk");
    let main_cfg = worktrees_root.join(".work-shmirk");
    // Three-way check using a single metadata probe to avoid TOCTOU and double
    // syscalls. `is_dir()` follows symlinks; we use `symlink_metadata()` to
    // distinguish "absent" from "present but wrong type".
    let config_dir = if target_cfg.is_dir() {
        target_cfg
    } else {
        match target_cfg.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => main_cfg,
            Err(e) => {
                return Err(anyhow!(
                    "{} could not be probed: {}",
                    target_cfg.display(),
                    e
                ))
            }
            Ok(_) => {
                return Err(anyhow!(
                    "{} exists but is not a directory; it must be a directory (or absent) for work-shmirk to load config",
                    target_cfg.display()
                ))
            }
        }
    };

    let settings = load(&config_dir)?;

    // --- Remove symlinks ---
    if let Some(dir_raw) = settings.symlink_dir.as_deref() {
        if let Some(base) = expand_symlink_dir(dir_raw)? {
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

    // Read ownership from the worktree's local git config *before* removing
    // the worktree: the worktree directory must still exist for `git config
    // --local` to work, and `git worktree list` also needs it to still exist.
    let owned_branch: Option<String> = git::get_worktree_owned_branch(&target_path).unwrap_or(None);
    let attached_branch: Option<String> =
        git::worktree_branch(git_cwd, &target_path).unwrap_or(None);

    println!("Removing worktree '{name}'");
    git::worktree_remove(git_cwd, &target_path)?;

    // Primary gate: only delete if work-shmirk created the branch (ownership
    // recorded in git config at creation time).
    // Defense-in-depth: also require the porcelain-reported branch name
    // matches the worktree name (skip this extra check if lookup failed).
    let branch_name_matches = attached_branch
        .as_deref()
        .map_or(true, |b| b == name.as_str());

    if owned_branch.is_some() && branch_name_matches {
        println!("Deleting local branch '{name}'");
        git::branch_delete_force(git_cwd, &name)?;
    } else if owned_branch.is_none() {
        eprintln!(
            "Skipping branch delete: '{name}' was not created by work-shmirk (no ownership record found)"
        );
    } else {
        eprintln!(
            "Skipping branch delete: attached branch '{}' does not match worktree name '{name}'",
            attached_branch.as_deref().unwrap_or("<unknown>")
        );
    }

    Ok(())
}
