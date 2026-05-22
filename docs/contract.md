# Internal contract

A small set of behaviors and security choices are load-bearing — they shape the public surface or the security posture of the tool. They're documented here so changes don't quietly drift.

## Behaviors to preserve

These look like they could be "cleaned up" but are part of the documented surface:

- `symlink_links` strips a single leading `.` from each entry, not all leading dots. e.g. `..env` becomes `.env`, not `env`.
- `tmux.project_name_substitution` replaces only the first occurrence of `issues.project` in the window name.
- The issue-prefix regex is `^([A-Za-z]{2,7})-([0-9]{1,5})`, evaluated case-insensitively, and **the prefix case is preserved**. `eng-42-foo` produces `eng-42` in the prompt, not `ENG-42`.
- A branch named `issue-7` matches the 2-7 letter prefix regex (`issue` is 5 letters), so `issue.type = "linear"` produces `issue-7`, not just `7`.

## Security design

- **Pane id targeting:** both the right-pane split and the bottom-left split capture their pane ids via `tmux split-window -P -F '#{pane_id}'`. The starting pane id is captured via `display-message -p '#{pane_id}'` before the first split. `respawn-pane -t <id>` and `select-pane -t <id>` reference these captured ids, not layout-relative names like `-t right` or `-t top-left` which silently misfire on default tmux configurations.
- **Tmux prompt delivery:** pane commands are launched via `split-window -c <dir> -- sh -c '<script>'` and `respawn-pane -k -c <dir> -- sh -c '<script>'`. The worktree path is an argv element to tmux (`-c`) and never traverses a shell. The Claude prompt is passed as a single shell-escaped argument inside the bottom-left `sh -c` script (`claude '<escaped-prompt>'`). The claude binary path is also shell-escaped. No other user-derived strings are interpolated into a shell. Don't route the prompt through `claude "$(cat <tempfile>)"` or any shell-substitution path — prompt content is derived from branch names and must not be re-expanded by the shell.
- **Path-containment validation:** `copy_files` destinations must stay inside the worktree root, and `symlink_links` entries must stay inside both the configured `symlink_dir` and the worktree.
- **POSIX single-quote escaping:** applies only to the **prompt** and the **claude binary path** inside the bottom-left pane's `sh -c` script. The worktree path is passed via tmux `-c` (argv) and requires no shell escaping.
- **Worktree-name validation up front:** `cli.rs::parse_worktree_name` rejects empty names, `/`, `.`, `..`, shell metacharacters (`'`, `` ` ``, `$`, `\`, newline), and control characters before any subprocess runs. Since the worktree path now flows through tmux `-c` (not a shell string), this validator is **defense-in-depth rather than load-bearing** — the characters in the reject-list are still hostile on filesystems, in tool integrations that read branch names, and in shells users run inside the worktree.
- **`respawn-pane -k` behavior:** the top-left pane (the one `work-shmirk` was invoked from) is replaced by a fresh `$SHELL` cwd'd in the worktree via `respawn-pane -k`. `$SHELL` in the `respawn-pane` payload is intentionally expanded from the tmux session environment at pane startup time (not hardcoded), so the user gets their configured shell. Two intentional trade-offs: (a) the original pane's **scrollback is discarded** — whatever was visible above the `work-shmirk new` invocation is gone; (b) the new shell **does not inherit the original shell's in-process env mutations** (e.g. `export FOO=...` or `OLDPWD` from prior `cd`s) — it inherits tmux's session environment subject to `update-environment`. These are acceptable for the target use case; the benefit is a clean shell history with no `cd` keystrokes.

## Trust boundary on `remove`

On `work-shmirk remove`, the binary reads config from the target worktree's `.work-shmirk/` first, falling back to the main repo's. A worktree checked out from an untrusted branch can therefore supply the `symlink_dir` and `symlink_links` used to clean up symlinks. The binary validates that every symlink candidate lies inside the configured per-worktree link dir before deleting it, so a hostile config cannot drive `remove_file` against arbitrary paths — but you should still only `remove` worktrees whose contents you trust.

## Binary override env vars

For testability the binary honors these env vars in all builds, including release:

- `WORK_SHMIRK_CLAUDE_BIN` — path to the `claude` binary (default: `claude`)
- `WORK_SHMIRK_TMUX_BIN` — path to the `tmux` binary (default: `tmux`)
- `WORK_SHMIRK_GIT_BIN` — path to the `git` binary (default: `git`)

Deliberate trade-off: tests stub `claude` and `tmux` via these variables. The security implication is that anyone who can set environment variables in your shell (a sourced rc file, a `direnv` `.envrc`, an inadvertently-trusted shell script) can redirect work-shmirk's subprocess calls. Treat them like any other shell-resolved binary: only trust the environment you trust.

When adding a new subprocess invocation, route it through the matching `*_bin()` helper in `src/{git,tmux,worktree}.rs` so tests can stub it.
