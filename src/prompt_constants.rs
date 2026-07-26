//! Prompt section headers as constants for type safety.
//!
//! This module centralizes all string constants used in orchestration prompts,
//! ensuring consistency across the codebase and enabling easy updates.
//!
//! # Research Inspirations
//!
//! Based on patterns from:
//! - LangChain's Write-Select-Compress-Isolate framework
//! - LlamaIndex's Context Store with typed state
//! - Anthropic's context engineering principles

/// Section headers for the continuation prompt (post-execute decision point).
pub mod continuation {
    pub const COMPLETED_TASKS: &str = "COMPLETED TASKS:";
    pub const BLOCKED_TASKS: &str = "BLOCKED TASKS (dependencies failed):";
    pub const FAILED_TASKS: &str = "FAILED TASKS:";
    pub const FAILURE_SUMMARY: &str = "FAILURE SUMMARY:";
    pub const AREAS_NEEDING_ATTENTION: &str = "AREAS NEEDING ATTENTION:";
    pub const FAILURE_HISTORY: &str = "FAILURE HISTORY:";
    pub const OBSERVED_PATTERNS: &str = "OBSERVED PATTERNS:";
    pub const FINAL_ATTEMPT: &str = " (FINAL ATTEMPT)";
    pub const REPEATED_FAILURE_SUFFIX: &str = " — consider a fundamentally different approach";
}

/// Conditional guidance fragments injected into continuation prompts.
pub(crate) mod guidance {
    pub const RESULT_FORWARDING: &str = "When creating a follow-up plan, do not re-execute completed tasks. Workers cannot see prior iteration results — if a new task needs data from a completed task above, embed the key values in the task description or include the artifact filename so the worker can call `read_artifact`.\n\n";
}
