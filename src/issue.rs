//! Branch-name → issue reference parsing.
//!
//! Four regexes evaluated in order, returning on first match:
//!   1. `^issue-([0-9]+)`             — bare issue number, no prefix
//!   2. `^([A-Za-z]{2,7})-([0-9]{1,5})` — generic 2-7 letter prefix
//!   3. `issue-([0-9]+)` (unanchored) — issue reference embedded in the name
//!   4. `^([0-9]+)-`                  — leading number with dash
//!
//! Prefix case is preserved verbatim (bash uses `${BASH_REMATCH[1]}` directly).

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub prefix: Option<String>,
    pub number: u32,
}

fn re_issue_anchored() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^issue-([0-9]{1,10})").unwrap())
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
    // Step 1: `issue-N` at the start of the name is always a bare issue number.
    if let Some(caps) = re_issue_anchored().captures(name) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: None,
            number,
        });
    }
    // Step 2: generic 2-7 letter prefix (e.g. ENG-123, gh-42).
    if let Some(caps) = re_prefix().captures(name) {
        let prefix = caps.get(1)?.as_str().to_string();
        let number: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: Some(prefix),
            number,
        });
    }
    // Step 3: `issue-N` embedded anywhere in the name.
    if let Some(caps) = re_issue().captures(name) {
        let number: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(IssueRef {
            prefix: None,
            number,
        });
    }
    // Step 4: leading number followed by a dash.
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
        // `issue-N` at the start of a name always produces a bare issue number
        // (no prefix). Step 1 (`^issue-([0-9]+)`) short-circuits before the
        // generic prefix regex ever runs.
        let r = parse_issue("issue-7").unwrap();
        assert_eq!(r.prefix, None);
        assert_eq!(r.number, 7);
    }

    #[test]
    fn parses_issue_number_any_length() {
        // Guards against future reordering that would re-introduce a 5-digit
        // cap; step 1 uses `[0-9]+` with no upper bound.
        let r = parse_issue("issue-12345").unwrap();
        assert_eq!(r.prefix, None);
        assert_eq!(r.number, 12345);
    }

    #[test]
    fn parses_issue_unanchored() {
        // "foo-issue-7-bar": step 1 (`^issue-`) does not match (string does
        // not start with `issue-`); step 2 (prefix regex) does not match
        // (`foo-` is followed by `issue`, not digits); falls to step 3
        // (unanchored `issue-([0-9]+)`).
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
