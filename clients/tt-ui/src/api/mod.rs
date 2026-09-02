use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default)]
pub struct TtClient {
    pub base_url: String,
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityItem {
    pub id: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: Option<String>,
    pub time_ago: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRow {
    pub name: String,
    pub full_name: String,
    pub branch: String,
    pub commits: u32,
    pub prs: u32,
    pub issues: u32,
    pub updated: String,
}

pub fn mock_activity() -> Vec<ActivityItem> {
    vec![]
}

pub fn mock_repos() -> Vec<RepoRow> {
    vec![]
}
