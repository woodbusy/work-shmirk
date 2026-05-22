//! Shared test scaffolding: throwaway git repo + stub binaries for `claude`
//! and `tmux`. Pointed at via WORK_SHMIRK_{CLAUDE,TMUX}_BIN env vars.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TestEnv {
    pub repo_dir: PathBuf,
    pub stubs_dir: PathBuf,
    pub claude_log: PathBuf,
    pub tmux_log: PathBuf,
    _tmp: tempfile::TempDir,
}

impl TestEnv {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let repo_dir = root.join("repo");
        let stubs_dir = root.join("stubs");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&stubs_dir).unwrap();

        let claude_log = stubs_dir.join("claude.log");
        let tmux_log = stubs_dir.join("tmux.log");

        // Stub claude: log args (ignoring stdin) and exit 0. Also log the
        // current working directory so tests can assert claude was launched
        // from the worktree target (matches bash `cd <wt> && claude ...`).
        let claude_script = format!(
            "#!/bin/sh\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{log}\"\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> \"{log}\"; done\nexit 0\n",
            log = claude_log.display(),
        );
        write_executable(&stubs_dir.join("claude"), &claude_script);

        // Stub tmux: log all argv elements (one per line) then append --end--.
        // For invocations that produce a pane id in real tmux (display-message
        // or split-window/-P), print a distinct fake pane id per invocation.
        // The id is derived from the number of --end-- markers already present
        // in the log before this invocation appends its own:
        //   0 prior markers → %99 (display-message: top-left pane)
        //   1 prior marker  → %100 (first split-window -P: right pane)
        //   2 prior markers → %101 (second split-window -P: bottom pane)
        // This keeps per-TestEnv isolation automatic and avoids a counter file.
        let tmux_script = format!(
            concat!(
                "#!/bin/sh\n",
                "log='{log}'\n",
                // Count --end-- markers before this invocation writes its own.
                "n=$(grep -c '^--end--$' \"$log\" 2>/dev/null || printf '0')\n",
                "for a in \"$@\"; do printf '%s\\n' \"$a\" >> \"$log\"; done\n",
                "printf '\\n--end--\\n' >> \"$log\"\n",
                // display-message prints the current pane id.
                "case \"$1\" in\n",
                "  display-message)\n",
                "    printf '%%%s\\n' \"$((99 + n))\"\n",
                "    exit 0\n",
                "    ;;\n",
                "esac\n",
                // split-window with -P prints the new pane id.
                "for a in \"$@\"; do\n",
                "  case \"$a\" in\n",
                "    -P)\n",
                "      printf '%%%s\\n' \"$((99 + n))\"\n",
                "      exit 0\n",
                "      ;;\n",
                "  esac\n",
                "done\n",
                "exit 0\n",
            ),
            log = tmux_log.display(),
        );
        write_executable(&stubs_dir.join("tmux"), &tmux_script);

        // Initialize git repo with one commit.
        git(&repo_dir, &["init", "-q", "-b", "main"]);
        git(&repo_dir, &["config", "user.email", "test@example.com"]);
        git(&repo_dir, &["config", "user.name", "Test"]);
        git(&repo_dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo_dir.join("README.md"), "x\n").unwrap();
        git(&repo_dir, &["add", "README.md"]);
        git(&repo_dir, &["commit", "-q", "-m", "init"]);

        Self {
            repo_dir,
            stubs_dir,
            claude_log,
            tmux_log,
            _tmp: tmp,
        }
    }

    /// Build a `Command` for the binary under test with stubs and NO_EXEC set.
    pub fn bin(&self) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("work-shmirk").unwrap();
        cmd.current_dir(&self.repo_dir);
        cmd.env("WORK_SHMIRK_CLAUDE_BIN", self.stubs_dir.join("claude"));
        cmd.env("WORK_SHMIRK_TMUX_BIN", self.stubs_dir.join("tmux"));
        cmd.env("WORK_SHMIRK_NO_EXEC", "1");
        cmd.env("SHELL", "/bin/sh");
        // Ensure TMUX is NOT set unless a test explicitly opts in.
        cmd.env_remove("TMUX");
        cmd
    }

    pub fn worktrees_root(&self) -> &Path {
        // For a non-bare repo, worktrees_root == parent(.git) == repo_dir.
        &self.repo_dir
    }
}

pub fn write_executable(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, content).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

pub fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}
