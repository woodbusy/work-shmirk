//! work-shmirk: git worktree manager with optional Claude and tmux integration.
//!
//! Organized as a thin CLI dispatcher (`run`) plus focused modules so the
//! pure logic can be unit-tested in isolation.

pub mod cli;
pub mod config;
pub mod copyfiles;
pub mod git;
pub mod issue;
pub mod prompt;
pub mod removal;
pub mod symlinks;
pub mod tmux;
pub mod worktree;

use anyhow::Result;
use clap::Parser;

/// Top-level entrypoint. Parses the CLI and dispatches to the appropriate
/// subcommand. Errors bubble up to `main` which prints them.
pub fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Command::New { existing, name } => worktree::run_new(&name, existing),
        cli::Command::Remove { name } => removal::run_remove(name.as_deref()),
    }
}
