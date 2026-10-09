//! W7 JEV rubric research spike.
//!
//! Offline-only phase: sealed rubric v2 as jev-driver derive types, a
//! labelled synthetic edge corpus parsed from aura SSE captures, three
//! evidence-selection arms, and a replay runner that writes versioned
//! verdict artifacts.

pub mod accounting;
pub mod arms;
pub mod chat;
pub mod claim;
pub mod client;
pub mod corpus;
pub mod exhaustion;
pub mod http_retry;
pub mod judge;
pub mod report;
pub mod rubric;
pub mod runner;
pub mod state;
pub mod summarizer;
pub mod verdict;

use std::path::PathBuf;

/// Default artifact directory for the spike.
#[must_use]
pub fn default_artifact_dir() -> PathBuf {
    PathBuf::from("edge-spike-artifacts")
}
