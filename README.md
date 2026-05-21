# work-shmirk

A single Rust binary that wraps `git worktree` with optional Claude and tmux
integration. It is a port of the `new-worktree` and `remove-worktree` bash
scripts at <https://github.com/woodbusy/new-worktree>, consolidated into one
CLI with subcommands.

## Install

```sh
cargo install --path .
```

This installs a `work-shmirk` binary on your `$PATH`.

## Usage

```sh
work-shmirk new [--existing|-e] <worktree-name>
work-shmirk remove [<worktree-name>]
```

- `work-shmirk new feature-x` creates a new branch `feature-x`, adds a worktree
  for it alongside the main checkout, sets up `.worktree-local/`, optionally
  runs `copy_files` / creates symlinks, and launches Claude (in a tmux 3-pane
  layout when `$TMUX` is set, otherwise inline followed by `exec $SHELL`).
- `work-shmirk new -e existing-branch` checks out an existing branch instead
  of creating one.
- `work-shmirk remove feature-x` removes the symlinks, the worktree, and the
  local branch. Run with no argument from inside a worktree to remove that
  worktree. Refuses to default-target the main worktree.

### Worktree-name validation

Worktree names must be a single path component (no `/`, no `..`) and must not
contain shell metacharacters (`'`, `` ` ``, `$`, `\`, newline) or control
characters. The CLI rejects invalid names up front, before any subprocess
runs.

## Configuration

Config lives in `<repo>/.work-shmirk/`:

- `settings.json` — committed defaults
- `settings.local.json` — gitignored per-user overrides

The two files are deep-merged: nested objects merge key-by-key, while scalars
and arrays from the local file replace those in the base. This matches `jq`'s
`*` operator.

### Schema

```jsonc
{
  // Issue tracker integration. Affects the Claude prompt only.
  "issues": {
    "type": "linear",         // "github" | "linear" | "jira" | other
    "project": "ENG",         // fallback project prefix
    "cli": "linear",          // optional CLI for fetching issue details
    "skill": "linear-cli"     // optional Claude skill name
  },

  // Files to copy from .work-shmirk/<src> into <worktree>/<dest>.
  // Destinations are validated to remain inside the worktree.
  "copy_files": {
    "CLAUDE.md": "CLAUDE.md",
    ".env.template": ".env"
  },

  // Per-worktree symlink directory. Leading "~" and "$HOME" are expanded.
  // Empty string or omitted = feature disabled.
  "symlink_dir": "~/worktree-links",

  // Files inside the new worktree to expose as symlinks under
  // <symlink_dir>/<worktree-name>/. A single leading "." is stripped from
  // the link name (matches bash `${name#.}`).
  "symlink_links": [".env", ".envrc"],

  // Tmux integration.
  "tmux": {
    // If both this and issues.project are set, the first occurrence of
    // issues.project in the window name is replaced with this string.
    "project_name_substitution": "E"
  }
}
```

## Differences from new-worktree (bash)

### Config-format break

`.new-worktree/` (bash) is **not** compatible with `.work-shmirk/` (this tool).
There is no migration script.

### Bash quirks the Rust port preserves verbatim

- `symlink_links` strips a single leading `.` (bash `${link_source#.}`), not
  all leading dots. e.g. `..env` becomes `.env`, not `env`.
- `tmux.project_name_substitution` replaces only the first occurrence in the
  window name (bash `${var/x/y}` is single-replace).
- The issue-prefix regex is `^([A-Za-z]{2,7})-([0-9]{1,5})`, evaluated
  case-insensitively, and **the prefix case is preserved**. `eng-42-foo`
  produces `eng-42` in the prompt, not `ENG-42`.
- A branch named `issue-7` matches the 2-7 letter prefix regex first
  (`issue` is 5 letters), so `issue.type = "linear"` would produce `issue-7`,
  not just `7`. This matches the bash script.

### Deliberate divergences

- **Right-pane targeting:** uses the pane id returned from
  `tmux split-window -h -P -F '#{pane_id}'` instead of the bash script's
  `-t right`, which is a non-standard pane reference that silently misfires
  on default tmux configurations.
- **Tmux prompt delivery:** the Claude prompt is passed as a single shell-
  escaped argument (`claude '<escaped-prompt>'`), not via
  `claude "$(cat <tempfile>)"`. This eliminates the temp file and the
  shell command-substitution path, so prompt content (derived from branch
  names) cannot be re-expanded by the shell.
- **Path-containment validation:** `copy_files` destinations are required to
  stay inside the worktree root, and `symlink_links` entries are required to
  stay inside both the configured `symlink_dir` and the worktree. The bash
  script performs neither check, so a hostile config could write outside the
  worktree.
- **POSIX single-quote escaping:** all interpolated paths and the prompt
  string in tmux `send-keys` payloads are escaped via the standard
  `'` → `'\''` rewrite before being wrapped in single quotes.

## Binary override env vars

For testability the binary honors the following env vars (in all builds,
including release):

- `WORK_SHMIRK_CLAUDE_BIN` — path to the `claude` binary (default: `claude`)
- `WORK_SHMIRK_TMUX_BIN` — path to the `tmux` binary (default: `tmux`)
- `WORK_SHMIRK_GIT_BIN` — path to the `git` binary (default: `git`)

These are a deliberate trade-off: tests stub `claude` and `tmux` via these
variables. The security implication is that anyone who can set environment
variables in your shell (a sourced rc file, a `direnv` `.envrc`, an
inadvertently-trusted shell script) can redirect work-shmirk's subprocess
calls. Treat them like any other shell-resolved binary: only trust the
environment you trust.

## Test-only env var

- `WORK_SHMIRK_NO_EXEC=1` — in the inline (non-tmux) flow, skip the final
  `exec $SHELL`. Only useful for integration tests.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
