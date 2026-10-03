mod common;

use common::TestEnv;
use std::fs;

const DEV_WORKFLOW_CONFIG: &str = ".claude/dev-workflow.local.yaml";

fn write_settings(env: &TestEnv, settings: serde_json::Value) {
    let cfg_dir = env.repo_dir.join(".work-shmirk");
    fs::create_dir_all(&cfg_dir).unwrap();
    fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
}

#[test]
fn personal_issues_copies_config_and_sets_current_issue() {
    let env = TestEnv::new();
    write_settings(
        &env,
        serde_json::json!({
            "personal_issues": true,
            "issues": { "type": "linear", "project": "ENG" }
        }),
    );
    fs::create_dir_all(env.repo_dir.join(".claude")).unwrap();
    fs::write(env.repo_dir.join(DEV_WORKFLOW_CONFIG), "backend: linear\n").unwrap();

    let output = env.bin().args(["new", "eng-42-foo"]).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).to_string();

    let wt = env.worktrees_root().join("eng-42-foo");
    assert_eq!(
        fs::read_to_string(wt.join(DEV_WORKFLOW_CONFIG)).unwrap(),
        "backend: linear\n"
    );

    let log = fs::read_to_string(&env.personal_issues_log).unwrap();
    let expected_pwd = wt.canonicalize().unwrap();
    assert_eq!(
        log,
        format!("PWD={}\nset-current\nENG-42\n", expected_pwd.display()),
    );

    assert!(
        !stdout.contains("personal-issues-stdout"),
        "personal-issues stdout leaked into ours: {stdout}"
    );
    assert_eq!(stdout.trim(), expected_pwd.to_string_lossy());
}

#[test]
fn personal_issues_skips_set_current_without_issue_ref() {
    let env = TestEnv::new();
    write_settings(&env, serde_json::json!({ "personal_issues": true }));

    env.bin()
        .args(["new", "no-issue-here"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "skipping personal-issues set-current",
        ));

    assert!(!env.personal_issues_log.exists());
}

#[test]
fn dev_workflow_config_copied_with_personal_issues_disabled() {
    let env = TestEnv::new();
    fs::create_dir_all(env.repo_dir.join(".claude")).unwrap();
    fs::write(env.repo_dir.join(DEV_WORKFLOW_CONFIG), "backend: work\n").unwrap();

    env.bin().args(["new", "ENG-1-foo"]).assert().success();

    let wt = env.worktrees_root().join("ENG-1-foo");
    assert_eq!(
        fs::read_to_string(wt.join(DEV_WORKFLOW_CONFIG)).unwrap(),
        "backend: work\n"
    );
    assert!(!env.personal_issues_log.exists());
}

#[test]
fn personal_issues_failure_fails_new() {
    let env = TestEnv::new();
    write_settings(&env, serde_json::json!({ "personal_issues": true }));
    common::write_executable(
        &env.stubs_dir.join("personal-issues"),
        "#!/bin/sh\nexit 3\n",
    );

    env.bin()
        .args(["new", "ENG-7-foo"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "personal-issues set-current ENG-7",
        ));
}
