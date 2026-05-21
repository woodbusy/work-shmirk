//! Branch-name → issue reference parsing.
//!
//! Mirrors the bash control flow exactly: three regexes evaluated in order,
//! returning on first match. Prefix case is preserved verbatim (bash uses
//! `${BASH_REMATCH[1]}` directly).

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub prefix: Option<String>,
    pub number: u32,
}

fn re_prefix() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([A-Za-z]{2,7})-([0-9]{1,5})").unwrap())
}

fn re_issue() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"issue-([0-9]+)").unwrap())
}

fn re_leading_num() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([0-9]+)-").unwrap())
}

/// Parse a worktree/branch name and return the issue reference if any.
pub fn parse_issue(name: &str) -> Option<IssueRef> {
    if let Some(caps) = re_prefix().captures(name) {
        let prefix = caps.get(1)?.as_str().to_string();
        let number: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: Some(prefix),
            number,
        });
    }
    if let Some(caps) = re_issue().captures(name) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: None,
            number,
        });
    }
    if let Some(caps) = re_leading_num().captures(name) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: None,
            number,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uppercase_prefix() {
        let r = parse_issue("ENG-123-foo").unwrap();
        assert_eq!(r.prefix.as_deref(), Some("ENG"));
        assert_eq!(r.number, 123);
    }

    #[test]
    fn preserves_lowercase_prefix() {
        let r = parse_issue("eng-42-foo").unwrap();
        assert_eq!(r.prefix.as_deref(), Some("eng"));
        assert_eq!(r.number, 42);
    }

    #[test]
    fn parses_issue_prefix() {
        // "issue" is 5 letters and matches the 2-7 letter prefix regex first.
        // Matches bash: `[[ issue-7 =~ ^([A-Za-z]{2,7})-([0-9]{1,5}) ]]` captures
        // "issue" and "7". (The plan's expectation of "no prefix, num 7" was
        // wrong — scripts are authoritative and the script falls through
        // regex 1 here.)
        let r = parse_issue("issue-7").unwrap();
        assert_eq!(r.prefix.as_deref(), Some("issue"));
        assert_eq!(r.number, 7);
    }

    #[test]
    fn parses_issue_unanchored() {
        // First regex is anchored at start, so "foo-issue-7-bar" skips it
        // (no leading 2-7 letter prefix-dash-digit pattern at position 0:
        // "foo" is 3 letters but "foo-i" isn't followed by digits). It falls
        // through to the unanchored `issue-([0-9]+)` regex.
        let r = parse_issue("foo-issue-7-bar").unwrap();
        assert_eq!(r.prefix, None);
        assert_eq!(r.number, 7);
    }

    #[test]
    fn parses_leading_number_with_dash() {
        let r = parse_issue("42-foo").unwrap();
        assert_eq!(r.prefix, None);
        assert_eq!(r.number, 42);
    }

    #[test]
    fn bare_number_does_not_match() {
        assert_eq!(parse_issue("42"), None);
    }

    #[test]
    fn random_branch_does_not_match() {
        assert_eq!(parse_issue("feat/no-issue"), None);
        assert_eq!(parse_issue("just-a-name"), None);
    }

    #[test]
    fn eight_letter_prefix_falls_through() {
        // LONGESTX is 8 letters → fails regex 1.
        // Regex 2 (issue-...) doesn't match. Regex 3 (^[0-9]+-) doesn't match.
        assert_eq!(parse_issue("LONGESTX-1"), None);
    }

    #[test]
    fn mixed_case_prefix() {
        let r = parse_issue("Eng-1-foo").unwrap();
        assert_eq!(r.prefix.as_deref(), Some("Eng"));
        assert_eq!(r.number, 1);
    }
}
