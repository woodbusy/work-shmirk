# Internal contract

A small set of behaviors and security choices are load-bearing — they shape the public surface or the security posture of the tool. They're documented here so changes don't quietly drift.

## Behaviors to preserve

These look like they could be "cleaned up" but are part of the documented surface:

- `symlink_links` strips a single leading `.` from each entry, not all leading dots. e.g. `..env` becomes `.env`, not `env`.
- `tmux.project_name_substitution` replaces only the first occurrence of `issues.project` in the window name.
- The issue-prefix regex is `^([A-Za-z]{2,7})-([0-9]{1,5})`, evaluated case-insensitively, and **the prefix case is preserved**. `eng-42-foo` produces `eng-42` in the prompt, not `ENG-42`.
- A branch named `issue-7` matches the 2-7 letter prefix regex (`issue` is 5 letters), so `issue.type = "linear"` produces `issue-7`, not just `7`.

## Security design

- **Right-pane targeting:** use the pane id captured from `tmux split-window -h -P -F '#{pane_id}'` to target subsequent `send-keys`. Don't substitute `-t right` — it's a non-standard pane reference that silently misfires on default tmux configurations.
- **Tmux prompt delivery:** the Claude prompt is passed as a single shell-escaped argument (`claude '<escaped-prompt>'`). Don't route it through `claude "$(cat <tempfile>)"` or any other shell-substitution path — prompt content is derived from branch names and must not be re-expanded by the shell.
- **Path-containment validation:** `copy_files` destinations must stay inside the worktree root, and `symlink_links` entries must stay inside both the configured `symlink_dir` and the worktree.
- **POSIX single-quote escaping:** all interpolated paths and the prompt string in tmux `send-keys` payloads are escaped via the standard `'` → `'\''` rewrite before being wrapped in single quotes.
- **Worktree-name validation up front:** `cli.rs::parse_worktree_name` rejects empty names, `/`, `.`, `..`, shell metacharacters (`'`, `` ` ``, `$`, `\`, newline), and control characters before any subprocess runs. The downstream POSIX escape is still applied, but the early reject produces a clear error.

## Trust boundary on `remove`

On `work-shmirk remove`, the binary reads config from the target worktree's `.work-shmirk/` first, falling back to the main repo's. A worktree checked out from an untrusted branch can therefore supply the `symlink_dir` and `symlink_links` used to clean up symlinks. The binary validates that every symlink candidate lies inside the configured per-worktree link dir before deleting it, so a hostile config cannot drive `remove_file` against arbitrary paths — but you should still only `remove` worktrees whose contents you trust.

## Binary override env vars

For testability the binary honors these env vars in all builds, including release:

- `WORK_SHMIRK_CLAUDE_BIN` — path to the `claude` binary (default: `claude`)
- `WORK_SHMIRK_TMUX_BIN` — path to the `tmux` binary (default: `tmux`)
- `WORK_SHMIRK_GIT_BIN` — path to the `git` binary (default: `git`)

Deliberate trade-off: tests stub `claude` and `tmux` via these variables. The security implication is that anyone who can set environment variables in your shell (a sourced rc file, a `direnv` `.envrc`, an inadvertently-trusted shell script) can redirect work-shmirk's subprocess calls. Treat them like any other shell-resolved binary: only trust the environment you trust.

When adding a new subprocess invocation, route it through the matching `*_bin()` helper in `src/{git,tmux,worktree}.rs` so tests can stub it.
