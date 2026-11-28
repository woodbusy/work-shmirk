mod common;

use common::{git, TestEnv};
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

fn scratch_root(env: &TestEnv) -> PathBuf {
    env.repo_dir.parent().unwrap().to_path_buf()
}

/// Create the worktree under a benign main-repo config, then point the
/// main-repo config at `symlink_dir` / `links` for the `relink` under test.
fn setup(env: &TestEnv, symlink_dir: &str, links: &[&str]) -> PathBuf {
    let benign_base = scratch_root(env).join("links");
    fs::create_dir_all(&benign_base).unwrap();
    let main_cfg = env.repo_dir.join(".work-shmirk");
    write_settings(
        &main_cfg,
        serde_json::json!({ "symlink_dir": benign_base.to_string_lossy(), "symlink_links": [] }),
    );
    env.bin().args(["new", WORKTREE]).assert().success();
    write_settings(
        &main_cfg,
        serde_json::json!({ "symlink_dir": symlink_dir, "symlink_links": links }),
    );
    env.worktrees_root().join(WORKTREE)
}

fn relink(env: &TestEnv, worktree: &Path) -> assert_cmd::assert::Assert {
    env.bin().current_dir(worktree).arg("relink").assert()
}

fn link_dir_entries(link_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(link_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn relink_rejects_dest_escape_and_changes_nothing() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let sentinel_target = scratch_root(&env).join("sentinel-target");
    fs::write(&sentinel_target, "x").unwrap();
    let sentinel_link = scratch_root(&env).join("sentinel-link");
    symlink(&sentinel_target, &sentinel_link).unwrap();
    let wt = setup(
        &env,
        &base.to_string_lossy(),
        &[".env", "../../sentinel-link"],
    );

    relink(&env, &wt).failure().stderr(contains(
        "validating symlink dest for '../../sentinel-link'",
    ));

    assert_eq!(fs::read_link(&sentinel_link).unwrap(), sentinel_target);
    assert!(link_dir_entries(&base.join(WORKTREE)).is_empty());
}

#[test]
fn relink_rejects_absolute_entry_outside_worktree_and_changes_nothing() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let sentinel = scratch_root(&env).join("sentinel");
    fs::write(&sentinel, "keep").unwrap();
    let wt = setup(
        &env,
        &base.to_string_lossy(),
        &[".env", &sentinel.to_string_lossy()],
    );

    relink(&env, &wt).failure().stderr(contains("escapes"));

    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep");
    assert!(link_dir_entries(&base.join(WORKTREE)).is_empty());
}

#[test]
fn relink_rejects_tilde_user_symlink_dir() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let wt = setup(&env, "~alice/links", &[".env"]);

    relink(&env, &wt).failure().stderr(contains(
        "`~user`-style home-dir expansion is not supported",
    ));

    assert!(link_dir_entries(&base.join(WORKTREE)).is_empty());
}

#[test]
fn relink_refuses_symlinked_link_dir() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let wt = setup(&env, &base.to_string_lossy(), &[".env"]);
    fs::write(wt.join(".env"), "x").unwrap();
    let outside = scratch_root(&env).join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("env"), "sentinel").unwrap();
    let link_dir = base.join(WORKTREE);
    fs::remove_dir(&link_dir).unwrap();
    symlink(&outside, &link_dir).unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("is a symlink or not a directory"));

    let sentinel_meta = fs::symlink_metadata(outside.join("env")).unwrap();
    assert!(sentinel_meta.file_type().is_file());
    assert_eq!(fs::read_to_string(outside.join("env")).unwrap(), "sentinel");
    assert_eq!(fs::read_link(&link_dir).unwrap(), outside);
}

#[test]
fn relink_skips_symlinked_intermediate_component() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let wt = setup(&env, &base.to_string_lossy(), &[".claude/skills"]);
    fs::create_dir_all(wt.join(".claude/skills")).unwrap();
    let outside = scratch_root(&env).join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, base.join(WORKTREE).join("claude")).unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("skipping '.claude/skills'"))
        .stderr(contains("is a symlink or not a directory"));

    assert!(link_dir_entries(&outside).is_empty());
}

#[test]
fn relink_skips_non_directory_intermediate() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let wt = setup(&env, &base.to_string_lossy(), &[".claude/skills"]);
    fs::create_dir_all(wt.join(".claude/skills")).unwrap();
    let blocking_file = base.join(WORKTREE).join("claude");
    fs::write(&blocking_file, "mine").unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("skipping '.claude/skills'"))
        .stderr(contains("is a symlink or not a directory"));

    assert!(fs::symlink_metadata(&blocking_file)
        .unwrap()
        .file_type()
        .is_file());
    assert_eq!(fs::read_to_string(&blocking_file).unwrap(), "mine");
}

#[test]
fn relink_refuses_link_dir_equal_to_worktree() {
    let env = TestEnv::new();
    let wt = setup(&env, &env.repo_dir.to_string_lossy(), &[".env"]);
    fs::write(wt.join(".env"), "x").unwrap();
    symlink("README.md", wt.join("env")).unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("overlaps the worktree"));

    assert_eq!(
        fs::read_link(wt.join("env")).unwrap(),
        Path::new("README.md")
    );
}

#[test]
fn relink_refuses_link_dir_that_contains_worktree() {
    // A worktree at <repo>/holder/wt/wt with symlink_dir <repo>/holder makes
    // the link dir <repo>/holder/wt an ancestor of the worktree, so the entry
    // `wt/.env` would land on the worktree's own `.env`.
    let env = TestEnv::new();
    let holder = env.repo_dir.join("holder");
    let wt = holder.join("wt").join("wt");
    fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        &env.repo_dir,
        &["worktree", "add", "-q", &wt.to_string_lossy(), "-b", "wt"],
    );
    write_settings(
        &env.repo_dir.join(".work-shmirk"),
        serde_json::json!({ "symlink_dir": holder.to_string_lossy(), "symlink_links": ["wt/.env"] }),
    );
    fs::create_dir_all(wt.join("wt")).unwrap();
    fs::write(wt.join("wt/.env"), "x").unwrap();
    symlink("README.md", wt.join(".env")).unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("overlaps the worktree"));

    assert_eq!(
        fs::read_link(wt.join(".env")).unwrap(),
        Path::new("README.md")
    );
}

#[test]
fn relink_leaves_regular_file_at_dest_and_links_the_rest() {
    let env = TestEnv::new();
    let base = scratch_root(&env).join("links");
    let wt = setup(&env, &base.to_string_lossy(), &[".env", ".other"]);
    fs::write(wt.join(".env"), "x").unwrap();
    fs::write(wt.join(".other"), "y").unwrap();
    let link_dir = base.join(WORKTREE);
    fs::write(link_dir.join("env"), "mine").unwrap();

    relink(&env, &wt)
        .failure()
        .stderr(contains("skipping '.env'"))
        .stderr(contains("exists and is not a symlink"));

    assert_eq!(fs::read_to_string(link_dir.join("env")).unwrap(), "mine");
    assert!(fs::symlink_metadata(link_dir.join("env"))
        .unwrap()
        .file_type()
        .is_file());
    assert_eq!(
        fs::read_link(link_dir.join("other")).unwrap(),
        wt.canonicalize().unwrap().join(".other")
    );
}

#[test]
fn relink_refuses_missing_symlink_dir_without_creating_it() {
    let env = TestEnv::new();
    let missing = scratch_root(&env).join("missing-base");
    let wt = setup(&env, &missing.to_string_lossy(), &[".env"]);

    relink(&env, &wt)
        .failure()
        .stderr(contains("does not exist; create it before running relink"));

    assert!(!missing.exists());
}
