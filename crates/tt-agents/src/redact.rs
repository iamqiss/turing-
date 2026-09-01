//! On-device redaction before anything reaches Pearl's model context.
//!
//! Anything in the model context is a potential leak. We strip secret-shaped
//! strings from goals, tool results, and assistant echoes before inference.

use tt_secrets::redact_for_model;

use crate::inference::ChatMessage;

/// Redact all message contents in-place before sending to the inference server.
pub fn redact_chat_messages(messages: &mut [ChatMessage]) {
    for m in messages.iter_mut() {
        m.content = redact_for_model(&m.content);
    }
}

/// Redact a single string (goal, tool JSON, user nudge).
pub fn redact_text(s: &str) -> String {
    redact_for_model(s)
}

/// Strip value fields from tool args that must never be re-fed to the model
/// (e.g. scarlet_put_secret after approval logging).
pub fn sanitize_tool_args_for_context(tool_name: &str, args: &serde_json::Value) -> serde_json::Value {
    let mut v = args.clone();
    if tool_name == "scarlet_put_secret" {
        if let Some(obj) = v.as_object_mut() {
            if obj.contains_key("value") {
                obj.insert(
                    "value".into(),
                    serde_json::json!("[REDACTED:scarlet_value]"),
                );
            }
        }
    }
    // Generic: redact any string values that look secret-shaped
    sanitize_value(&mut v);
    v
}

fn sanitize_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) => {
            *s = redact_for_model(s);
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                sanitize_value(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if matches!(
                    k.as_str(),
                    "value" | "password" | "secret" | "token" | "api_key" | "content"
                ) {
                    if let serde_json::Value::String(s) = val {
                        if s.len() > 8 {
                            *s = format!("[REDACTED:field:{k}]");
                            continue;
                        }
                    }
                }
                sanitize_value(val);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::ChatMessage;

    #[test]
    fn redact_chat_messages_mutates() {
        let mut msgs = vec![ChatMessage {
            role: "user".into(),
            content: "token ghp_abcdefghijklmnopqrstuvwxyz0123456789".into(),
        }];
        redact_chat_messages(&mut msgs);
        assert!(msgs[0].content.contains("REDACTED") || !msgs[0].content.contains("ghp_"));
    }

    #[test]
    fn sanitize_scarlet_put_value() {
        let args = serde_json::json!({"name": "DB", "value": "super-secret-password-value"});
        let s = sanitize_tool_args_for_context("scarlet_put_secret", &args);
        assert_ne!(s["value"], "super-secret-password-value");
    }
}
