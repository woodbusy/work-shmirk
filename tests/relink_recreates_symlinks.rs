mod common;

use common::TestEnv;
use predicates::str::contains;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

const WORKTREE: &str = "feature-x";

fn write_settings(cfg_dir: &Path, settings: serde_json::Value) {
    fs::create_dir_all(cfg_dir).unwrap();
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string(&settings).unwrap(),
    )
    .unwrap();
}

/// Links base outside the repo, main-repo config pointing at it, and a
/// worktree created by `new` with no links. Returns (worktree, links base).
fn setup(env: &TestEnv, links: &[&str]) -> (PathBuf, PathBuf) {
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    let main_cfg = env.repo_dir.join(".work-shmirk");
    write_settings(
        &main_cfg,
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": [] }),
    );
    env.bin().args(["new", WORKTREE]).assert().success();
    write_settings(
        &main_cfg,
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": links }),
    );
    (env.worktrees_root().join(WORKTREE), base)
}

fn relink(env: &TestEnv, worktree: &Path) -> assert_cmd::assert::Assert {
    env.bin().current_dir(worktree).arg("relink").assert()
}

fn expected_target(worktree: &Path, entry: &str) -> PathBuf {
    worktree.canonicalize().unwrap().join(entry)
}

#[test]
fn relink_recreates_deleted_link_dir() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &[".env", ".worktree-local"]);
    fs::write(wt.join(".env"), "x").unwrap();
    let link_dir = base.join(WORKTREE);
    fs::remove_dir_all(&link_dir).unwrap();

    relink(&env, &wt).success();

    assert_eq!(
        fs::read_link(link_dir.join("env")).unwrap(),
        expected_target(&wt, ".env")
    );
    assert_eq!(
        fs::read_link(link_dir.join("worktree-local")).unwrap(),
        expected_target(&wt, ".worktree-local")
    );
}

#[test]
fn relink_creates_absent_intermediates_for_nested_entry() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &["a/x", "b/y", ".env"]);
    for dir in ["a", "b"] {
        fs::create_dir_all(wt.join(dir)).unwrap();
    }
    fs::write(wt.join("a/x"), "x").unwrap();
    fs::write(wt.join("b/y"), "y").unwrap();
    fs::write(wt.join(".env"), "e").unwrap();
    let link_dir = base.join(WORKTREE);
    fs::create_dir(link_dir.join("a")).unwrap();

    relink(&env, &wt).success();

    assert!(fs::symlink_metadata(link_dir.join("b"))
        .unwrap()
        .file_type()
        .is_dir());
    for entry in ["a/x", "b/y"] {
        assert_eq!(
            fs::read_link(link_dir.join(entry)).unwrap(),
            expected_target(&wt, entry)
        );
    }
    assert_eq!(
        fs::read_link(link_dir.join("env")).unwrap(),
        expected_target(&wt, ".env")
    );
}

#[test]
fn relink_then_remove_tears_down_created_intermediates() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &[".claude/skills", ".claude/commands", "a/b/c"]);
    let link_dir = base.join(WORKTREE);

    relink(&env, &wt).success();
    assert!(fs::symlink_metadata(link_dir.join("claude/skills"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(link_dir.join("a/b/c"))
        .unwrap()
        .file_type()
        .is_symlink());

    env.bin().args(["remove", WORKTREE]).assert().success();

    assert!(
        fs::symlink_metadata(&link_dir).is_err(),
        "link dir should be gone"
    );
    assert!(!wt.exists(), "worktree dir should be gone");
}

#[test]
fn relink_replaces_existing_symlinks_and_is_repeatable() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &[".correct", ".elsewhere", ".broken"]);
    for entry in [".correct", ".elsewhere", ".broken"] {
        fs::write(wt.join(entry), entry).unwrap();
    }
    let link_dir = base.join(WORKTREE);
    let other = env.repo_dir.parent().unwrap().join("other-file");
    fs::write(&other, "other").unwrap();
    symlink(expected_target(&wt, ".correct"), link_dir.join("correct")).unwrap();
    symlink(&other, link_dir.join("elsewhere")).unwrap();
    symlink(base.join("does-not-exist"), link_dir.join("broken")).unwrap();

    for _ in 0..2 {
        relink(&env, &wt).success();
        for entry in [".correct", ".elsewhere", ".broken"] {
            let dest = link_dir.join(entry.trim_start_matches('.'));
            assert_eq!(fs::read_link(&dest).unwrap(), expected_target(&wt, entry));
        }
    }
    assert_eq!(fs::read_to_string(&other).unwrap(), "other");
}

#[test]
fn relink_creates_dangling_link_for_missing_source_with_warning() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &[".missing"]);

    relink(&env, &wt)
        .success()
        .stderr(contains("does not exist; the link will dangle"));

    let dest = base.join(WORKTREE).join("missing");
    assert!(fs::symlink_metadata(&dest)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&dest).unwrap(),
        expected_target(&wt, ".missing")
    );
}

#[test]
fn relink_rejects_worktree_name_argument() {
    let env = TestEnv::new();
    let (wt, base) = setup(&env, &[".env"]);

    env.bin()
        .current_dir(&wt)
        .args(["relink", "foo"])
        .assert()
        .failure()
        .stderr(contains("unexpected argument"));

    assert!(fs::read_dir(base.join(WORKTREE)).unwrap().next().is_none());
}

#[test]
fn relink_without_symlink_dir_changes_nothing() {
    let env = TestEnv::new();
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    env.bin().args(["new", WORKTREE]).assert().success();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_links": [".env"] }),
    );
    let wt = env.worktrees_root().join(WORKTREE);

    relink(&env, &wt)
        .success()
        .stderr(contains("symlink_dir is not set; nothing to relink"));

    assert!(!base.join(WORKTREE).exists());
}

#[test]
fn relink_refuses_main_worktree() {
    let env = TestEnv::new();
    let base = env.repo_dir.parent().unwrap().join("links");
    fs::create_dir_all(&base).unwrap();
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": base.to_string_lossy(), "symlink_links": [".env"] }),
    );

    env.bin()
        .arg("relink")
        .assert()
        .failure()
        .stderr(contains("main worktree"));

    assert!(!base.join("repo").exists());
}

#[test]
fn relink_prefers_per_worktree_config() {
    let env = TestEnv::new();
    let (wt, main_base) = setup(&env, &[".env"]);
    fs::write(wt.join(".env"), "x").unwrap();
    let worktree_base = env.repo_dir.parent().unwrap().join("worktree-links");
    fs::create_dir_all(&worktree_base).unwrap();
    write_settings(
        &wt.join(".work-shmirk"),
        serde_json::json!({
            "symlink_dir": worktree_base.to_string_lossy(),
            "symlink_links": [".env"]
        }),
    );

    relink(&env, &wt).success();

    assert_eq!(
        fs::read_link(worktree_base.join(WORKTREE).join("env")).unwrap(),
        expected_target(&wt, ".env")
    );
    assert!(fs::symlink_metadata(main_base.join(WORKTREE).join("env")).is_err());
}
