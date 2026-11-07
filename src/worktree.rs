//! Orchestrates the `new` flow.
//!
//! Steps: resolve source root + config, create worktree, set up
//! `.worktree-local/` + symlinks + copy_files + the dev-workflow local
//! config, optionally set up `personal-issues`, optionally launch a tmux 3-pane layout, then print the
//! new worktree path to stdout.
//!
//! `git rev-parse --show-toplevel` returns the worktree path when run inside a
//! worktree, so `.work-shmirk/` config follows the user into worktrees.

use anyhow::{Context, Result};
use std::path::Path;

use crate::config::{expand_symlink_dir, Settings};
use crate::copyfiles::{copy_dev_workflow_config, copy_files};
use crate::git;
use crate::issue::{format_issue_id, parse_issue};
use crate::personal_issues;
use crate::symlinks::create_symlinks;
use crate::tmux;

pub fn run_new(name: &str, existing: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current dir")?;

    let source_root = git::show_toplevel(&cwd)?;
    let config_dir = source_root.join(".work-shmirk");
    let settings = crate::config::load(&config_dir)?;
    if let Some(reference) = op_token_ref(&settings) {
        crate::config::validate_op_token_ref(reference)?;
    }

    let worktrees_root = git::worktrees_root(&cwd)?;
    let target = worktrees_root.join(name);

    eprintln!("Creating worktree '{name}' in {}...", target.display());
    git::worktree_add(&cwd, &target, name, existing)?;
    setup_worktree_local(&target)?;

    // Record branch ownership in the worktree's local git config only when we
    // created the branch ourselves.  This signals to `run_remove` that it is
    // safe to delete the branch.  When `-e` is used the branch pre-existed
    // and must not be deleted.  Git config lives in `.git/worktrees/<name>/config`
    // which is unaffected by `git clean` and cannot be forged by writing files
    // in the working tree.
    if !existing {
        git::set_worktree_owned_branch(&target, name)?;
    }

    setup_symlinks(&settings, &target, name)?;
    if let Some(ref files) = settings.copy_files {
        copy_files(&config_dir, &target, files)?;
    }
    copy_dev_workflow_config(&source_root, &target)?;

    if settings.personal_issues {
        let issue_id =
            parse_issue(name).map(|issue| format_issue_id(&issue, settings.issues.as_ref()));
        personal_issues::setup(&target, issue_id.as_deref())?;
    }

    if std::env::var_os("TMUX").is_some() {
        run_tmux_flow(&settings, &target, name)?;
    }

    println!("{}", target.display());
    Ok(())
}

fn op_token_ref(settings: &Settings) -> Option<&str> {
    settings
        .claude
        .as_ref()
        .and_then(|claude| claude.op_token_ref.as_deref())
        .filter(|reference| !reference.is_empty())
}

fn setup_worktree_local(target: &Path) -> Result<()> {
    let wl = target.join(".worktree-local");
    let wl_tmp = wl.join("tmp");
    std::fs::create_dir_all(&wl).with_context(|| format!("creating {}", wl.display()))?;
    std::fs::create_dir_all(&wl_tmp).with_context(|| format!("creating {}", wl_tmp.display()))?;
    Ok(())
}

fn setup_symlinks(settings: &Settings, target: &Path, name: &str) -> Result<()> {
    let Some(ref dir_raw) = settings.symlink_dir else {
        return Ok(());
    };
    let Some(base) = expand_symlink_dir(dir_raw)? else {
        return Ok(());
    };
    let links = settings.symlink_links.clone().unwrap_or_default();
    create_symlinks(target, &base, name, &links)?;
    Ok(())
}

fn run_tmux_flow(settings: &Settings, target: &Path, name: &str) -> Result<()> {
    // Window name: replace at most once (first occurrence only).
    let window_name = {
        let mut wn = name.to_string();
        if let (Some(tmux_cfg), Some(issues_cfg)) =
            (settings.tmux.as_ref(), settings.issues.as_ref())
        {
            if let (Some(sub), Some(proj)) = (
                tmux_cfg.project_name_substitution.as_deref(),
                issues_cfg.project.as_deref(),
            ) {
                if !sub.is_empty() && !proj.is_empty() {
                    wn = wn.replacen(proj, sub, 1);
                }
            }
        }
        wn
    };

    tmux::setup_panes(target, &window_name, op_token_ref(settings))
}
