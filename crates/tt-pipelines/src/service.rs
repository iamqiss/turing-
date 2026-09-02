//! Turing Tasks engine — register pipelines, dispatch runs, bill minutes.

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};
use tt_billing::Meter;
use tt_billing::BillingService;
use tt_core::domain::*;
use tt_core::ids::*;
use uuid::Uuid;

use crate::error::{PipelineError, Result};
use crate::store::TaskStore;
use crate::yaml::{matches_trigger, parse_pipeline_yaml, PipelineFile};

/// Minutes included per plan before overage.
pub fn included_minutes(plan: &str) -> u32 {
    match plan {
        "free" => 500,
        "team" => 3_000,
        "business" => 15_000,
        "enterprise" => 100_000,
        _ => 500,
    }
}

pub struct TaskService {
    store: Arc<dyn TaskStore>,
    billing: Arc<BillingService>,
}

impl TaskService {
    pub fn new(store: Arc<dyn TaskStore>, billing: Arc<BillingService>) -> Self {
        Self { store, billing }
    }

    pub async fn register_pipeline(
        &self,
        repository_id: RepositoryId,
        org_id: Option<OrgId>,
        name: String,
        path: String,
        yaml: String,
    ) -> Result<TaskPipeline> {
        let _ = parse_pipeline_yaml(&yaml)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(PipelineError::Invalid("pipeline name required".into()));
        }
        let p = TaskPipeline {
            id: Uuid::now_v7(),
            repository_id,
            org_id,
            name: name.clone(),
            path,
            definition_yaml: yaml,
            enabled: true,
            updated_at: Utc::now(),
        };
        self.store.upsert_pipeline(&p).await?;
        self.store
            .list_pipelines(repository_id)
            .await?
            .into_iter()
            .find(|x| x.name == name)
            .ok_or(PipelineError::NotFound)
    }

    pub async fn list_pipelines(&self, repository_id: RepositoryId) -> Result<Vec<TaskPipeline>> {
        self.store.list_pipelines(repository_id).await
    }

    pub async fn list_runs(
        &self,
        repository_id: RepositoryId,
        limit: i64,
    ) -> Result<Vec<TaskRun>> {
        self.store.list_runs(repository_id, limit).await
    }

    pub async fn get_run(&self, id: Uuid) -> Result<(TaskRun, Vec<TaskJob>)> {
        let run = self.store.get_run(id).await?.ok_or(PipelineError::NotFound)?;
        let jobs = self.store.list_jobs(id).await?;
        Ok((run, jobs))
    }

    pub async fn dispatch(
        &self,
        repository_id: RepositoryId,
        org_id: Option<OrgId>,
        pipeline_name: Option<&str>,
        trigger: TaskTrigger,
        git_ref: Option<String>,
        git_sha: Option<String>,
        triggered_by: Option<PrincipalId>,
        yaml_override: Option<String>,
    ) -> Result<TaskRun> {
        let branch = git_ref
            .as_deref()
            .map(|r| r.strip_prefix("refs/heads/").unwrap_or(r));

        let (pipeline_id, pipeline_name, yaml) = if let Some(yaml) = yaml_override {
            let parsed = parse_pipeline_yaml(&yaml)?;
            let name = pipeline_name
                .map(|s| s.to_string())
                .or(parsed.name.clone())
                .unwrap_or_else(|| "adhoc".into());
            let p = self
                .register_pipeline(
                    repository_id,
                    org_id,
                    name.clone(),
                    format!(".tt/pipelines/{name}.yml"),
                    yaml.clone(),
                )
                .await?;
            (p.id, name, yaml)
        } else {
            let pipes = self.store.list_pipelines(repository_id).await?;
            let pipe = if let Some(n) = pipeline_name {
                pipes
                    .into_iter()
                    .find(|p| p.name == n && p.enabled)
                    .ok_or(PipelineError::NotFound)?
            } else {
                let mut found = None;
                for p in pipes {
                    if !p.enabled {
                        continue;
                    }
                    if let Ok(parsed) = parse_pipeline_yaml(&p.definition_yaml) {
                        if matches_trigger(&parsed, trigger.as_str(), branch) {
                            found = Some(p);
                            break;
                        }
                    }
                }
                found.ok_or(PipelineError::NotFound)?
            };
            (pipe.id, pipe.name.clone(), pipe.definition_yaml.clone())
        };

        let parsed = parse_pipeline_yaml(&yaml)?;
        if trigger != TaskTrigger::Manual && !matches_trigger(&parsed, trigger.as_str(), branch)
        {
            return Err(PipelineError::Invalid(format!(
                "pipeline does not match trigger {}",
                trigger.as_str()
            )));
        }

        let now = Utc::now();
        let run = TaskRun {
            id: Uuid::now_v7(),
            pipeline_id,
            repository_id,
            org_id,
            pipeline_name: pipeline_name.clone(),
            status: TaskRunStatus::Queued,
            trigger,
            git_ref,
            git_sha,
            triggered_by,
            billable_minutes: 0,
            started_at: None,
            finished_at: None,
            error: None,
            created_at: now,
        };
        self.store.insert_run(&run).await?;

        for (job_name, job_def) in &parsed.jobs {
            let job = TaskJob {
                id: Uuid::now_v7(),
                run_id: run.id,
                name: job_def.name.clone().unwrap_or_else(|| job_name.clone()),
                status: TaskRunStatus::Queued,
                runs_on: job_def.runs_on.clone(),
                billable_minutes: 0,
                log_excerpt: None,
                started_at: None,
                finished_at: None,
            };
            self.store.insert_job(&job).await?;
        }

        self.execute_run(run.id, &parsed).await
    }

    async fn execute_run(&self, run_id: Uuid, parsed: &PipelineFile) -> Result<TaskRun> {
        let mut run = self
            .store
            .get_run(run_id)
            .await?
            .ok_or(PipelineError::NotFound)?;
        run.status = TaskRunStatus::InProgress;
        run.started_at = Some(Utc::now());
        self.store.update_run(&run).await?;

        let mut jobs = self.store.list_jobs(run_id).await?;
        let mut total_minutes = 0u32;

        for job in jobs.iter_mut() {
            job.status = TaskRunStatus::InProgress;
            job.started_at = Some(Utc::now());
            self.store.update_job(job).await?;

            let step_count = parsed
                .jobs
                .get(&job.name)
                .or_else(|| {
                    parsed
                        .jobs
                        .values()
                        .find(|j| j.name.as_deref() == Some(job.name.as_str()))
                })
                .map(|j| j.steps.len().max(1))
                .unwrap_or(1);

            let mins = step_count as u32;
            job.status = TaskRunStatus::Success;
            job.billable_minutes = mins;
            job.log_excerpt = Some(format!(
                "Turing Tasks control-plane stub\njob={}\nsteps={}\nbilled_minutes={mins}\n",
                job.name, step_count
            ));
            job.finished_at = Some(Utc::now());
            self.store.update_job(job).await?;
            total_minutes = total_minutes.saturating_add(mins);
        }

        run.status = TaskRunStatus::Success;
        run.billable_minutes = total_minutes;
        run.finished_at = Some(Utc::now());
        self.store.update_run(&run).await?;

        if let Some(org_id) = run.org_id {
            if total_minutes > 0 {
                if let Err(e) = self
                    .billing
                    .record_usage(
                        org_id,
                        Meter::TasksMinutes,
                        total_minutes as f64,
                        Some(run.id.to_string()),
                    )
                    .await
                {
                    warn!(error = %e, "failed to record task minutes");
                }
            }
        }

        info!(run = %run.id, minutes = total_minutes, "turing task run completed");
        Ok(run)
    }

    pub async fn org_minutes_used(&self, org_id: OrgId, period_ym: &str) -> Result<u32> {
        self.store.sum_org_minutes(org_id, period_ym).await
    }
}
