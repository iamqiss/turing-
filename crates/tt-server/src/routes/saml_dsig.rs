//! SAML XML-DSig verification against IdP X.509.
//!
//! - RSA-SHA256 / RSA-SHA1 over exclusive-C14N `SignedInfo`
//! - Reference digests with enveloped-signature transform + exclusive C14N
//! - IdP cert required (PEM or base64 DER)

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::Padding;
use openssl::sign::Verifier;
use openssl::x509::X509;
use sha1::{Digest as _, Sha1};
use sha2::{Digest as Digest2, Sha256};

/// Verify the XML-DSig on a SAML response and return the XML of every element
/// whose digest was verified (signature-stripped, i.e. exactly the bytes the
/// digest covers). Callers MUST read claims only from the returned strings,
/// never from the original document.
pub fn verify_saml_xml_dsig(xml: &str, idp_cert_pem_or_der: &str) -> Result<Vec<String>, String> {
    super::saml_assertion::reject_dangerous_xml(xml)?;
    let cert = parse_x509(idp_cert_pem_or_der)?;
    let pkey = cert
        .public_key()
        .map_err(|e| format!("cert public key: {e}"))?;

    let signed_info = extract_element(xml, "SignedInfo")
        .ok_or_else(|| "SAML response missing SignedInfo".to_string())?;
    let signature_b64 = extract_element_text(xml, "SignatureValue")
        .ok_or_else(|| "SAML response missing SignatureValue".to_string())?;

    let sig = decode_b64(&signature_b64)?;
    let prefix_list = inclusive_ns_prefix_list(xml);
    let c14n_si = exclusive_c14n(&signed_info, &prefix_list)?;

    let alg = signature_method_algorithm(&signed_info)?;
    verify_signature(&pkey, alg, &c14n_si, &sig)?;

    verify_references(&signed_info, xml, &prefix_list)
}

fn parse_x509(material: &str) -> Result<X509, String> {
    let trimmed = material.trim();
    if trimmed.contains("BEGIN CERTIFICATE") {
        return X509::from_pem(trimmed.as_bytes()).map_err(|e| format!("x509 pem: {e}"));
    }
    let der = decode_b64(trimmed)?;
    X509::from_der(&der).map_err(|e| format!("x509 der: {e}"))
}

#[derive(Clone, Copy)]
enum SigAlg {
    RsaSha256,
    RsaSha1,
}

fn signature_method_algorithm(signed_info: &str) -> Result<SigAlg, String> {
    let el = extract_element(signed_info, "SignatureMethod")
        .ok_or_else(|| "SignedInfo missing SignatureMethod".to_string())?;
    match attr_value(&el, "Algorithm").as_deref() {
        Some("http://www.w3.org/2001/04/xmldsig-more#rsa-sha256") => Ok(SigAlg::RsaSha256),
        Some("http://www.w3.org/2000/09/xmldsig#rsa-sha1") => Ok(SigAlg::RsaSha1),
        other => Err(format!("unsupported SignatureMethod: {other:?}")),
    }
}

fn verify_signature(
    pkey: &PKey<openssl::pkey::Public>,
    alg: SigAlg,
    data: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let md = match alg {
        SigAlg::RsaSha256 => MessageDigest::sha256(),
        SigAlg::RsaSha1 => MessageDigest::sha1(),
    };
    let mut verifier = Verifier::new(md, pkey).map_err(|e| format!("verifier: {e}"))?;
    verifier
        .update(data)
        .map_err(|e| format!("verifier update: {e}"))?;
    let ok = verifier
        .verify(signature)
        .map_err(|e| format!("verify: {e}"))?;
    if ok {
        return Ok(());
    }
    if let Ok(rsa) = pkey.rsa() {
        if try_rsa_pkcs1_verify(&rsa, alg, data, signature) {
            return Ok(());
        }
    }
    Err("XML-DSig signature verification failed".into())
}

fn try_rsa_pkcs1_verify(
    rsa: &openssl::rsa::Rsa<openssl::pkey::Public>,
    alg: SigAlg,
    data: &[u8],
    signature: &[u8],
) -> bool {
    let mut dec = vec![0u8; rsa.size() as usize];
    let n = match rsa.public_decrypt(signature, &mut dec, Padding::PKCS1) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let dec = &dec[..n];
    let digest: Vec<u8> = match alg {
        SigAlg::RsaSha256 => Sha256::digest(data).to_vec(),
        SigAlg::RsaSha1 => Sha1::digest(data).to_vec(),
    };
    let prefix: &[u8] = match alg {
        SigAlg::RsaSha256 => &[
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x01, 0x05, 0x00, 0x04, 0x20,
        ],
        SigAlg::RsaSha1 => &[
            0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
            0x14,
        ],
    };
    dec.len() == prefix.len() + digest.len()
        && dec.starts_with(prefix)
        && &dec[prefix.len()..] == digest.as_slice()
}

fn inclusive_ns_prefix_list(xml: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    // InclusiveNamespaces PrefixList="xs #default" etc.
    if let Some(idx) = xml.find("PrefixList=\"") {
        let start = idx + "PrefixList=\"".len();
        if let Some(end) = xml[start..].find('"') {
            for p in xml[start..start + end].split_whitespace() {
                if p == "#default" {
                    set.insert("".into());
                } else {
                    set.insert(p.to_string());
                }
            }
        }
    }
    set
}

/// Every `Reference` element inside `signed_info` (and only there).
fn reference_blocks(signed_info: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = signed_info[pos..].find('<') {
        let start = pos + rel;
        let rest = &signed_info[start + 1..];
        if rest.starts_with('/') || rest.starts_with('!') || rest.starts_with('?') {
            pos = start + 1;
            continue;
        }
        let name_len = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let local = rest[..name_len].rsplit(':').next().unwrap_or("");
        if local == "Reference" {
            let block = extract_element_from_offset(signed_info, start, "Reference")
                .ok_or_else(|| "malformed Reference element".to_string())?;
            pos = start + block.len();
            out.push(block);
        } else {
            pos = start + 1;
        }
    }
    Ok(out)
}

#[derive(Clone, Copy)]
enum DigestAlg {
    Sha1,
    Sha256,
}

fn digest_algorithm(block: &str) -> Result<DigestAlg, String> {
    let el = extract_element(block, "DigestMethod")
        .ok_or_else(|| "Reference missing DigestMethod".to_string())?;
    match attr_value(&el, "Algorithm").as_deref() {
        Some("http://www.w3.org/2001/04/xmlenc#sha256") => Ok(DigestAlg::Sha256),
        Some("http://www.w3.org/2000/09/xmldsig#sha1") => Ok(DigestAlg::Sha1),
        other => Err(format!("unsupported DigestMethod: {other:?}")),
    }
}

/// Verify every `Reference` of the (already signature-verified) `signed_info`.
///
/// Fails closed: at least one reference is required, each must be a same-document
/// `#id` reference with a digest, the referenced element must contain this very
/// Signature (enveloped), and the digest must match. Returns the digested XML
/// of each referenced element.
fn verify_references(
    signed_info: &str,
    xml: &str,
    prefix_list: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let blocks = reference_blocks(signed_info)?;
    if blocks.is_empty() {
        return Err("SignedInfo contains no Reference".into());
    }
    let mut verified = Vec::new();
    for block in blocks {
        let uri = attr_value(&block, "URI").ok_or_else(|| "Reference missing URI".to_string())?;
        let id = uri
            .strip_prefix('#')
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("unsupported Reference URI: {uri:?}"))?;

        let digest_b64 = extract_element_text(&block, "DigestValue")
            .filter(|d| !d.is_empty())
            .ok_or_else(|| "Reference missing DigestValue".to_string())?;
        let alg = digest_algorithm(&block)?;

        let element = find_element_by_id(xml, id)
            .ok_or_else(|| format!("signed reference id not found: {id}"))?;
        if !element.contains(signed_info) {
            return Err(format!(
                "Signature is not enveloped in referenced element #{id}"
            ));
        }
        let without_sig = strip_enveloped_signatures(&element);
        let c14n = exclusive_c14n(&without_sig, prefix_list)?;

        let expected = decode_b64(&digest_b64)?;
        let actual: Vec<u8> = match alg {
            DigestAlg::Sha1 => Sha1::digest(&c14n).to_vec(),
            DigestAlg::Sha256 => Sha256::digest(&c14n).to_vec(),
        };
        if expected != actual {
            return Err(format!("reference digest mismatch for URI=#{id}"));
        }
        verified.push(without_sig);
    }
    Ok(verified)
}

// ----- exclusive C14N (subset of W3C exclusive XML canonicalization) -----

#[derive(Debug, Clone)]
struct XmlNode {
    /// prefix:local or local
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<XmlChild>,
}

#[derive(Debug, Clone)]
enum XmlChild {
    Text(String),
    Elem(XmlNode),
}

fn exclusive_c14n(fragment: &str, inclusive_prefixes: &BTreeSet<String>) -> Result<Vec<u8>, String> {
    let root = parse_fragment(fragment)?;
    let mut out = String::new();
    // ancestor ns map empty at root of fragment
    write_exc_c14n(&root, &BTreeMap::new(), inclusive_prefixes, &mut out);
    Ok(out.into_bytes())
}

fn write_exc_c14n(
    node: &XmlNode,
    parent_ns: &BTreeMap<String, String>,
    inclusive: &BTreeSet<String>,
    out: &mut String,
) {
    let (prefix, local) = split_name(&node.name);

    // namespaces declared on this element
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in &node.attrs {
        if k == "xmlns" {
            declared.insert(String::new(), v.clone());
        } else if let Some(p) = k.strip_prefix("xmlns:") {
            declared.insert(p.to_string(), v.clone());
        }
    }

    // visibly utilized prefixes: element prefix + attr prefixes
    let mut utilized: BTreeSet<String> = BTreeSet::new();
    utilized.insert(prefix.clone());
    for (k, _) in &node.attrs {
        if k == "xmlns" || k.starts_with("xmlns:") {
            continue;
        }
        let (ap, _) = split_name(k);
        if !ap.is_empty() {
            utilized.insert(ap);
        }
    }
    for p in inclusive {
        utilized.insert(p.clone());
    }

    // ns to emit: utilized, different from parent, known in declared∪parent
    let mut ns_ctx = parent_ns.clone();
    for (p, u) in &declared {
        ns_ctx.insert(p.clone(), u.clone());
    }

    let mut emit_ns: BTreeMap<String, String> = BTreeMap::new();
    for p in &utilized {
        if let Some(uri) = ns_ctx.get(p) {
            if parent_ns.get(p) != Some(uri) {
                emit_ns.insert(p.clone(), uri.clone());
            }
        }
    }

    out.push('<');
    if prefix.is_empty() {
        out.push_str(&local);
    } else {
        out.push_str(&prefix);
        out.push(':');
        out.push_str(&local);
    }

    // namespace nodes lexicographically by prefix
    for (p, uri) in &emit_ns {
        out.push(' ');
        if p.is_empty() {
            out.push_str("xmlns=\"");
        } else {
            out.push_str("xmlns:");
            out.push_str(p);
            out.push_str("=\"");
        }
        out.push_str(&escape_attr(uri));
        out.push('"');
    }

    // attributes: sort by namespace URI then local name (C14N)
    let mut attrs: Vec<(String, String, String, String)> = Vec::new();
    // tuple: (ns_uri, local, qname, value)
    for (k, v) in &node.attrs {
        if k == "xmlns" || k.starts_with("xmlns:") {
            continue;
        }
        let (ap, alocal) = split_name(k);
        let ns_uri = if ap.is_empty() {
            String::new()
        } else {
            ns_ctx.get(&ap).cloned().unwrap_or_default()
        };
        attrs.push((ns_uri, alocal, k.clone(), v.clone()));
    }
    attrs.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

    for (_, _, qname, value) in attrs {
        out.push(' ');
        out.push_str(&qname);
        out.push_str("=\"");
        out.push_str(&escape_attr(&value));
        out.push('"');
    }
    out.push('>');

    for child in &node.children {
        match child {
            XmlChild::Text(t) => out.push_str(&escape_text(t)),
            XmlChild::Elem(e) => write_exc_c14n(e, &ns_ctx, inclusive, out),
        }
    }

    out.push_str("</");
    if prefix.is_empty() {
        out.push_str(&local);
    } else {
        out.push_str(&prefix);
        out.push(':');
        out.push_str(&local);
    }
    out.push('>');
}

fn split_name(name: &str) -> (String, String) {
    if let Some((p, l)) = name.split_once(':') {
        (p.to_string(), l.to_string())
    } else {
        (String::new(), name.to_string())
    }
}

fn escape_attr(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '"' => o.push_str("&quot;"),
            '\r' => o.push_str("&#xD;"),
            '\n' => o.push_str("&#xA;"),
            '\t' => o.push_str("&#x9;"),
            _ => o.push(c),
        }
    }
    o
}

fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\r' => o.push_str("&#xD;"),
            _ => o.push(c),
        }
    }
    o
}

/// Minimal XML fragment parser (elements, attrs, text). Enough for SAML assertions.
fn parse_fragment(s: &str) -> Result<XmlNode, String> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if !s.starts_with('<') {
        return Err("fragment must start with element".into());
    }
    let (node, _) = parse_element(bytes, 0)?;
    Ok(node)
}

fn parse_element(bytes: &[u8], mut i: usize) -> Result<(XmlNode, usize), String> {
    if i >= bytes.len() || bytes[i] != b'<' {
        return Err("expected '<'".into());
    }
    i += 1;
    if i < bytes.len() && bytes[i] == b'/' {
        return Err("unexpected close".into());
    }
    let name_start = i;
    while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
        i += 1;
    }
    let name = std::str::from_utf8(&bytes[name_start..i])
        .map_err(|_| "utf8")?
        .to_string();

    let mut attrs = Vec::new();
    loop {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i >= bytes.len() {
            return Err("unclosed tag".into());
        }
        if bytes[i] == b'/' {
            i += 1;
            if i < bytes.len() && bytes[i] == b'>' {
                i += 1;
                return Ok((
                    XmlNode {
                        name,
                        attrs,
                        children: vec![],
                    },
                    i,
                ));
            }
            return Err("bad self-close".into());
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        // attr
        let an_start = i;
        while i < bytes.len() && bytes[i] != b'=' {
            i += 1;
        }
        let aname = std::str::from_utf8(&bytes[an_start..i])
            .map_err(|_| "utf8")?
            .trim()
            .to_string();
        i += 1; // =
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            return Err("attr quotes".into());
        }
        let q = bytes[i];
        i += 1;
        let av_start = i;
        while i < bytes.len() && bytes[i] != q {
            i += 1;
        }
        let aval = std::str::from_utf8(&bytes[av_start..i])
            .map_err(|_| "utf8")?
            .to_string();
        i += 1; // close quote
        attrs.push((aname, aval));
    }

    let mut children = Vec::new();
    loop {
        if i + 1 < bytes.len() && bytes[i] == b'<' && bytes[i + 1] == b'/' {
            // closing tag
            i += 2;
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i += 1;
            return Ok((
                XmlNode {
                    name,
                    attrs,
                    children,
                },
                i,
            ));
        }
        if i < bytes.len() && bytes[i] == b'<' {
            let (child, ni) = parse_element(bytes, i)?;
            children.push(XmlChild::Elem(child));
            i = ni;
        } else {
            let t_start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            if i > t_start {
                let text = std::str::from_utf8(&bytes[t_start..i])
                    .map_err(|_| "utf8")?
                    .to_string();
                if !text.is_empty() {
                    children.push(XmlChild::Text(text));
                }
            } else {
                return Err("parse stuck".into());
            }
        }
    }
}

// ----- extract helpers -----

fn extract_element(xml: &str, local: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(rel) = xml[search_from..].find(local) {
        let abs = search_from + rel;
        if abs > 0 {
            let prev = xml.as_bytes()[abs - 1];
            if prev != b'<' && prev != b':' {
                search_from = abs + local.len();
                continue;
            }
        }
        let start = xml[..abs].rfind('<')?;
        return extract_element_from_offset(xml, start, local);
    }
    None
}

fn extract_element_from_offset(xml: &str, start: usize, local_name: &str) -> Option<String> {
    let slice = &xml[start..];
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < slice.len() {
        if slice.as_bytes()[i] == b'<' {
            let closing = slice[i..].starts_with("</");
            let rest = if closing {
                &slice[i + 2..]
            } else {
                &slice[i + 1..]
            };
            let name_len = rest
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(0);
            let name = &rest[..name_len];
            let local = name.rsplit(':').next().unwrap_or(name);
            let gt = rest.find('>')?;
            let self_close = rest[..gt].ends_with('/');
            if local == local_name {
                if closing {
                    depth -= 1;
                    if depth == 0 {
                        let end = i + 2 + gt + 1;
                        return Some(slice[..end.min(slice.len())].to_string());
                    }
                } else if self_close {
                    if depth == 0 {
                        return Some(slice[..i + 1 + gt + 1].to_string());
                    }
                } else {
                    depth += 1;
                }
            }
            i += 1 + gt + 1;
            continue;
        }
        i += 1;
    }
    None
}

fn extract_element_text(xml: &str, local: &str) -> Option<String> {
    let el = extract_element(xml, local)?;
    if let Some(gt) = el.find('>') {
        let rest = &el[gt + 1..];
        if let Some(end) = rest.find("</") {
            return Some(rest[..end].trim().to_string());
        }
    }
    None
}

fn attr_value(block: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let idx = block.find(&key)?;
    let start = idx + key.len();
    let rest = &block[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn find_element_by_id(xml: &str, id: &str) -> Option<String> {
    for key in ["ID=\"", "Id=\"", "id=\"", "wsu:Id=\""] {
        let pattern = format!("{key}{id}\"");
        if let Some(pos) = xml.find(&pattern) {
            let head = xml[..pos].rfind('<')?;
            let name_end = xml[head + 1..]
                .find(|c: char| c.is_whitespace() || c == '>')
                .map(|i| head + 1 + i)?;
            let tag = xml[head + 1..name_end].trim();
            let local = tag.rsplit(':').next().unwrap_or(tag);
            return extract_element_from_offset(xml, head, local);
        }
    }
    None
}

fn strip_enveloped_signatures(xml: &str) -> String {
    let mut out = xml.to_string();
    loop {
        let Some(sig_rel) = out.find("Signature") else {
            break;
        };
        // ensure tag
        let start = match out[..sig_rel].rfind('<') {
            Some(s) => s,
            None => break,
        };
        let head = &out[start..sig_rel + 20.min(out.len() - sig_rel)];
        if !(head.contains("<Signature") || head.contains(":Signature")) {
            // false positive in text
            if let Some(n) = out[sig_rel + 1..].find("Signature") {
                // skip this occurrence by truncating search — replace with space trick
                out.replace_range(sig_rel..sig_rel + 1, " ");
                continue;
            }
            break;
        }
        let local = "Signature";
        if let Some(el) = extract_element_from_offset(&out, start, local) {
            if let Some(pos) = out.find(&el) {
                out.replace_range(pos..pos + el.len(), "");
                continue;
            }
        }
        break;
    }
    out
}

fn decode_b64(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&clean))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exc_c14n_simple() {
        let xml = r#"<foo xmlns="urn:test"><bar a="1">x</bar></foo>"#;
        let out = exclusive_c14n(xml, &BTreeSet::new()).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("xmlns=\"urn:test\""));
        assert!(s.contains("<bar"));
    }

    const SI_ONE_REF: &str = r##"<ds:SignedInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:SignatureMethod Algorithm="http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"/><ds:Reference URI="#a1"><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue>AAAA</ds:DigestValue></ds:Reference></ds:SignedInfo>"##;

    fn no_prefixes() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn reference_blocks_finds_only_references_in_signed_info() {
        let blocks = reference_blocks(SI_ONE_REF).unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].contains("URI=\"#a1\""));
    }

    #[test]
    fn signature_method_is_read_from_signed_info_not_document_text() {
        let alg = signature_method_algorithm(SI_ONE_REF).unwrap();
        assert!(matches!(alg, SigAlg::RsaSha256));
        // attacker text elsewhere in the document cannot downgrade the algorithm
        let si = SI_ONE_REF.replace(
            "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256",
            "http://www.w3.org/2000/09/xmldsig#rsa-sha1",
        );
        assert!(matches!(signature_method_algorithm(&si).unwrap(), SigAlg::RsaSha1));
        let bad = SI_ONE_REF.replace("rsa-sha256", "hmac-sha256");
        assert!(signature_method_algorithm(&bad).is_err());
    }

    #[test]
    fn no_reference_is_an_error_not_a_pass() {
        let si = r#"<ds:SignedInfo xmlns:ds="x"><ds:SignatureMethod Algorithm="y"/></ds:SignedInfo>"#;
        let err = verify_references(si, si, &no_prefixes()).unwrap_err();
        assert!(err.contains("no Reference"), "{err}");
    }

    #[test]
    fn whole_document_and_external_uris_are_rejected() {
        for uri in ["", "http://evil.example/x"] {
            let si = SI_ONE_REF.replace("#a1", uri);
            let err = verify_references(&si, &si, &no_prefixes()).unwrap_err();
            assert!(err.contains("unsupported Reference URI"), "{uri}: {err}");
        }
    }

    #[test]
    fn missing_digest_value_is_an_error_not_skipped() {
        let si = SI_ONE_REF.replace("<ds:DigestValue>AAAA</ds:DigestValue>", "");
        let err = verify_references(&si, &si, &no_prefixes()).unwrap_err();
        assert!(err.contains("DigestValue"), "{err}");
    }

    #[test]
    fn unknown_digest_method_is_rejected() {
        let si = SI_ONE_REF.replace("xmlenc#sha256", "xmlenc#sha999");
        let err = verify_references(&si, &si, &no_prefixes()).unwrap_err();
        assert!(err.contains("DigestMethod"), "{err}");
    }

    #[test]
    fn referenced_id_must_exist() {
        let xml = format!(r#"<r><x ID="other"/>{SI_ONE_REF}</r>"#);
        let err = verify_references(SI_ONE_REF, &xml, &no_prefixes()).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn referenced_element_must_envelop_the_signature() {
        // The element with ID a1 exists but the SignedInfo lives outside it:
        // a signature lifted from elsewhere must not "sign" this element.
        let xml = format!(r#"<r><saml:Assertion ID="a1"><x/></saml:Assertion>{SI_ONE_REF}</r>"#);
        let err = verify_references(SI_ONE_REF, &xml, &no_prefixes()).unwrap_err();
        assert!(err.contains("not enveloped"), "{err}");
    }

    #[test]
    fn duplicate_id_decoy_before_real_element_is_rejected_not_trusted() {
        let xml = format!(
            r#"<r><saml:Assertion ID="a1"><NameID>admin@victim.com</NameID></saml:Assertion><saml:Assertion ID="a1">{SI_ONE_REF}</saml:Assertion></r>"#
        );
        // first element with ID=a1 is the decoy; it does not contain the signature
        assert!(verify_references(SI_ONE_REF, &xml, &no_prefixes()).is_err());
    }

    #[test]
    fn non_ascii_text_does_not_panic_element_extraction() {
        let xml = "<a>é日本語<b>ü</b></a>";
        assert!(extract_element(xml, "b").is_some());
        assert!(extract_element_text(xml, "b").is_some());
    }
}
