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
  runs `copy_files` / creates symlinks, copies `.claude/dev-workflow.local.yaml`
  from the current worktree if present, optionally sets up `personal-issues`, launches a
  tmux 3-pane layout (with `claude` in the bottom-left pane) when `$TMUX` is
  set, and prints the new worktree path to stdout before exiting.
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

Config lives in `<repo>/.work-shmirk/settings.json`. It is personal
configuration and must not be tracked by git: work-shmirk refuses to run when
the file (or the file a symlink points to inside the repo) is tracked, so a
repository cannot configure work-shmirk on your machine. Add `.work-shmirk/`
to your global gitignore.

The file may be a symlink to a file outside the repository, such as one in a
dotfiles repo.

Earlier versions also read `settings.local.json`. That file is no longer
supported, and work-shmirk errors when it finds one; move its contents into
`settings.json`.

### Schema

```jsonc
{
  // Issue tracker settings. Used to format the issue id passed to
  // `personal-issues set-current` (see `personal_issues`) and for tmux window
  // naming (see `tmux.project_name_substitution`).
  "issues": {
    "type": "linear",         // "github" | "linear" | "jira" | other
    // For "linear" and "jira" types, `project` controls case normalization:
    // when the branch prefix matches this value case-insensitively, the
    // configured spelling is used in the issue id (e.g. branch `eng-42-foo` +
    // `"project": "ENG"` → `ENG-42`). When no `project` is configured, or the
    // branch prefix does not match, the branch prefix is used verbatim; when
    // the branch has no prefix at all, `project` is used as the fallback
    // prefix. "github" uses the bare issue number (`42`); other types use the
    // branch prefix verbatim.
    "project": "APPFOO"
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
  "symlink_links": [".worktree-local", ".claude/skills"],

  // Tmux integration.
  "tmux": {
    // If both this and issues.project are set, the first occurrence of
    // issues.project in the window name is replaced with this string.
    "project_name_substitution": "AF"
  },

  // personal-issues integration. Default false. When true, `new` runs
  // `personal-issues set-current <issue-id>` in the new worktree, where
  // <issue-id> is parsed from the branch name and formatted per `issues`
  // above. Skipped with a warning when the branch name has no issue
  // reference. A non-zero exit fails `new`. (`.claude/dev-workflow.local.yaml`
  // is always copied into the new worktree beforehand, regardless of this
  // setting.)
  "personal_issues": false,

  // Claude launch settings.
  "claude": {
    // 1Password secret reference (must start with `op://`) holding a
    // service account token. When set, the bottom-left pane runs
    // `op read <ref>`, exports the result as OP_SERVICE_ACCOUNT_TOKEN for
    // claude only, and starts claude. If the read fails or is empty, claude
    // is not started and you land in your shell. Requires the `op` CLI
    // signed in to your own 1Password account.
    "op_token_ref": "op://Private/claude-service-account/credential"
  }
}
```

## Security notes

work-shmirk only loads an untracked `settings.json`, so a repository cannot
ship settings that drive it. This does not cover a file you create yourself
from untrusted content.

On `work-shmirk remove`, config is read from the target worktree first, so a
hostile worktree could supply `symlink_dir` and `symlink_links` values. The
binary validates containment before deleting, but you should still only
`remove` worktrees whose contents you trust. See
[docs/contract.md](docs/contract.md) for the full trust model and the
load-bearing behaviors maintainers should preserve.

### 1Password service account token

With `claude.op_token_ref` set, anything claude runs (including Bash tool
calls) inherits `OP_SERVICE_ACCOUNT_TOKEN`, so scope the service account to
only the vault Claude needs. The variable is unset again when claude exits, so
the shell you land in afterwards does not keep it.

## Binary override env vars

For testability the binary honors these env vars in all builds:

- `WORK_SHMIRK_CLAUDE_BIN` — path to the `claude` binary (default: `claude`)
- `WORK_SHMIRK_TMUX_BIN` — path to the `tmux` binary (default: `tmux`)
- `WORK_SHMIRK_GIT_BIN` — path to the `git` binary (default: `git`)
- `WORK_SHMIRK_PERSONAL_ISSUES_BIN` — path to the `personal-issues` binary
  (default: `personal-issues`)
- `WORK_SHMIRK_OP_BIN` — path to the 1Password `op` binary (default: `op`)

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
