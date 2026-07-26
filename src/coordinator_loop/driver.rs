//! The wrapper that assembles the session, drives one loop, and reads the
//! result back.

use std::sync::Arc;

use agent_driver_rs::{ModelId, Provider, Session, SystemPrompt};

use crate::bounding::ToolListLimit;
use crate::config::{OrchestrationConfig, VectorStoreConfig};
use crate::context::PinnedGoal;

use super::budget::LoopBudget;
use super::error::CoordinatorRunError;
use super::executor::PlanExecutor;
use super::outcome::CoordinatorOutcome;
use super::run_store::RunStore;
use super::terminal::{FinalResponse, TerminalSlot};

/// The worker roster and assignment guidelines the planning message
/// interpolates.
///
/// The two halves are produced together from one configuration and travel
/// together, so a message cannot describe one roster while instructing the
/// coordinator to assign from another.
#[derive(Debug, Clone, Default)]
pub struct WorkerSections {
    roster: String,
    guidelines: String,
}

impl WorkerSections {
    /// Render both sections from an orchestration configuration.
    pub fn from_config(
        _config: &OrchestrationConfig,
        _tool_list_limit: ToolListLimit,
        _vector_stores: &[VectorStoreConfig],
    ) -> Self {
        todo!("S71 Phase 2")
    }

    /// The rendered roster section.
    pub fn roster(&self) -> &str {
        todo!("S71 Phase 2")
    }

    /// The rendered worker-assignment guidelines.
    pub fn guidelines(&self) -> &str {
        todo!("S71 Phase 2")
    }
}

/// Everything the loop needs before its first provider call.
///
/// The system prompt is supplied rather than composed here: the ported
/// preamble builder describes the bounded router's tool surface, which is
/// not the surface this loop registers, so composing it in would ship a
/// system prompt that contradicts the tools.
pub struct CoordinatorLoopConfig {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    pub system_prompt: SystemPrompt,
    pub budget: LoopBudget,
    pub executor: Arc<dyn PlanExecutor>,
    pub worker_sections: WorkerSections,
}

/// One coordinator run; its session and answer slot never outlive it.
///
/// Running consumes the loop. The answer slot and the run records belong to
/// a single run, so a second run over the same loop would inherit an answer
/// it did not write; making `run` take ownership removes that state rather
/// than documenting it.
pub struct CoordinatorLoop {
    session: Session,
    budget: LoopBudget,
    answer: TerminalSlot<FinalResponse>,
    runs: RunStore,
    worker_sections: WorkerSections,
}

impl CoordinatorLoop {
    /// Build the session, register the loop tools, and arm the budget.
    ///
    /// The registered surface is `create_plan`, `execute`, `inspect_run` and
    /// `respond`. Worker result submission is deliberately absent: it is a
    /// worker's tool, and mounting it here would offer the coordinator a way
    /// to report evidence it never gathered.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorRunError::Session`] when the provider and model
    /// do not yield a session.
    pub async fn new(_config: CoordinatorLoopConfig) -> Result<Self, CoordinatorRunError> {
        todo!("S71 Phase 2")
    }

    /// The run's records, shareable before the run consumes the loop.
    ///
    /// [`RunStore`] is a handle, so a caller that clones it here still sees
    /// what the loop wrote once the run is over.
    pub fn runs(&self) -> &RunStore {
        todo!("S71 Phase 2")
    }

    /// Run the loop over one user query.
    ///
    /// The opening message is the rendered planning wrapper, so the loop
    /// starts from the same template the frame corpus pins rather than from
    /// a message assembled at the call site. Everything after it is ordinary
    /// conversation history: tool calls and their observations, with no
    /// state replayed into a prompt.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorRunError::AgentLoop`] when the substrate loop
    /// fails outright. A loop that stops for any reported reason — including
    /// the turn budget — is an outcome, not an error.
    pub async fn run(self, _query: &PinnedGoal) -> Result<CoordinatorOutcome, CoordinatorRunError> {
        todo!("S71 Phase 2")
    }
}
