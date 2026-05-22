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

        // Stub tmux: log args, exit 0 (and for split-window -P, print a fake pane id).
        let tmux_script = format!(
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> \"{log}\"; done\nprintf '\\n--end--\\n' >> \"{log}\"\nfor a in \"$@\"; do\n  case \"$a\" in\n    -P) echo '%99'; exit 0 ;;\n  esac\ndone\nexit 0\n",
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
