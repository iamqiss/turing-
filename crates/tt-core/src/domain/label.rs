use serde::{Deserialize, Serialize};

use crate::ids::{LabelId, RepositoryId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: LabelId,
    pub repository_id: RepositoryId,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}
