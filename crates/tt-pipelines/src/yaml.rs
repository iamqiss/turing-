//! Parse `.tt/pipelines/*.yml` (GitHub Actions–shaped subset).

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::{PipelineError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct PipelineFile {
    pub name: Option<String>,
    #[serde(default, rename = "on")]
    pub on: Triggers,
    #[serde(default)]
    pub jobs: HashMap<String, JobDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Triggers {
    #[serde(default)]
    pub push: Option<BranchFilter>,
    #[serde(default)]
    pub pull_request: Option<BranchFilter>,
    #[serde(default)]
    pub workflow_dispatch: Option<bool>,
    #[serde(default)]
    pub schedule: Option<Vec<ScheduleItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BranchFilter {
    #[serde(default)]
    pub branches: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleItem {
    pub cron: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobDef {
    #[serde(default = "default_runs_on", rename = "runs-on")]
    pub runs_on: String,
    #[serde(default)]
    pub steps: Vec<StepDef>,
    #[serde(default)]
    pub name: Option<String>,
}

fn default_runs_on() -> String {
    "tt-linux-latest".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepDef {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub with: Option<HashMap<String, String>>,
}

pub fn parse_pipeline_yaml(yaml: &str) -> Result<PipelineFile> {
    let p: PipelineFile =
        serde_yaml::from_str(yaml).map_err(|e| PipelineError::Invalid(format!("yaml: {e}")))?;
    if p.jobs.is_empty() {
        return Err(PipelineError::Invalid("pipeline has no jobs".into()));
    }
    Ok(p)
}

pub fn matches_trigger(p: &PipelineFile, trigger: &str, branch: Option<&str>) -> bool {
    match trigger {
        "manual" | "workflow_dispatch" => true,
        "push" => match &p.on.push {
            None => false,
            Some(f) if f.branches.is_empty() => true,
            Some(f) => branch
                .map(|b| f.branches.iter().any(|x| x == b || x == "*"))
                .unwrap_or(true),
        },
        "pull_request" => match &p.on.pull_request {
            None => false,
            Some(f) if f.branches.is_empty() => true,
            Some(f) => branch
                .map(|b| f.branches.iter().any(|x| x == b || x == "*"))
                .unwrap_or(true),
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let y = r#"
name: ci
on:
  push:
    branches: [main]
jobs:
  test:
    runs-on: tt-linux-latest
    steps:
      - run: cargo test
"#;
        let p = parse_pipeline_yaml(y).unwrap();
        assert!(p.jobs.contains_key("test"));
        assert!(matches_trigger(&p, "push", Some("main")));
        assert!(!matches_trigger(&p, "push", Some("dev")));
    }
}
