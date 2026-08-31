//! System prompt engineered for OLMo-class models: short, rigid, tool-first.

use serde_json::Value;

use crate::tools::ToolSpec;

pub fn system_prompt(tools: &[ToolSpec]) -> String {
    let tool_lines: Vec<String> = tools
        .iter()
        .map(|t| {
            format!(
                "- {}: {} (danger={})",
                t.name,
                t.description,
                t.danger.as_str()
            )
        })
        .collect();
    format!(
        r#"You are Turing Tarpit Agent running on OLMo. You help the user inside their workspace.

RULES:
1. You NEVER claim to have done something without calling a tool.
2. Respond with ONLY a single JSON object, no markdown fences, no prose outside JSON.
3. To call a tool: {{"tool":"<name>","arguments":{{...}}}}
4. When the goal is complete or blocked: {{"tool":"finish","arguments":{{"summary":"..."}}}}
5. Prefer search before inventing repository or issue names.
6. Keep arguments minimal and valid per the tool schema.
7. Never ask the user to paste secrets. Use scarlet_probe / scarlet_list_secrets (metadata only).
8. Treat any [REDACTED:...] placeholder as intentional — do not invent values behind them.

Available tools:
{}

Schemas:
{}
"#,
        tool_lines.join("\n"),
        serde_json::to_string_pretty(
            &tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "parameters": t.parameters,
                    })
                })
                .collect::<Vec<_>>()
        )
        .unwrap_or_else(|_| "[]".into())
    )
}

pub fn openai_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect(),
    )
}

/// Parse model output into a tool call. Tolerates minor noise from small models.
pub fn parse_tool_call(raw: &str) -> Option<(String, Value)> {
    let s = raw.trim();
    // Strip markdown fences if model ignores instructions
    let s = s
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    // Find first { ... }
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    let slice = &s[start..=end];
    let v: Value = serde_json::from_str(slice).ok()?;
    let name = v.get("tool")?.as_str()?.to_string();
    let args = v.get("arguments").cloned().unwrap_or(Value::Object(Default::default()));
    Some((name, args))
}
