//! Parse "Fixes #N" / "Closes #N" references from PR title/body.

use regex::Regex;
use std::sync::OnceLock;

/// Extract unique issue numbers referenced as fixed/closed by this text.
pub fn parse_closing_issue_numbers(text: &str) -> Vec<i32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:fix(?:es|ed)?|close[sd]?|resolve[sd]?)\s+#(\d+)\b",
        )
        .expect("regex")
    });
    let mut nums = Vec::new();
    for cap in re.captures_iter(text) {
        if let Some(n) = cap.get(1).and_then(|m| m.as_str().parse().ok()) {
            if n > 0 && !nums.contains(&n) {
                nums.push(n);
            }
        }
    }
    nums
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixes() {
        let t = "Fixes #12 and closes #3. Also Fix #12 again.";
        let mut n = parse_closing_issue_numbers(t);
        n.sort();
        assert_eq!(n, vec![3, 12]);
    }
}
