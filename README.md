# work-shmirk

A single Rust binary that wraps `git worktree` with optional Claude and tmux
integration.

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
  runs `copy_files` / creates symlinks, launches a tmux 3-pane layout when
  `$TMUX` is set, and prints the new worktree path to stdout before exiting.
- `work-shmirk new -e existing-branch` checks out an existing branch instead
  of creating one.
- `work-shmirk remove feature-x` removes the symlinks, the worktree, and the
  local branch. Run with no argument from inside a worktree to remove that
  worktree. Refuses to default-target the main worktree.

### Worktree-name validation

Worktree names must be a single path component (no `/`) and must not be the
path-traversal components `.` or `..`. They must not contain shell
metacharacters (`'`, `` ` ``, `$`, `\`, newline) or control characters. The
CLI rejects invalid names up front, before any subprocess runs.

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
    // For "linear" and "jira" types, `project` controls case normalization:
    // when the branch prefix matches this value case-insensitively, the
    // configured spelling is used in the emitted issue reference and fetch
    // command (e.g. branch `eng-42-foo` + `"project": "ENG"` → `ENG-42`).
    // When no `project` is configured, or the branch prefix does not match,
    // the branch prefix is used verbatim; when the branch has no prefix at
    // all, `project` is used as the fallback prefix. Users relying on a
    // case-sensitive Linear or Jira CLI should configure `project` or use
    // canonical-case branch names. For "github" and other types, `project`
    // is unused.
    "project": "ENG",
    "cli": "linear",          // optional CLI for fetching issue details
    "skill": "linear-cli"     // optional Claude skill name
  },

  // Files to copy from .work-shmirk/<src> into <worktree>/<dest>.
  // Destinations are validated to remain inside the worktree.
  "copy_files": {
    "CLAUDE.md": "CLAUDE.md",
    ".env.template": ".env"
  },

  // Per-worktree symlink directory. Leading "~", "$HOME", and "${HOME}" are
  // expanded to the user's home directory. "~user" is not supported.
  // Empty string or omitted = feature disabled.
  "symlink_dir": "~/worktree-links",

  // Files inside the new worktree to expose as symlinks under
  // <symlink_dir>/<worktree-name>/. All leading dots are stripped from the
  // link name: ".env" → "env", "..env" → "env". Entries that are empty or
  // that strip to empty ("..", "...", etc.) are rejected with an error.
  "symlink_links": [".env", ".envrc"],

  // Tmux integration.
  "tmux": {
    // If both this and issues.project are set, the first occurrence of
    // issues.project in the window name is replaced with this string.
    "project_name_substitution": "E"
  }
}
```

## Security notes

On `work-shmirk remove`, config is read from the target worktree first, so a
hostile worktree could supply `symlink_dir` and `symlink_links` values. The
binary validates containment before deleting, but you should still only
`remove` worktrees whose contents you trust. See
[docs/contract.md](docs/contract.md) for the full trust model and the
load-bearing behaviors maintainers should preserve.

## Binary override env vars

For testability the binary honors these env vars in all builds:

- `WORK_SHMIRK_CLAUDE_BIN` — path to the `claude` binary (default: `claude`)
- `WORK_SHMIRK_TMUX_BIN` — path to the `tmux` binary (default: `tmux`)
- `WORK_SHMIRK_GIT_BIN` — path to the `git` binary (default: `git`)

These can redirect work-shmirk's subprocess calls — see
[docs/contract.md](docs/contract.md) for the trust model.

## Shell wrapper

`work-shmirk new` prints the worktree path to stdout on success. A shell
function can capture it to land in the new worktree automatically:

```sh
ws() {
  local target
  target=$(work-shmirk new "$@") && cd "$target"
}
```

Add this to your shell's rc file. The shell wrapper `cd`s into the new
worktree after `work-shmirk new` exits. When inside tmux a 3-pane layout is
also set up.

## Development

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
