//! Agent harness — tool loop optimized for small local models (OLMo-3-7B).
//!
//! Design for limited fine-tuning data:
//! - Backend owns truth (tools execute real TT APIs)
//! - Rigid JSON protocol + tolerant parser
//! - Low temperature, short context, max_steps cap
//! - Write/dangerous tools pause for human approval

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};
use tt_core::domain::{AgentApproval, AgentRun, AgentRunStatus};
use tt_core::ids::{OrgId, PrincipalId, WorkspaceId};
use uuid::Uuid;

use crate::error::{AgentError, Result};
use crate::inference::{ChatMessage, InferenceClient};
use crate::prompt::{openai_tools, parse_tool_call, system_prompt};
use crate::store::AgentStore;
use crate::redact::{redact_chat_messages, redact_text, sanitize_tool_args_for_context};
use crate::tools::{BuiltinTools, ToolCall, ToolContext, ToolExecutor};

pub struct AgentHarness {
    store: Arc<dyn AgentStore>,
    inference: InferenceClient,
    tools: BuiltinTools,
}

impl AgentHarness {
    pub fn new(store: Arc<dyn AgentStore>, inference: InferenceClient, tools: BuiltinTools) -> Self {
        Self {
            store,
            inference,
            tools,
        }
    }

    pub fn inference_enabled(&self) -> bool {
        self.inference.is_enabled()
    }

    pub async fn start_run(
        &self,
        owner_id: PrincipalId,
        agent_id: PrincipalId,
        goal: String,
        workspace_id: Option<WorkspaceId>,
        org_id: Option<OrgId>,
        max_steps: Option<u32>,
    ) -> Result<AgentRun> {
        let goal = goal.trim().to_string();
        if goal.is_empty() {
            return Err(AgentError::Invalid("goal required".into()));
        }
        let now = Utc::now();
        let run = AgentRun {
            id: Uuid::now_v7(),
            agent_id,
            owner_id,
            workspace_id,
            org_id,
            status: AgentRunStatus::Queued,
            goal: goal.clone(),
            model: self.inference.model().to_string(),
            max_steps: max_steps.unwrap_or(12).clamp(1, 32),
            step_count: 0,
            result_summary: None,
            error: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        self.store.insert_run(&run).await?;
        Ok(run)
    }

    pub async fn get_run(&self, id: Uuid) -> Result<AgentRun> {
        self.store
            .get_run(id)
            .await?
            .ok_or(AgentError::NotFound)
    }

    pub async fn list_runs(&self, owner_id: PrincipalId, limit: u32) -> Result<Vec<AgentRun>> {
        self.store.list_runs_for_owner(owner_id, limit.min(50)).await
    }

    /// Drive the tool loop until finish, approval, failure, or max steps.
    pub async fn tick_run(&self, run_id: Uuid) -> Result<AgentRun> {
        let mut run = self.get_run(run_id).await?;
        if matches!(
            run.status,
            AgentRunStatus::Succeeded
                | AgentRunStatus::Failed
                | AgentRunStatus::Cancelled
                | AgentRunStatus::AwaitingApproval
        ) {
            return Ok(run);
        }

        run.status = AgentRunStatus::Running;
        run.updated_at = Utc::now();
        self.store.update_run(&run).await?;

        let specs = self.tools.specs();
        let sys = system_prompt(&specs);
        let tools_json = openai_tools(&specs);

        let safe_goal = redact_text(&run.goal);
        let mut messages = vec![
            ChatMessage {
                role: "system".into(),
                content: sys,
            },
            ChatMessage {
                role: "user".into(),
                content: format!(
                    "Workspace context: workspace_id={:?}\nGoal: {}\n\n(Note: secret values are redacted before model context. Use scarlet_probe / scarlet_list_secrets for metadata only.)",
                    run.workspace_id, safe_goal
                ),
            },
        ];

        let ctx = ToolContext {
            owner_id: run.owner_id,
            agent_id: run.agent_id,
            workspace_id: run.workspace_id,
            org_id: run.org_id,
            run_id: run.id,
        };

        while run.step_count < run.max_steps {
            // On-device redaction — never send secret-shaped text to the model
            redact_chat_messages(&mut messages);
            let raw = match self.inference.chat(&messages, &tools_json).await {
                Ok(r) => r,
                Err(e) => {
                    run.status = AgentRunStatus::Failed;
                    run.error = Some(e.to_string());
                    run.finished_at = Some(Utc::now());
                    run.updated_at = Utc::now();
                    self.store.update_run(&run).await?;
                    return Ok(run);
                }
            };

            let step_idx = run.step_count;
            let _ = self
                .store
                .insert_step(
                    Uuid::now_v7(),
                    run.id,
                    step_idx,
                    "assistant",
                    &raw,
                    None,
                    None,
                    None,
                )
                .await;

            let Some((name, args)) = parse_tool_call(&raw) else {
                // Nudge the model once
                messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: raw,
                });
                messages.push(ChatMessage {
                    role: "user".into(),
                    content: "Invalid response. Reply with ONLY JSON: {\"tool\":\"...\",\"arguments\":{...}}".into(),
                });
                run.step_count += 1;
                continue;
            };

            let policy = BuiltinTools::policy_for(&name);
            if policy.requires_approval {
                let approval = AgentApproval {
                    id: Uuid::now_v7(),
                    run_id: run.id,
                    tool_name: name.clone(),
                    tool_args: args.clone(),
                    status: "pending".into(),
                    requested_at: Utc::now(),
                    resolved_at: None,
                    resolved_by: None,
                };
                self.store.insert_approval(&approval).await?;
                run.status = AgentRunStatus::AwaitingApproval;
                run.metadata = serde_json::json!({ "pending_approval_id": approval.id });
                run.updated_at = Utc::now();
                self.store.update_run(&run).await?;
                info!(run = %run.id, tool = %name, "awaiting human approval");
                return Ok(run);
            }

            let call = ToolCall {
                name: name.clone(),
                arguments: args.clone(),
            };
            let output = match self.tools.execute(&call, &ctx).await {
                Ok(o) => o,
                Err(e) => {
                    let err_out = serde_json::json!({ "ok": false, "error": e.to_string() });
                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        content: redact_text(&raw),
                    });
                    messages.push(ChatMessage {
                        role: "user".into(),
                        content: format!(
                            "Tool error: {}. Try a different tool or finish.",
                            redact_text(&e.to_string())
                        ),
                    });
                    let _ = self
                        .store
                        .insert_step(
                            Uuid::now_v7(),
                            run.id,
                            step_idx + 1000,
                            "tool",
                            &e.to_string(),
                            Some(&name),
                            Some(args),
                            Some(err_out),
                        )
                        .await;
                    run.step_count += 1;
                    continue;
                }
            };

            let log_args = sanitize_tool_args_for_context(&name, &args);
            let log_data: serde_json::Value =
                serde_json::from_str(&redact_text(&output.data.to_string()))
                    .unwrap_or_else(|_| serde_json::json!({ "redacted": true }));
            let _ = self
                .store
                .insert_step(
                    Uuid::now_v7(),
                    run.id,
                    step_idx + 1000,
                    "tool",
                    &redact_text(&output.summary),
                    Some(&name),
                    Some(log_args),
                    Some(log_data),
                )
                .await;

            if name == "finish" {
                run.status = AgentRunStatus::Succeeded;
                run.result_summary = Some(output.summary.clone());
                run.finished_at = Some(Utc::now());
                run.step_count += 1;
                run.updated_at = Utc::now();
                self.store.update_run(&run).await?;
                return Ok(run);
            }

            let safe_args = sanitize_tool_args_for_context(&name, &args);
            let safe_data = redact_text(&output.data.to_string());
            let safe_summary = redact_text(&output.summary);
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: redact_text(&raw),
            });
            messages.push(ChatMessage {
                role: "user".into(),
                content: format!(
                    "Tool `{}` result: {}\nData: {}\nContinue or finish.",
                    name,
                    safe_summary,
                    safe_data
                ),
            });
            let _ = safe_args; // kept for future step logging without values
            run.step_count += 1;
            run.updated_at = Utc::now();
            self.store.update_run(&run).await?;
        }

        run.status = AgentRunStatus::Failed;
        run.error = Some("max steps exceeded".into());
        run.finished_at = Some(Utc::now());
        run.updated_at = Utc::now();
        self.store.update_run(&run).await?;
        warn!(run = %run.id, "max steps exceeded");
        Ok(run)
    }

    /// After human approves, execute the pending tool and resume.
    pub async fn resolve_approval(
        &self,
        approval_id: Uuid,
        owner_id: PrincipalId,
        approve: bool,
    ) -> Result<AgentRun> {
        let status = if approve { "approved" } else { "rejected" };
        let approval = self
            .store
            .resolve_approval(approval_id, status, owner_id)
            .await?
            .ok_or(AgentError::NotFound)?;
        let mut run = self.get_run(approval.run_id).await?;
        if run.owner_id != owner_id {
            return Err(AgentError::Forbidden("not run owner".into()));
        }

        if !approve {
            run.status = AgentRunStatus::Cancelled;
            run.error = Some(format!("user rejected tool {}", approval.tool_name));
            run.finished_at = Some(Utc::now());
            run.updated_at = Utc::now();
            self.store.update_run(&run).await?;
            return Ok(run);
        }

        let ctx = ToolContext {
            owner_id: run.owner_id,
            agent_id: run.agent_id,
            workspace_id: run.workspace_id,
            org_id: run.org_id,
            run_id: run.id,
        };
        let call = ToolCall {
            name: approval.tool_name.clone(),
            arguments: approval.tool_args.clone(),
        };
        match self.tools.execute(&call, &ctx).await {
            Ok(out) => {
                let _ = self
                    .store
                    .insert_step(
                        Uuid::now_v7(),
                        run.id,
                        run.step_count + 2000,
                        "tool",
                        &out.summary,
                        Some(&approval.tool_name),
                        Some(approval.tool_args.clone()),
                        Some(out.data),
                    )
                    .await;
                run.status = AgentRunStatus::Running;
                run.metadata = serde_json::json!({ "last_approved_tool": approval.tool_name });
                run.step_count += 1;
                run.updated_at = Utc::now();
                self.store.update_run(&run).await?;
                // Continue loop
                self.tick_run(run.id).await
            }
            Err(e) => {
                run.status = AgentRunStatus::Failed;
                run.error = Some(e.to_string());
                run.finished_at = Some(Utc::now());
                run.updated_at = Utc::now();
                self.store.update_run(&run).await?;
                Ok(run)
            }
        }
    }
}
