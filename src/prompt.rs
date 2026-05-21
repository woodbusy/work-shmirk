//! Claude prompt construction.
//!
//! Reproduces the exact text emitted by `new-worktree`, including:
//!   - the lead sentence with the branch name
//!   - the conditional issue block (only when `issues.type` is set)
//!   - the optional skill line
//!   - the closing instructions

use crate::config::IssuesConfig;
use crate::issue::IssueRef;

/// Build the prompt string passed to `claude`.
pub fn build_prompt(
    worktree_name: &str,
    issue_ref: Option<&IssueRef>,
    issues_cfg: Option<&IssuesConfig>,
) -> String {
    let mut out =
        format!("You are helping set up a new git worktree for a branch named '{worktree_name}'.");

    if let Some(issue) = issue_ref {
        if let Some(cfg) = issues_cfg {
            if let Some(issue_type) = cfg.type_.as_deref() {
                let (issue_ref_text, fetch_cmd) = format_issue_ref(issue, cfg, issue_type);

                out.push_str("\n\nIMPORTANT: This branch references issue ");
                out.push_str(&issue_ref_text);
                out.push('.');

                if let Some(cmd) = fetch_cmd {
                    out.push_str(" You MUST read this issue using the command '");
                    out.push_str(&cmd);
                    out.push_str("' before asking the user any questions.");
                }

                if let Some(skill) = cfg.skill.as_deref() {
                    out.push_str(" Use the /");
                    out.push_str(skill);
                    out.push_str(" skill to interact with the issue tracker.");
                }
            }
        }
    }

    out.push_str(
        "\n\nPlease interact with the user to understand the goal and purpose of this branch/worktree, then create a file at .worktree-local/context.md (which directory already exists and does not need to be created) that provides context for future Claude agents working in this worktree.\n\nCRITICAL: The context file should be BRIEF (20-200 words) and capture only the essential goals. Details available elsewhere (e.g., in a GitHub/Jira/Linear issue) should be REFERENCED not duplicated in the context file. Implementation details should not be included either.\n\nThe context should include:\n- The branch purpose and goals but not implementation details\n- Links to related issues, PRs, or documentation\n- Any critical constraints or considerations\n\nKeep it concise - link to details rather than repeating them.\n\nPlease start by asking the user about their goals for this branch.\n\nIMPORTANT: After you have created the context.md file, your task is complete. Do not continue the conversation or offer additional help.",
    );

    out
}

/// Returns `(issue_ref_text, fetch_cmd)` per issue type. `fetch_cmd` is None
/// when no command should be emitted in the prompt.
fn format_issue_ref(
    issue: &IssueRef,
    cfg: &IssuesConfig,
    issue_type: &str,
) -> (String, Option<String>) {
    match issue_type {
        "github" => (
            format!("#{}", issue.number),
            Some(format!("gh issue view {}", issue.number)),
        ),
        "linear" | "jira" => {
            // Prefix from branch wins; fall back to configured project.
            let proj = issue
                .prefix
                .as_deref()
                .or(cfg.project.as_deref())
                .filter(|s| !s.is_empty());
            let issue_ref_text = match proj {
                Some(p) => format!("{p}-{}", issue.number),
                None => issue.number.to_string(),
            };
            let fetch_cmd = cfg
                .cli
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|cli| format!("{cli} issue view {issue_ref_text}"));
            (issue_ref_text, fetch_cmd)
        }
        _ => {
            let issue_ref_text = match issue.prefix.as_deref().filter(|s| !s.is_empty()) {
                Some(p) => format!("{p}-{}", issue.number),
                None => issue.number.to_string(),
            };
            (issue_ref_text, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(
        t: Option<&str>,
        project: Option<&str>,
        cli: Option<&str>,
        skill: Option<&str>,
    ) -> IssuesConfig {
        IssuesConfig {
            type_: t.map(String::from),
            project: project.map(String::from),
            cli: cli.map(String::from),
            skill: skill.map(String::from),
        }
    }

    #[test]
    fn no_issue_no_type() {
        let p = build_prompt("feature-x", None, None);
        assert!(p.starts_with(
            "You are helping set up a new git worktree for a branch named 'feature-x'."
        ));
        assert!(!p.contains("IMPORTANT: This branch references issue"));
    }

    #[test]
    fn issue_but_no_type_omits_block() {
        let issue = IssueRef {
            prefix: Some("ENG".into()),
            number: 1,
        };
        let p = build_prompt("ENG-1", Some(&issue), Some(&cfg(None, None, None, None)));
        assert!(!p.contains("references issue"));
    }

    #[test]
    fn github_type() {
        let issue = IssueRef {
            prefix: None,
            number: 42,
        };
        let p = build_prompt(
            "issue-42",
            Some(&issue),
            Some(&cfg(Some("github"), None, None, None)),
        );
        assert!(p.contains("references issue #42."));
        assert!(p.contains("'gh issue view 42'"));
    }

    #[test]
    fn linear_with_branch_prefix_preserves_case() {
        let issue = IssueRef {
            prefix: Some("eng".into()),
            number: 42,
        };
        let p = build_prompt(
            "eng-42-foo",
            Some(&issue),
            Some(&cfg(Some("linear"), Some("ENG"), Some("linear"), None)),
        );
        assert!(p.contains("references issue eng-42."));
        assert!(p.contains("'linear issue view eng-42'"));
        assert!(!p.contains("ENG-42"));
    }

    #[test]
    fn linear_without_cli_omits_fetch() {
        let issue = IssueRef {
            prefix: Some("ENG".into()),
            number: 1,
        };
        let p = build_prompt(
            "ENG-1",
            Some(&issue),
            Some(&cfg(Some("linear"), None, None, None)),
        );
        assert!(p.contains("references issue ENG-1."));
        assert!(!p.contains("issue view"));
    }

    #[test]
    fn jira_with_config_project_fallback() {
        let issue = IssueRef {
            prefix: None,
            number: 5,
        };
        let p = build_prompt(
            "issue-5",
            Some(&issue),
            Some(&cfg(Some("jira"), Some("PROJ"), Some("jira-cli"), None)),
        );
        assert!(p.contains("references issue PROJ-5."));
    }

    #[test]
    fn other_type_uses_prefix_or_number() {
        let issue = IssueRef {
            prefix: Some("FOO".into()),
            number: 9,
        };
        let p = build_prompt(
            "FOO-9",
            Some(&issue),
            Some(&cfg(Some("custom"), None, None, None)),
        );
        assert!(p.contains("references issue FOO-9."));
        assert!(!p.contains("issue view"));
    }

    #[test]
    fn skill_line_appended() {
        let issue = IssueRef {
            prefix: None,
            number: 1,
        };
        let p = build_prompt(
            "issue-1",
            Some(&issue),
            Some(&cfg(Some("github"), None, None, Some("gh-skill"))),
        );
        assert!(p.contains("Use the /gh-skill skill"));
    }

    #[test]
    fn closing_block_present() {
        let p = build_prompt("x", None, None);
        assert!(p.contains(".worktree-local/context.md"));
        assert!(p.contains("Do not continue the conversation"));
    }
}
