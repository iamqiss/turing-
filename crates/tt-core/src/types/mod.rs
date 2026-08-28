//! Shared value types that are not full entities.

use serde::{Deserialize, Serialize};

/// Simple markdown/plain content for v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichText {
    pub raw: String,
    // Future: blocks, mentions parsed, etc.
}

impl RichText {
    pub fn plain(s: impl Into<String>) -> Self {
        Self { raw: s.into() }
    }
}
