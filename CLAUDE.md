# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Map for agents: start here, follow pointers. Each doc owns one thing.

## Documentation map

| Doc | Owns |
|-----|------|
| [README.md](README.md) | User-facing: install, usage, config schema, env vars, dev commands |
| [docs/contract.md](docs/contract.md) | Internal contract: load-bearing behaviors, security design, trust boundary on `remove`, env-var trust model |

## Quick reference

```sh
cargo build
cargo test
cargo test --test new_creates_worktree       # single integration test file
cargo test -- some_test_name                 # single unit test by name
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo install --path .                       # install the work-shmirk binary
```

## Architecture (one screen)

The crate splits a thin CLI dispatcher (`lib.rs::run` → `cli.rs`) from per-concern modules so each can be unit-tested in isolation:

- `worktree.rs` orchestrates the `new` flow; `removal.rs` orchestrates `remove`. These are the only modules that compose the others.
- `git.rs` wraps the `git` CLI (each call takes a `cwd` so tests can drive it without a process-wide chdir).
- `config.rs` loads `.work-shmirk/settings.json` + `settings.local.json` and deep-merges them with `jq *` semantics. Also home to path-containment validation.
- `tmux.rs`, `copyfiles.rs`, `symlinks.rs` are the side-effect modules invoked from `worktree.rs` / `removal.rs`.
- `issue.rs`, `prompt.rs` are pure: branch-name → issue ref → Claude prompt string.
- `cli.rs::parse_worktree_name` rejects names containing characters hostile on filesystems, in branch-name tool integrations, and in shells; it is defense-in-depth rather than a load-bearing control. Downstream code also POSIX-single-quote-escapes everything it interpolates.

The Rust process **never changes its own cwd**. Every module takes paths explicitly so tests can run in parallel against tempdir-backed repos.

## Workflow rules

- **Respect the contract.** Every change must honor [docs/contract.md](docs/contract.md). Don't "clean up" the documented behaviors and don't relax the documented security choices without explicit user direction.
- **New subprocess calls go through `*_bin()`.** Route every `claude`/`tmux`/`git` invocation through the matching helper so tests can stub it via `WORK_SHMIRK_{CLAUDE,TMUX,GIT}_BIN`.
- **Integration tests use `TestEnv`.** `tests/common/mod.rs::TestEnv` builds a tempdir-backed git repo, writes stub `claude`/`tmux` scripts, and points the binary at them via `WORK_SHMIRK_{CLAUDE,TMUX}_BIN`. `TMUX` is unset by default; tests that exercise the tmux flow set it explicitly.
- **Ship docs with the code change.** When you change behavior covered by the contract or the architecture summary above, update the relevant doc in the same PR.
