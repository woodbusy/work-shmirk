//! Clap-derived CLI definitions and worktree-name validation.
//!
//! The name validator is the first line of defense against shell-metacharacter
//! injection through the tmux `send-keys` payload. See plan Step 9 and the
//! README "differences" section for the rationale.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "work-shmirk",
    version,
    about = "Manage git worktrees with optional Claude/tmux integration"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new worktree (and branch by default).
    New {
        /// Check out an existing branch instead of creating a new one.
        #[arg(short = 'e', long = "existing")]
        existing: bool,

        /// Worktree (and branch) name.
        #[arg(value_parser = parse_worktree_name)]
        name: String,
    },
    /// Remove a worktree and its branch.
    Remove {
        /// Worktree name. Defaults to the current worktree's basename when omitted.
        #[arg(value_parser = parse_worktree_name)]
        name: Option<String>,
    },
}

/// Validate a worktree name. Rejects:
///   - empty strings
///   - names containing `/` or `..` (must be a single path component)
///   - shell-metacharacters: `'`, `` ` ``, `$`, `\`, newline, any control char
///
/// This is the chosen mitigation for the tmux-quoting injection concern. The
/// downstream POSIX single-quote escape (`replace("'", "'\\''")`) is still
/// applied to all interpolated strings, but rejecting the names up front
/// produces a clear error before any subprocess runs.
pub fn parse_worktree_name(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Err("worktree name must not be empty".to_string());
    }
    if raw.contains('/') {
        return Err("worktree name must not contain '/'".to_string());
    }
    if raw.contains("..") {
        return Err("worktree name must not contain '..'".to_string());
    }
    for ch in raw.chars() {
        if ch.is_control() {
            return Err(format!(
                "worktree name must not contain control characters (got 0x{:02x})",
                ch as u32
            ));
        }
        match ch {
            '\'' | '`' | '$' | '\\' | '\n' => {
                return Err(format!(
                    "worktree name must not contain shell-metacharacter '{}'",
                    ch.escape_default()
                ));
            }
            _ => {}
        }
    }
    Ok(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_names() {
        assert!(parse_worktree_name("feature-x").is_ok());
        assert!(parse_worktree_name("ENG-42-foo").is_ok());
        assert!(parse_worktree_name("issue-7").is_ok());
        assert!(parse_worktree_name("a").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_worktree_name("").is_err());
    }

    #[test]
    fn rejects_path_separators() {
        assert!(parse_worktree_name("foo/bar").is_err());
        assert!(parse_worktree_name("../escape").is_err());
        assert!(parse_worktree_name("foo..bar").is_err());
    }

    #[test]
    fn rejects_shell_metacharacters() {
        for bad in ["foo'bar", "foo`bar", "foo$bar", "foo\\bar", "foo\nbar"] {
            assert!(
                parse_worktree_name(bad).is_err(),
                "expected reject: {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_control_characters() {
        assert!(parse_worktree_name("foo\tbar").is_err());
        assert!(parse_worktree_name("foo\x07bar").is_err());
    }
}
