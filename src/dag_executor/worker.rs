//! The worker inner loop: one `AgentLoop` per task on four tools.

use std::sync::Arc;

use agent_driver_rs::agent::{
    AgentEvent, AgentLoop, AgentLoopConfig, AgentObserver, LoopStopReason,
};
use agent_driver_rs::error::ProviderError;
use agent_driver_rs::{ConfigError, ModelId, Provider, SessionBuilder, SystemPrompt};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::WorkerSubmission;
use crate::coordinator_loop::{InterruptionReason, LoopBudget, SubmitResultTool, TerminalSlot};
use crate::mcp_client::SidecarClient;
use crate::types::{FailureCategory, Task};

use super::tools::{CapturePaneTool, KeystrokesTool, ReadArtifactTool};

/// One task run's observation lane: the observer the worker loop attaches,
/// and the provider the loop should run on.
///
/// Both pieces are per-task by the same token: the observer tags its
/// events with the task and worker identity, and the provider carries any
/// per-agent metering state (the shim's usage lanes), so a lane's
/// final-turn attribution stays correct even when sibling tool calls run
/// concurrently.
pub struct WorkerLane {
    pub observer: Arc<dyn AgentObserver>,
    pub provider: Arc<dyn Provider>,
}

/// Mints the per-task observation lane a worker loop runs on.
///
/// The factory (not a bare observer) rides in [`WorkerLoopConfig`] because
/// the lane is per-task: it needs the task id and the worker's agent id,
/// which the executor resolves per dispatch. The shim implements this to
/// return a `ShimWorkerObserver` plus its metered provider lane, both
/// wired to the request's SSE channel and usage accounting; `None`
/// (tests, the standalone bins) runs workers unobserved on the template
/// provider, exactly as before S102.
pub trait WorkerObserverFactory: Send + Sync {
    /// The lane for one task run: `task_id` identifies the plan task and
    /// `worker_id` the assigned worker (the executor's own resolution,
    /// e.g. `"default"` for an unassigned task).
    fn lane_for(&self, task_id: usize, worker_id: &str) -> WorkerLane;
}

/// Forwards loop events to a shared observer handle, so an `Arc<dyn
/// AgentObserver>` can attach to a loop that takes its observer by value
/// (the pin has no `AgentObserver for Arc<dyn AgentObserver>` impl — same
/// wrapper the coordinator driver uses).
struct SharedWorkerObserver(Arc<dyn AgentObserver>);

#[async_trait]
impl AgentObserver for SharedWorkerObserver {
    async fn on_event(&self, event: &AgentEvent) {
        self.0.on_event(event).await;
    }
}

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
    /// The run's cancellation token: a child of the request token. The
    /// honored value is the executor's per-dispatch
    /// `ctx.cancellation.child_token()`, never the stored template value.
    /// Both cancel paths - the loop-top stop and the in-flight
    /// `AgentLoopError::Cancelled` - converge on `WorkerOutcome::Interrupted`.
    pub cancellation: CancellationToken,
    /// Optional per-task observer factory (S102): the executor resolves the
    /// task and worker identity, the factory mints the lane (observer plus
    /// provider), and the worker loop attaches the observer and runs on the
    /// lane's provider so worker tool calls, reasoning, and final-turn
    /// context reach the SSE stream. `None` runs unobserved on the template
    /// provider.
    pub observer_factory: Option<Arc<dyn WorkerObserverFactory>>,
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
    pub fn new(config: WorkerLoopConfig, sidecar: SidecarClient, artifacts: ArtifactStore) -> Self {
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

        // S102: mint the per-task lane up front — its provider replaces the
        // template for this run, and its observer attaches to the loop. The
        // worker id mirrors the executor's own resolution (task.worker,
        // falling back to "default") so lifecycle and tool events agree on
        // the agent identity.
        let worker_id = task.worker.as_deref().unwrap_or("default");
        let lane = self
            .config
            .observer_factory
            .as_ref()
            .map(|factory| factory.lane_for(task.id, worker_id));
        let provider = match &lane {
            Some(lane) => Arc::clone(&lane.provider),
            None => Arc::clone(&self.config.provider),
        };

        let session = match SessionBuilder::new()
            .provider(provider)
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
        let mut agent_loop = AgentLoop::new(&session)
            .with_config(config)
            .with_cancellation(self.config.cancellation.clone());
        if let Some(lane) = lane {
            agent_loop = agent_loop.with_observer(SharedWorkerObserver(lane.observer));
        }
        let outcome = match agent_loop.run(&task.description).await {
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
/// `ProviderError`. Cancellation is an external signal rather than a worker
/// failure, so it maps to `Interrupted` like the loop-top stop path. Where
/// the remaining errors carry a `ProviderError` distinction the
/// `FailureCategory` enum can name, the mapping preserves it. Everything
/// else collapses to `AgentError`: invalid config is a startup defect, and
/// the remaining `ProviderError` variants (stream errors, HTTP errors,
/// invalid requests) have no dedicated `FailureCategory`.
fn agent_loop_error_to_outcome(error: &agent_driver_rs::AgentLoopError) -> WorkerOutcome {
    match error {
        agent_driver_rs::AgentLoopError::Cancelled => {
            WorkerOutcome::Interrupted(InterruptionReason::Unclassified("cancelled".to_owned()))
        }
        error => {
            let category = match error.as_provider_error() {
                Some(ProviderError::Auth { .. }) => FailureCategory::ProviderAuthError,
                Some(ProviderError::Timeout(_)) => FailureCategory::AgentTimeout,
                Some(ProviderError::ContextWindowExceeded { .. }) => {
                    FailureCategory::ContextOverflow
                }
                Some(ProviderError::ModelNotFound { .. }) => FailureCategory::ProviderNotFound,
                Some(ProviderError::RateLimited { .. }) => FailureCategory::ProviderOverloaded,
                _ => FailureCategory::AgentError,
            };
            tracing::warn!("worker agent loop failed: {error}");
            WorkerOutcome::Failed(category)
        }
    }
}

/// Map the substrate's stop reason to a [`WorkerOutcome`] when the worker
/// did not submit a result.
fn stop_reason_to_outcome(reason: LoopStopReason) -> WorkerOutcome {
    match reason {
        LoopStopReason::EndTurn => WorkerOutcome::StoppedWithoutSubmission,
        LoopStopReason::MaxToolDepthReached => WorkerOutcome::BudgetExhausted,
        LoopStopReason::MaxTokens => WorkerOutcome::Interrupted(InterruptionReason::TokenLimit),
        LoopStopReason::StopSequence => {
            WorkerOutcome::Interrupted(InterruptionReason::StopSequence)
        }
        LoopStopReason::ContentFilter => {
            WorkerOutcome::Interrupted(InterruptionReason::ContentFilter)
        }
        LoopStopReason::Cancelled => {
            WorkerOutcome::Interrupted(InterruptionReason::Unclassified("cancelled".to_owned()))
        }
        LoopStopReason::ToolError { .. } | LoopStopReason::LoopFailed { .. } => {
            WorkerOutcome::Failed(FailureCategory::AgentError)
        }
        _ => WorkerOutcome::Failed(FailureCategory::AgentError),
    }
}
