//! Structured worker output via the `submit_result` tool.
//!
//! Workers call `submit_result` as their final action to provide structured
//! output with a summary, full result, and confidence level. Uses the same
//! first-write-wins pattern as coordinator routing tools.

use serde::{Deserialize, Serialize};

/// Worker-reported confidence level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => write!(f, "high"),
            Confidence::Medium => write!(f, "medium"),
            Confidence::Low => write!(f, "low"),
        }
    }
}
