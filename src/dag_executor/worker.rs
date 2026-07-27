//! The worker inner loop: one `AgentLoop` per task on four tools.

use std::sync::Arc;

use agent_driver_rs::agent::{AgentLoop, AgentLoopConfig, LoopStopReason};
use agent_driver_rs::error::ProviderError;
use agent_driver_rs::{ConfigError, ModelId, Provider, SessionBuilder, SystemPrompt};

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::{InterruptionReason, LoopBudget, SubmitResultTool, TerminalSlot};
use crate::coordinator_loop::WorkerSubmission;
use crate::mcp_client::SidecarClient;
use crate::types::{FailureCategory, Task};

use super::tools::{CapturePaneTool, KeystrokesTool, ReadArtifactTool};

/// Everything a worker inner loop needs before its first provider call.
///
/// Forbidden invalid state: a worker loop that discovers a missing
/// provider, model, or budget mid-run. The constructor takes all three
/// before the loop starts.
#[derive(Clone)]
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
    config: WorkerLoopConfig,
    sidecar: SidecarClient,
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
        task: &Task,
        submission_slot: TerminalSlot<WorkerSubmission>,
    ) -> WorkerOutcome {
        let keystrokes: agent_driver_rs::DynTool =
            Arc::new(KeystrokesTool::new(self.sidecar.clone()));
        let capture_pane: agent_driver_rs::DynTool =
            Arc::new(CapturePaneTool::new(self.sidecar.clone()));
        let read_artifact: agent_driver_rs::DynTool =
            Arc::new(ReadArtifactTool::new(self.artifacts.clone()));
        let submit_result: agent_driver_rs::DynTool =
            Arc::new(SubmitResultTool::new(submission_slot.clone()));

        let session = match SessionBuilder::new()
            .provider(Arc::clone(&self.config.provider))
            .model(self.config.model.clone())
            .system_prompt(self.config.system_prompt.clone())
            .tools([keystrokes, capture_pane, read_artifact, submit_result])
            .build()
            .await
        {
            Ok(session) => session,
            Err(error) => return session_build_error_to_outcome(&error),
        };

        let config = self.agent_loop_config();
        let outcome = match AgentLoop::new(&session)
            .with_config(config)
            .run(&task.description)
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => return agent_loop_error_to_outcome(&error),
        };

        if let Some(submission) = submission_slot.recorded() {
            return WorkerOutcome::Submitted(submission);
        }

        stop_reason_to_outcome(outcome.stop_reason)
    }

    /// The `AgentLoopConfig` derived from the worker budget.
    fn agent_loop_config(&self) -> AgentLoopConfig {
        AgentLoopConfig {
            max_tool_depth: self.config.budget.into(),
            ..AgentLoopConfig::default()
        }
    }
}

/// Map a `ConfigError` from session construction to a [`WorkerOutcome`].
///
/// `ConfigError` is a startup-time failure: a missing provider, an invalid
/// model id, or an unknown provider kind. No provider-level distinction
/// (auth, timeout, rate-limit) is available because no provider call has
/// happened yet. `AgentError` is the honest category for every variant.
fn session_build_error_to_outcome(error: &ConfigError) -> WorkerOutcome {
    tracing::warn!("worker session build failed: {error}");
    WorkerOutcome::Failed(FailureCategory::AgentError)
}

/// Map an `AgentLoopError` from the worker's loop run to a [`WorkerOutcome`].
///
/// The substrate's `AgentLoopError` wraps `SessionError`, which wraps
/// `ProviderError`. Where `ProviderError` carries a distinction the
/// `FailureCategory` enum can name, the mapping preserves it. Everything
/// else collapses to `AgentError`: cancellation is an external signal
/// rather than a worker failure, invalid config is a startup defect, and
/// the remaining `ProviderError` variants (stream errors, HTTP errors,
/// invalid requests) have no dedicated `FailureCategory`.
fn agent_loop_error_to_outcome(error: &agent_driver_rs::AgentLoopError) -> WorkerOutcome {
    let category = match error.as_provider_error() {
        Some(ProviderError::Auth { .. }) => FailureCategory::ProviderAuthError,
        Some(ProviderError::Timeout(_)) => FailureCategory::AgentTimeout,
        Some(ProviderError::ContextWindowExceeded { .. }) => FailureCategory::ContextOverflow,
        Some(ProviderError::ModelNotFound { .. }) => FailureCategory::ProviderNotFound,
        Some(ProviderError::RateLimited { .. }) => FailureCategory::ProviderOverloaded,
        _ => FailureCategory::AgentError,
    };
    tracing::warn!("worker agent loop failed: {error}");
    WorkerOutcome::Failed(category)
}

/// Map the substrate's stop reason to a [`WorkerOutcome`] when the worker
/// did not submit a result.
fn stop_reason_to_outcome(reason: LoopStopReason) -> WorkerOutcome {
    match reason {
        LoopStopReason::EndTurn => WorkerOutcome::StoppedWithoutSubmission,
        LoopStopReason::MaxToolDepthReached => WorkerOutcome::BudgetExhausted,
        LoopStopReason::MaxTokens => {
            WorkerOutcome::Interrupted(InterruptionReason::TokenLimit)
        }
        LoopStopReason::StopSequence => {
            WorkerOutcome::Interrupted(InterruptionReason::StopSequence)
        }
        LoopStopReason::ContentFilter => {
            WorkerOutcome::Interrupted(InterruptionReason::ContentFilter)
        }
        LoopStopReason::Cancelled => WorkerOutcome::Interrupted(
            InterruptionReason::Unclassified("cancelled".to_owned()),
        ),
        LoopStopReason::ToolError { .. } | LoopStopReason::LoopFailed { .. } => {
            WorkerOutcome::Failed(FailureCategory::AgentError)
        }
        _ => WorkerOutcome::Failed(FailureCategory::AgentError),
    }
}
