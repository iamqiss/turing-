//! Human gates for agentic actions — small models must not free-run writes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DangerLevel {
    Read,
    Write,
    Dangerous,
}

#[derive(Debug, Clone)]
pub struct ToolPolicy {
    pub danger: DangerLevel,
    pub requires_approval: bool,
}

impl DangerLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Dangerous => "dangerous",
        }
    }
}
