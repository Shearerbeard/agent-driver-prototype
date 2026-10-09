//! Halt-on-exhaustion bookkeeping for live runs.
//!
//! When a live endpoint returns 429/quota, the runner stops cleanly, writes
//! partial artifacts, and records which (arm, variant_id) pairs completed so
//! the operator can resume after topping up quota.

use serde::{Deserialize, Serialize};

/// One completed scoring unit.
///
/// The same variant is scored under every arm, so the key must include both
/// the arm and the variant id. A bare variant id would mark all three arms as
/// done and silently skip work on resume.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompletedKey {
    pub arm: String,
    pub variant_id: String,
}

impl CompletedKey {
    /// Create a key from an arm name and variant id.
    pub fn new(arm: impl Into<String>, variant_id: impl Into<String>) -> Self {
        Self {
            arm: arm.into(),
            variant_id: variant_id.into(),
        }
    }
}

/// Edge completion state for resumability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumeState {
    /// (arm, variant_id) pairs that completed successfully before exhaustion.
    pub completed: Vec<CompletedKey>,
    /// Human-readable description of the exhausted service and error.
    pub error: String,
}

impl ResumeState {
    /// Create a resume note for a service exhaustion error.
    pub fn exhausted(service: &str, completed: Vec<CompletedKey>, err: &str) -> Self {
        Self {
            completed,
            error: format!("{service} exhausted: {err}"),
        }
    }

    /// Write the resume state next to the artifact directory.
    pub fn write(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("resume.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Check whether a given (arm, variant_id) pair is already completed.
    pub fn is_completed(&self, arm: &str, variant_id: &str) -> bool {
        self.completed
            .iter()
            .any(|k| k.arm == arm && k.variant_id == variant_id)
    }
}

/// Check whether an error represents quota/rate exhaustion that should halt
/// the run rather than retry forever.
pub fn is_exhaustion(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("rate limited")
        || msg.contains("quota")
        || msg.contains("429")
        || msg.contains("exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_state_tracks_arm_variant_pairs() {
        let mut state = ResumeState::exhausted(
            "jev_scorer",
            vec![
                CompletedKey::new("claim_indexed", "v1"),
                CompletedKey::new("head_truncation", "v1"),
            ],
            "test",
        );

        assert!(state.is_completed("claim_indexed", "v1"));
        assert!(state.is_completed("head_truncation", "v1"));
        assert!(!state.is_completed("flash_summarization", "v1"));

        state
            .completed
            .push(CompletedKey::new("flash_summarization", "v1"));
        assert!(state.is_completed("flash_summarization", "v1"));
    }
}
