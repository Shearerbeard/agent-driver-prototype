//! The worker inner loop: one `AgentLoop` per task on four tools.

use std::sync::Arc;

use agent_driver_rs::agent::AgentLoopConfig;
use agent_driver_rs::{ModelId, Provider, SystemPrompt};

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::InterruptionReason;
use crate::coordinator_loop::LoopBudget;
use crate::coordinator_loop::TerminalSlot;
use crate::coordinator_loop::WorkerSubmission;
use crate::mcp_client::SidecarClient;
use crate::types::{FailureCategory, Plan};

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

/// What a worker run produced, mirroring the S71 `CoordinatorOutcome` pattern.
///
/// The substrate reports why its loop stopped; that alone does not say what
/// the caller gets. This enum is the join of the stop reason with the
/// submission slot, separating the cases the executor must treat
/// differently: a submitted result, a clean stop with nothing written, a
/// budget exhaustion, a provider interruption, and a hard failure with a
/// category.
///
/// The `DagExecutor` maps `WorkerOutcome` to [`FailureCategory`] to classify
/// the task's failure for the coordinator:
///
/// | Variant | `FailureCategory` |
/// |---|---|
/// | `Submitted` | (not a failure) |
/// | `StoppedWithoutSubmission` | `DepthExhausted` |
/// | `BudgetExhausted` | `DepthExhausted` |
/// | `Interrupted` | `AgentTimeout` or the closest matching category |
/// | `Failed` | the carried category |
#[derive(Debug, Clone)]
pub enum WorkerOutcome {
    /// The worker submitted a result through `submit_result`.
    Submitted(WorkerSubmission),
    /// The worker ended its turn without calling `submit_result`.
    StoppedWithoutSubmission,
    /// The worker exhausted its tool-depth budget without submitting.
    BudgetExhausted,
    /// The provider stopped generating before the worker finished.
    Interrupted(InterruptionReason),
    /// The worker failed with a structured category.
    Failed(FailureCategory),
}

/// One worker inner loop: runs a single task to completion or budget.
///
/// The loop is constructed per task because the worker submission slot and
/// the artifact handles are per-task. The four worker tools mount as
/// `Arc<dyn Tool>` (concrete impls): `KeystrokesTool`, `CapturePaneTool`,
/// and `ReadArtifactTool` from `dag_executor::tools`, plus `SubmitResultTool`
/// reused from `coordinator_loop`. Each tool captures the sidecar client or
/// artifact store it forwards through. The skeleton declares the wrapper
/// type; the mounting body lands in Phase 2.
///
/// The submission slot is per-task: the `DagExecutor` mints a fresh
/// `TerminalSlot` for each task, so a second task cannot inherit the
/// first's slot. A second write to the same slot is detected at runtime
/// via [`AlreadyRecorded`](crate::coordinator_loop::AlreadyRecorded),
/// matching S71's honest-claim standard: the single-use property is not
/// type-enforced but is detected at runtime, and the executor's per-task
/// construction prevents production from sharing a slot.
pub struct WorkerLoop {
    #[allow(dead_code)]
    config: WorkerLoopConfig,
    #[allow(dead_code)]
    sidecar: SidecarClient,
    #[allow(dead_code)]
    artifacts: ArtifactStore,
}

impl WorkerLoop {
    /// Build a worker loop from its config and tool dependencies.
    ///
    /// The `sidecar` is the connected MCP client the `keystrokes` and
    /// `capture-pane` tools forward through. The `artifacts` is the
    /// filename-addressed store the `read_artifact` tool reads from.
    /// The loop builds the four-tool set per task from these handles.
    pub fn new(
        config: WorkerLoopConfig,
        sidecar: SidecarClient,
        artifacts: ArtifactStore,
    ) -> Self {
        Self {
            config,
            sidecar,
            artifacts,
        }
    }

    /// Run one task and read the worker's outcome.
    ///
    /// Returns [`WorkerOutcome`] rather than `Option<WorkerSubmission>` so
    /// every non-submission case is distinguishable: a clean stop, a budget
    /// exhaustion, a provider interruption, and a hard failure each carry
    /// the information the executor needs to classify the task's failure.
    pub async fn run_task(
        &self,
        task: &Plan,
        submission_slot: TerminalSlot<WorkerSubmission>,
    ) -> WorkerOutcome {
        let _ = (task, submission_slot);
        todo!(
            "Phase 2: build session with four Arc<dyn Tool>-mounted tools, \
             run AgentLoop, read the submission slot, map the stop reason \
             to a WorkerOutcome"
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
