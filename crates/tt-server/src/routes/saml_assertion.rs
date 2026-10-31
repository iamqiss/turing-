//! Assertion-level SAML checks that run on XML which `saml_dsig` has already
//! proven to be covered by a valid signature.
//!
//! The functions here are deliberately dependency-light (std + chrono) and
//! pure, so they can be unit-tested without Postgres, Redis or OpenSSL.
//!
//! What this module enforces:
//! - no DTD / entity / comment / CDATA constructs (they make hand-rolled string
//!   parsing and real XML parsers disagree about what a value is)
//! - exactly one `Assertion` and exactly one `NameID` in the signed element
//! - `StatusCode` (if present) is Success
//! - `Conditions` has a `NotOnOrAfter`, and `NotBefore` / `NotOnOrAfter` hold
//! - the configured SP audience (if any) is listed
//! - a `SubjectConfirmationData` whose `Recipient` is our ACS URL,
//!   `InResponseTo` is the AuthnRequest we issued, and `NotOnOrAfter` holds

use chrono::{DateTime, Duration, Utc};

/// What this deployment expects the assertion to say.
pub struct Expect<'a> {
    pub now: DateTime<Utc>,
    /// Our ACS URL (`SubjectConfirmationData/@Recipient`).
    pub recipient: &'a str,
    /// ID of the AuthnRequest we sent (`SubjectConfirmationData/@InResponseTo`).
    pub in_response_to: &'a str,
    /// SP entity ID. `None` skips the audience check.
    pub audience: Option<&'a str>,
    /// Allowed clock skew between us and the IdP.
    pub skew_secs: i64,
}

/// Reject XML constructs we never need in a SAML response and that cause
/// parser differentials.
pub fn reject_dangerous_xml(xml: &str) -> Result<(), String> {
    let upper = xml.to_ascii_uppercase();
    for (needle, what) in [
        ("<!DOCTYPE", "DOCTYPE"),
        ("<!ENTITY", "ENTITY"),
        ("<!--", "comment"),
        ("<![CDATA[", "CDATA"),
    ] {
        if upper.contains(needle) {
            return Err(format!("SAML XML contains forbidden {what}"));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct StartTag {
    local: String,
    attrs: Vec<(String, String)>,
    /// Byte offset just after the closing `>` of the start tag.
    end: usize,
}

impl StartTag {
    fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| unescape(v))
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Parse every start tag (and self-closing tag) in `xml`.
/// Quoted attribute values may contain `>`.
fn start_tags(xml: &str) -> Result<Vec<StartTag>, String> {
    let b = xml.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let next = b.get(i + 1).copied().unwrap_or(b' ');
        if next == b'/' || next == b'!' || next == b'?' {
            i += 1;
            continue;
        }
        // name
        let name_start = i + 1;
        let mut j = name_start;
        while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' && b[j] != b'/' {
            j += 1;
        }
        if j == name_start {
            return Err("malformed XML: empty tag name".into());
        }
        let name = &xml[name_start..j];
        let local = name.rsplit(':').next().unwrap_or(name).to_string();

        // attributes
        let mut attrs = Vec::new();
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            match b.get(j) {
                None => return Err("malformed XML: unterminated tag".into()),
                Some(b'>') => {
                    j += 1;
                    break;
                }
                Some(b'/') => {
                    j += 1;
                    continue;
                }
                _ => {}
            }
            let k_start = j;
            while j < b.len()
                && !b[j].is_ascii_whitespace()
                && b[j] != b'='
                && b[j] != b'>'
                && b[j] != b'/'
            {
                j += 1;
            }
            let key = &xml[k_start..j];
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if b.get(j) != Some(&b'=') {
                return Err("malformed XML: attribute without value".into());
            }
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let q = match b.get(j) {
                Some(&q) if q == b'"' || q == b'\'' => q,
                _ => return Err("malformed XML: unquoted attribute".into()),
            };
            j += 1;
            let v_start = j;
            while j < b.len() && b[j] != q {
                j += 1;
            }
            if j >= b.len() {
                return Err("malformed XML: unterminated attribute".into());
            }
            attrs.push((key.to_string(), xml[v_start..j].to_string()));
            j += 1;
        }
        out.push(StartTag { local, attrs, end: j });
        i = j;
    }
    Ok(out)
}

fn parse_instant(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| format!("bad SAML instant {s:?}: {e}"))
}

/// `now` must be strictly before `not_on_or_after` (plus skew).
fn not_expired(now: DateTime<Utc>, not_on_or_after: &str, skew: Duration) -> Result<bool, String> {
    Ok(now < parse_instant(not_on_or_after)? + skew)
}

/// Validate a signature-verified element (a `Response` or an `Assertion`).
pub fn validate(element: &str, exp: &Expect) -> Result<(), String> {
    reject_dangerous_xml(element)?;
    let tags = start_tags(element)?;
    let skew = Duration::seconds(exp.skew_secs);

    let count = |local: &str| tags.iter().filter(|t| t.local == local).count();

    let n = count("Assertion");
    if n != 1 {
        return Err(format!("expected exactly one Assertion, found {n}"));
    }
    let n = count("NameID");
    if n != 1 {
        return Err(format!("expected exactly one NameID, found {n}"));
    }

    if let Some(sc) = tags.iter().find(|t| t.local == "StatusCode") {
        let v = sc.attr("Value").unwrap_or_default();
        if !v.ends_with(":status:Success") {
            return Err(format!("SAML status is not Success: {v}"));
        }
    }

    // Conditions
    let conds: Vec<_> = tags.iter().filter(|t| t.local == "Conditions").collect();
    if conds.len() != 1 {
        return Err(format!("expected exactly one Conditions, found {}", conds.len()));
    }
    let c = conds[0];
    let noa = c
        .attr("NotOnOrAfter")
        .ok_or("Conditions missing NotOnOrAfter")?;
    if !not_expired(exp.now, &noa, skew)? {
        return Err("assertion expired (Conditions/NotOnOrAfter)".into());
    }
    if let Some(nb) = c.attr("NotBefore") {
        if exp.now + skew < parse_instant(&nb)? {
            return Err("assertion not yet valid (Conditions/NotBefore)".into());
        }
    }

    // Audience
    if let Some(aud) = exp.audience {
        let mut found = false;
        for t in tags.iter().filter(|t| t.local == "Audience") {
            let rest = &element[t.end..];
            let text = rest.split('<').next().unwrap_or("");
            if unescape(text.trim()) == aud {
                found = true;
                break;
            }
        }
        if !found {
            return Err("assertion Audience does not include this service provider".into());
        }
    }

    // Subject confirmation: Recipient + InResponseTo + expiry.
    let mut confirmed = false;
    for t in tags.iter().filter(|t| t.local == "SubjectConfirmationData") {
        let recipient_ok = t.attr("Recipient").as_deref() == Some(exp.recipient);
        let irt_ok = t.attr("InResponseTo").as_deref() == Some(exp.in_response_to);
        let time_ok = match t.attr("NotOnOrAfter") {
            Some(v) => not_expired(exp.now, &v, skew)?,
            None => false,
        };
        if recipient_ok && irt_ok && time_ok {
            confirmed = true;
            break;
        }
    }
    if !confirmed {
        return Err(
            "no SubjectConfirmationData with matching Recipient, InResponseTo and valid NotOnOrAfter"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACS: &str = "http://127.0.0.1:8080/api/auth/sso/saml/acs";
    const REQ: &str = "_abc123";

    fn now() -> DateTime<Utc> {
        parse_instant("2026-10-30T12:00:00Z").unwrap()
    }

    fn exp<'a>() -> Expect<'a> {
        Expect {
            now: now(),
            recipient: ACS,
            in_response_to: REQ,
            audience: Some("urn:tt:sp"),
            skew_secs: 60,
        }
    }

    fn good() -> String {
        format!(
            r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_r1">
<samlp:Status><samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/></samlp:Status>
<saml:Assertion ID="_a1">
<saml:Subject><saml:NameID>alice@example.com</saml:NameID>
<saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
<saml:SubjectConfirmationData InResponseTo="{REQ}" Recipient="{ACS}" NotOnOrAfter="2026-10-30T12:05:00Z"/>
</saml:SubjectConfirmation></saml:Subject>
<saml:Conditions NotBefore="2026-10-30T11:59:00Z" NotOnOrAfter="2026-10-30T12:05:00Z">
<saml:AudienceRestriction><saml:Audience>urn:tt:sp</saml:Audience></saml:AudienceRestriction>
</saml:Conditions>
</saml:Assertion></samlp:Response>"#
        )
    }

    #[test]
    fn accepts_well_formed_assertion() {
        validate(&good(), &exp()).unwrap();
    }

    #[test]
    fn rejects_expired_conditions() {
        let mut e = exp();
        e.now = parse_instant("2026-10-30T12:10:00Z").unwrap();
        let err = validate(&good(), &e).unwrap_err();
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn allows_small_clock_skew_but_not_large() {
        let mut e = exp();
        // 30s past NotOnOrAfter, within 60s skew
        e.now = parse_instant("2026-10-30T12:05:30Z").unwrap();
        validate(&good(), &e).unwrap();
        e.now = parse_instant("2026-10-30T12:07:00Z").unwrap();
        assert!(validate(&good(), &e).is_err());
    }

    #[test]
    fn rejects_not_yet_valid() {
        let mut e = exp();
        e.now = parse_instant("2026-10-30T11:50:00Z").unwrap();
        let err = validate(&good(), &e).unwrap_err();
        assert!(err.contains("not yet valid"), "{err}");
    }

    #[test]
    fn rejects_missing_not_on_or_after() {
        let xml = good().replace(
            r#" NotOnOrAfter="2026-10-30T12:05:00Z">
<saml:AudienceRestriction>"#,
            ">\n<saml:AudienceRestriction>",
        );
        let err = validate(&xml, &exp()).unwrap_err();
        assert!(err.contains("NotOnOrAfter"), "{err}");
    }

    #[test]
    fn rejects_wrong_in_response_to() {
        let mut e = exp();
        e.in_response_to = "_someone_elses_request";
        assert!(validate(&good(), &e).is_err());
    }

    #[test]
    fn rejects_wrong_recipient() {
        let mut e = exp();
        e.recipient = "https://evil.example/acs";
        assert!(validate(&good(), &e).is_err());
    }

    #[test]
    fn rejects_wrong_audience_and_skips_when_unset() {
        let mut e = exp();
        e.audience = Some("urn:other:sp");
        assert!(validate(&good(), &e).is_err());
        e.audience = None;
        validate(&good(), &e).unwrap();
    }

    #[test]
    fn rejects_two_assertions() {
        let xml = good().replace(
            "</samlp:Response>",
            r#"<saml:Assertion ID="_evil"><saml:Subject><saml:NameID>admin@example.com</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>"#,
        );
        let err = validate(&xml, &exp()).unwrap_err();
        assert!(err.contains("exactly one Assertion"), "{err}");
    }

    #[test]
    fn rejects_two_name_ids() {
        let xml = good().replace(
            "</saml:Subject>",
            "<saml:NameID>admin@example.com</saml:NameID></saml:Subject>",
        );
        let err = validate(&xml, &exp()).unwrap_err();
        assert!(err.contains("NameID"), "{err}");
    }

    #[test]
    fn rejects_non_success_status() {
        let xml = good().replace("status:Success", "status:Requester");
        let err = validate(&xml, &exp()).unwrap_err();
        assert!(err.contains("not Success"), "{err}");
    }

    #[test]
    fn rejects_comment_inside_name_id() {
        // classic parser-differential: alice@example.com<!---->.evil.com
        let xml = good().replace(
            "alice@example.com",
            "admin@victim.com<!---->.attacker.com",
        );
        let err = validate(&xml, &exp()).unwrap_err();
        assert!(err.contains("comment"), "{err}");
    }

    #[test]
    fn rejects_doctype_and_entities() {
        let xml = format!("<!DOCTYPE foo [<!ENTITY x \"y\">]>{}", good());
        assert!(reject_dangerous_xml(&xml).is_err());
        assert!(reject_dangerous_xml("<a><![CDATA[x]]></a>").is_err());
        assert!(reject_dangerous_xml(&good()).is_ok());
    }

    #[test]
    fn attribute_values_may_contain_gt_and_amp() {
        let tags = start_tags(r#"<a x="1>2" y='a&amp;b'><b/></a>"#).unwrap();
        assert_eq!(tags[0].local, "a");
        assert_eq!(tags[0].attr("x").as_deref(), Some("1>2"));
        assert_eq!(tags[0].attr("y").as_deref(), Some("a&b"));
        assert_eq!(tags[1].local, "b");
    }

    #[test]
    fn malformed_xml_is_an_error_not_a_panic() {
        assert!(start_tags("<a x=1>").is_err());
        assert!(start_tags("<a x=\"1").is_err());
        assert!(start_tags("<a").is_err());
        // multibyte text must not panic
        assert!(start_tags("<a>é日本</a>").is_ok());
    }
}
