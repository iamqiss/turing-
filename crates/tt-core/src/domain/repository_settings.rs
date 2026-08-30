use serde::{Deserialize, Serialize};

use crate::ids::RepositoryId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    /// Classic merge commit (`git merge --no-ff`).
    Merge,
    /// Squash all commits from source into one on target.
    Squash,
    /// Rebase source onto target, fast-forward target.
    Rebase,
}

impl MergeMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "merge" => Some(Self::Merge),
            "squash" => Some(Self::Squash),
            "rebase" => Some(Self::Rebase),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySettings {
    pub repository_id: RepositoryId,
    pub allowed_merge_methods: Vec<MergeMethod>,
    pub default_merge_method: MergeMethod,
    pub delete_branch_on_merge: bool,
    /// Source branch must include the tip of target (no behind commits).
    pub require_up_to_date: bool,
    /// Only the repository creator may merge (v1 admin stand-in).
    pub restrict_merge_to_admins: bool,
    /// Minimum approving reviews required before merge (0 = off).
    pub required_approvals: u32,
}

impl RepositorySettings {
    pub fn defaults(repository_id: RepositoryId) -> Self {
        Self {
            repository_id,
            allowed_merge_methods: vec![
                MergeMethod::Merge,
                MergeMethod::Squash,
                MergeMethod::Rebase,
            ],
            default_merge_method: MergeMethod::Merge,
            delete_branch_on_merge: false,
            require_up_to_date: false,
            restrict_merge_to_admins: false,
            required_approvals: 0,
        }
    }

    pub fn allows(&self, method: MergeMethod) -> bool {
        if self.allowed_merge_methods.is_empty() {
            return true;
        }
        self.allowed_merge_methods.contains(&method)
    }

    pub fn resolve_method(&self, requested: Option<MergeMethod>) -> Result<MergeMethod, String> {
        let method = requested.unwrap_or(self.default_merge_method);
        if !self.allows(method) {
            return Err(format!(
                "merge method '{}' is not allowed on this repository (allowed: {})",
                method.as_str(),
                self.allowed_merge_methods
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(method)
    }
}
