//! Orchestrates the `new` flow.
//!
//! Mirrors the order of the bash `new-worktree` script:
//!   1. resolve source root + config dir
//!   2. load config
//!   3. resolve worktrees root and target path
//!   4. echo creation banner + `git worktree add`
//!   5. set up `.worktree-local/` + symlinks + copy_files
//!   6. detect issue ref, build prompt
//!   7. tmux 3-pane layout OR inline claude + exec $SHELL
//!
//! Notes on `git rev-parse --show-toplevel`: when run inside a worktree it
//! returns that worktree's path. We deliberately use it (matching bash) so the
//! `.work-shmirk/` config dir follows the user into worktrees.

use anyhow::{anyhow, Context, Result};
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::Command;

use crate::config::{expand_symlink_dir, Settings};
use crate::copyfiles::copy_files;
use crate::git;
use crate::issue::parse_issue;
use crate::prompt::build_prompt;
use crate::symlinks::create_symlinks;
use crate::tmux;

fn claude_bin() -> String {
    std::env::var("WORK_SHMIRK_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string())
}

pub fn run_new(name: &str, existing: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current dir")?;

    let source_root = git::show_toplevel(&cwd)?;
    let config_dir = source_root.join(".work-shmirk");
    let settings = crate::config::load(&config_dir)?;

    let worktrees_root = git::worktrees_root(&cwd)?;
    let target = worktrees_root.join(name);

    println!("Creating worktree '{name}' in {}...", target.display());

    git::worktree_add(&cwd, &target, name, existing)?;

    // From here, everything that operates on the worktree uses `target` as
    // the base. We do not change the Rust process cwd.
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

    println!("Worktree created. Setting up environment...");

    let issue = parse_issue(name);
    let prompt = build_prompt(name, issue.as_ref(), settings.issues.as_ref());

    if std::env::var_os("TMUX").is_some() {
        run_tmux_flow(&settings, &target, name, &prompt)?;
    } else {
        run_inline_flow(&target, &prompt)?;
    }

    Ok(())
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
    let Some(base) = expand_symlink_dir(dir_raw) else {
        return Ok(());
    };
    let links = settings.symlink_links.clone().unwrap_or_default();
    create_symlinks(target, &base, name, &links)?;
    Ok(())
}

fn run_tmux_flow(settings: &Settings, target: &Path, name: &str, prompt: &str) -> Result<()> {
    println!("Setting up tmux panes...");

    // Window name: replace at most once, matching bash `${var/x/y}`.
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

    tmux::setup_panes(target, prompt, &window_name)?;

    // Bash does `clear` here. Emit the equivalent ANSI sequence rather than
    // shelling out, but only when stdout is a TTY so we don't corrupt
    // redirected logs.
    let mut stdout = std::io::stdout();
    if stdout.is_terminal() {
        print!("\x1b[H\x1b[2J");
        let _ = stdout.flush();
    }
    println!("Environment ready!");
    println!();
    Ok(())
}

fn run_inline_flow(target: &Path, prompt: &str) -> Result<()> {
    // Resolve the shell up front so we can fail clearly if unset/empty.
    let shell = match std::env::var("SHELL") {
        Ok(s) if !s.is_empty() => s,
        _ => return Err(anyhow!("SHELL not set; cannot launch shell")),
    };

    // Run claude with the prompt as a single CLI arg (no stdin coupling).
    // Set cwd to the worktree target so claude is launched from inside the
    // new worktree, matching the bash flow which `cd`s before invoking it.
    let claude_status = Command::new(claude_bin())
        .arg(prompt)
        .current_dir(target)
        .status()
        .context("invoking claude")?;
    if !claude_status.success() {
        return Err(anyhow!(
            "claude exited with {}",
            claude_status.code().unwrap_or(-1)
        ));
    }

    println!();
    println!("Launching shell in new worktree at {}", target.display());

    // Test-only seam: short-circuit before exec.
    if std::env::var_os("WORK_SHMIRK_NO_EXEC").is_some() {
        return Ok(());
    }

    exec_shell(&shell, target)
}

#[cfg(unix)]
fn exec_shell(shell: &str, target: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let err = Command::new(shell).current_dir(target).exec();
    Err(anyhow!("exec failed: {err}"))
}

#[cfg(not(unix))]
fn exec_shell(_shell: &str, _target: &Path) -> Result<()> {
    Err(anyhow!("non-unix targets not supported"))
}
