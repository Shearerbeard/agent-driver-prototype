//! The worker inner loop: one `AgentLoop` per task on four tools.

use std::sync::Arc;

use agent_driver_rs::agent::AgentLoopConfig;
use agent_driver_rs::{ModelId, Provider, SystemPrompt};

use crate::coordinator_loop::LoopBudget;
use crate::coordinator_loop::WorkerSubmission;
use crate::types::Plan;

/// Everything a worker inner loop needs before its first provider call.
///
/// Forbidden invalid state: a worker loop that discovers a missing
/// provider, model, or budget mid-run. The constructor takes all three
/// before the loop starts.
pub struct WorkerLoopConfig {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    pub budget: LoopBudget,
    pub system_prompt: SystemPrompt,
}

/// One worker inner loop: runs a single task to completion or budget.
///
/// The loop is constructed per task because the worker submission slot and
/// the artifact handles are per-task. The `FnTool` seam is how the four
/// worker tools are mounted on the worker's session: each tool is a closure
/// that captures the sidecar client or artifact store and forwards the
/// call. The skeleton declares the wrapper type; the mounting body lands in
/// Phase 2.
pub struct WorkerLoop {
    #[allow(dead_code)]
    config: WorkerLoopConfig,
}

impl WorkerLoop {
    /// Build a worker loop from its config.
    pub fn new(config: WorkerLoopConfig) -> Self {
        Self { config }
    }

    /// Run one task and read the worker's submission.
    ///
    /// Returns `None` when the worker exhausted its budget without calling
    /// `submit_result`; the caller reports that as a failure.
    pub async fn run_task(
        &self,
        task: &Plan,
        submission_slot: crate::coordinator_loop::TerminalSlot<WorkerSubmission>,
    ) -> Option<WorkerSubmission> {
        let _ = (task, submission_slot);
        todo!(
            "Phase 2: build session with four FnTool-mounted tools, \
             run AgentLoop, read the submission slot"
        )
    }

    /// The `AgentLoopConfig` derived from the worker budget.
    #[allow(dead_code)]
    fn agent_loop_config(&self) -> AgentLoopConfig {
        AgentLoopConfig {
            max_tool_depth: self.config.budget.into(),
            ..AgentLoopConfig::default()
        }
    }
}
