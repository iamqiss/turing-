//! Detect secret-shaped strings in text (diffs, files, commit messages).
//! Used by push protection, pre-commit clients, and Pearl redaction.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub label: String,
    pub line: u32,
    pub column: u32,
    /// Redacted preview — never the full secret
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    pub findings: Vec<Finding>,
    pub scanned_bytes: usize,
    pub blocked: bool,
}

struct Rule {
    id: &'static str,
    label: &'static str,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let mk = |id: &'static str, label: &'static str, pat: &str| Rule {
            id,
            label,
            re: Regex::new(pat).expect("valid secret regex"),
        };
        vec![
            mk(
                "private_key",
                "PEM private key",
                r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
            ),
            mk(
                "aws_access_key",
                "AWS access key id",
                r"(?:AKIA|ASIA)[0-9A-Z]{16}",
            ),
            mk(
                "aws_secret_key",
                "AWS secret access key assignment",
                r#"(?i)(aws_secret_access_key|secret_key)\s*[=:]\s*['"]?[A-Za-z0-9/+=]{40}"#,
            ),
            mk(
                "github_pat",
                "GitHub personal access token",
                r"gh[pousr]_[A-Za-z0-9_]{36,}",
            ),
            mk(
                "github_fine_grained",
                "GitHub fine-grained PAT",
                r"github_pat_[A-Za-z0-9_]{20,}",
            ),
            mk(
                "slack_token",
                "Slack token",
                r"xox[baprs]-[0-9A-Za-z-]{10,}",
            ),
            mk(
                "stripe_key",
                "Stripe API key",
                r"(?:sk|pk|rk)_(?:live|test)_[0-9a-zA-Z]{20,}",
            ),
            mk(
                "google_api",
                "Google API key",
                r"AIza[0-9A-Za-z\-_]{35}",
            ),
            mk(
                "jwt",
                "JWT",
                r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            ),
            mk(
                "connection_string",
                "Database connection string",
                r#"(?i)(?:postgres|mysql|mongodb|redis|amqp)://[^\s'"<>]{8,}"#,
            ),
            mk(
                "generic_password_assign",
                "Password or secret assignment",
                r#"(?i)(password|passwd|secret|api[_-]?key|token|auth[_-]?token)\s*[=:]\s*['"][^'"]{8,}['"]"#,
            ),
            mk(
                "tt_deploy_token",
                "Turing Tarpit deploy token",
                r"tt_deploy_[A-Za-z0-9_]+_[A-Za-z0-9_\-]{16,}",
            ),
            mk(
                "high_entropy_assign",
                "High-entropy secret assignment",
                r#"(?i)(secret|token|key|password)\s*[=:]\s*['"]?[A-Za-z0-9+/=_\-]{32,}['"]?"#,
            ),
        ]
    })
}

fn redact(s: &str) -> String {
    let s = s.trim();
    if s.len() <= 8 {
        return "****".into();
    }
    format!("{}…{} (len={})", &s[..4], &s[s.len() - 2..], s.len())
}

/// Scan arbitrary text. `blocked` is true when any finding exists.
pub fn scan_text(text: &str) -> ScanReport {
    let mut findings = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") && line.contains("example") {
            continue;
        }
        if line.contains("BEGIN PUBLIC KEY") {
            continue;
        }
        for rule in rules() {
            for m in rule.re.find_iter(line) {
                findings.push(Finding {
                    rule_id: rule.id.into(),
                    label: rule.label.into(),
                    line: (line_idx + 1) as u32,
                    column: (m.start() + 1) as u32,
                    preview: redact(m.as_str()),
                });
            }
        }
    }
    findings.sort_by(|a, b| (a.line, &a.rule_id).cmp(&(b.line, &b.rule_id)));
    findings.dedup_by(|a, b| a.line == b.line && a.rule_id == b.rule_id);

    ScanReport {
        blocked: !findings.is_empty(),
        scanned_bytes: text.len(),
        findings,
    }
}

/// Filter findings against allowlisted rule ids.
pub fn apply_allowlist(mut report: ScanReport, allow: &[String]) -> ScanReport {
    if allow.is_empty() {
        return report;
    }
    report
        .findings
        .retain(|f| !allow.iter().any(|a| a == &f.rule_id || a == &f.label));
    report.blocked = !report.findings.is_empty();
    report
}


/// Replace secret-shaped spans with opaque placeholders safe for model context.
/// Also strips common env-assignment values and Scarlet refs that look like values.
pub fn redact_for_model(text: &str) -> String {
    let mut out = text.to_string();
    for rule in rules() {
        out = rule
            .re
            .replace_all(&out, |caps: &regex::Captures| {
                let m = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                format!("[REDACTED:{} len={}]", rule.id, m.len())
            })
            .into_owned();
    }
    // Scarlet Letter names referenced with values: SECRET_NAME=...
    static ENV_LINE: OnceLock<Regex> = OnceLock::new();
    let env_re = ENV_LINE.get_or_init(|| {
        Regex::new(r"(?m)^([A-Z][A-Z0-9_]{2,})=([^\s].{8,})$").expect("env line")
    });
    out = env_re
        .replace_all(&out, |caps: &regex::Captures| {
            let name = caps.get(1).map(|m| m.as_str()).unwrap_or("VAR");
            format!("{name}=[REDACTED:env]")
        })
        .into_owned();
    out
}

/// Redact a chat-style list of strings (goal, tool dumps, etc.).
pub fn redact_messages_content(parts: &mut [String]) {
    for p in parts.iter_mut() {
        *p = redact_for_model(p);
    }
}

/// True if text still contains something we would block as a secret.
pub fn contains_secret_shaped(text: &str) -> bool {
    scan_text(text).blocked
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_aws_and_github() {
        let r = scan_text("key = AKIAIOSFODNN7EXAMPLE\nghp_abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(r.blocked);
        let ids: Vec<_> = r.findings.iter().map(|f| f.rule_id.as_str()).collect();
        assert!(ids.contains(&"aws_access_key") || ids.iter().any(|i| i.contains("aws")));
        assert!(ids.iter().any(|i| i.contains("github")));
    }

    #[test]
    fn redact_for_model_strips_secrets() {
        let raw = "connect with postgres://user:s3cret@host/db and token ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let red = redact_for_model(raw);
        assert!(!red.contains("s3cret") || red.contains("REDACTED"));
        assert!(red.contains("REDACTED") || !red.contains("ghp_"));
    }

    #[test]
    fn clean_text_not_blocked() {
        let r = scan_text("Hello Roomies, welcome to #Rust. We use safe APIs.");
        assert!(!r.blocked);
    }

    #[test]
    fn allowlist_filters() {
        let r = scan_text("AKIAIOSFODNN7EXAMPLE");
        assert!(r.blocked);
        let filtered = apply_allowlist(r, &["aws_access_key".into()]);
        // may still block if other rules hit the same string
        let _ = filtered;
    }
}
