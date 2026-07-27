//! The wrapper that assembles the session, drives one loop, and reads the
//! result back.

use std::sync::Arc;

use agent_driver_rs::agent::{AgentEvent, AgentLoop, AgentLoopConfig, AgentObserver};
use agent_driver_rs::{DynTool, ModelId, Provider, Session, SessionBuilder, SystemPrompt};
use async_trait::async_trait;

use crate::bounding::ToolListLimit;
use crate::config::{OrchestrationConfig, VectorStoreConfig};
use crate::context::PinnedGoal;
use crate::producers::{build_planning_wrapper, build_worker_prompt_sections};

use super::budget::LoopBudget;
use super::error::CoordinatorRunError;
use super::executor::PlanExecutor;
use super::outcome::CoordinatorOutcome;
use super::roster::WorkerRoster;
use super::run_store::RunStore;
use super::terminal::{FinalResponse, TerminalSlot};
use super::tools::{CreatePlanTool, ExecuteTool, InspectRunTool, RespondTool};

/// The worker material one configuration produces for the loop.
///
/// The roster text, the assignment guidelines and the worker-field fragment
/// are produced together from one configuration and travel together, so the
/// planning message cannot describe one roster while the planning schema
/// offers another.
#[derive(Debug, Clone, Default)]
pub struct WorkerSections {
    roster_section: String,
    worker_field: String,
    guidelines: String,
    roster: WorkerRoster,
}

impl WorkerSections {
    /// Render every worker section from an orchestration configuration.
    ///
    /// This is the parallel-derivation path: the roster text and the typed
    /// roster are produced independently from the same config, so a prose
    /// mismatch is representable. It stays until the single-derivation
    /// [`from_roster`](Self::from_roster) path is wired in and the S70
    /// goldens are re-goldened against it.
    pub fn from_config(
        config: &OrchestrationConfig,
        tool_list_limit: ToolListLimit,
        vector_stores: &[VectorStoreConfig],
    ) -> Self {
        let (roster_section, worker_field, guidelines) =
            build_worker_prompt_sections(config, tool_list_limit, vector_stores);
        Self {
            roster_section,
            worker_field,
            guidelines,
            roster: WorkerRoster::from_config(config),
        }
    }

    /// Render every worker section from a typed [`WorkerRoster`].
    ///
    /// This is the single-derivation path: the roster text, the worker-field
    /// fragment, and the guidelines are all rendered from the typed roster,
    /// so a prose/schema roster mismatch is unrepresentable. The skeleton
    /// declares the signature; the render body lands in Phase 2, at which
    /// point `from_config` is retired and the goldens are re-goldened
    /// against this path.
    ///
    /// # Switchover plan
    ///
    /// 1. Phase 2 implements the render body, reading worker descriptions
    ///    and tool lists from the roster's typed entries.
    /// 2. The call site in `CoordinatorLoop::run` switches from
    ///    `from_config` to `from_roster`.
    /// 3. The S70 goldens are re-goldened against the single-derivation
    ///    output.
    /// 4. `from_config` and `build_worker_prompt_sections` are deleted.
    pub fn from_roster(roster: WorkerRoster) -> Self {
        let _ = roster;
        todo!(
            "Phase 2: render roster_section, worker_field, and guidelines \
             from the typed WorkerRoster so the parallel derivation is removed"
        )
    }

    /// A run with no workers configured, which is what the producer returns
    /// for a configuration that has none.
    pub fn none() -> Self {
        Self::default()
    }

    /// The rendered roster section of the planning message.
    pub fn roster_section(&self) -> &str {
        &self.roster_section
    }

    /// The ported worker-field fragment, which shows the model the exact
    /// shape of an assigned task step.
    pub fn worker_field(&self) -> &str {
        &self.worker_field
    }

    /// The rendered worker-assignment guidelines.
    pub fn guidelines(&self) -> &str {
        &self.guidelines
    }

    /// The names a plan may assign work to.
    pub fn roster(&self) -> &WorkerRoster {
        &self.roster
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

/// Forwards loop events to a shared observer handle.
///
/// The substrate takes an owned observer, so a caller that wants to read the
/// events after the run hands in a handle and keeps a clone.
struct SharedObserver(Arc<dyn AgentObserver>);

#[async_trait]
impl AgentObserver for SharedObserver {
    async fn on_event(&self, event: &AgentEvent) {
        self.0.on_event(event).await;
    }
}

/// One coordinator run: one session, one budget, one answer.
///
/// Running consumes the loop. The answer slot and the run records belong to
/// a single run, so a second run over the same loop would inherit an answer
/// it did not write; making `run` take ownership removes that state rather
/// than documenting it. Both are handles, so a caller clones what it wants
/// to read before handing the loop over.
pub struct CoordinatorLoop {
    session: Session,
    budget: LoopBudget,
    answer: TerminalSlot<FinalResponse>,
    runs: RunStore,
    worker_sections: WorkerSections,
    observer: Option<Arc<dyn AgentObserver>>,
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
    pub async fn new(config: CoordinatorLoopConfig) -> Result<Self, CoordinatorRunError> {
        let runs = RunStore::new();
        let answer: TerminalSlot<FinalResponse> = TerminalSlot::new();

        let create_plan: DynTool =
            Arc::new(CreatePlanTool::new(runs.clone(), &config.worker_sections));
        let execute: DynTool =
            Arc::new(ExecuteTool::new(runs.clone(), Arc::clone(&config.executor)));
        let inspect_run: DynTool = Arc::new(InspectRunTool::new(runs.clone()));
        let respond: DynTool = Arc::new(RespondTool::new(answer.clone()));

        let session = SessionBuilder::new()
            .provider(config.provider)
            .model(config.model)
            .system_prompt(config.system_prompt)
            .tools([create_plan, execute, inspect_run, respond])
            .build()
            .await?;

        Ok(Self {
            session,
            budget: config.budget,
            answer,
            runs,
            worker_sections: config.worker_sections,
            observer: None,
        })
    }

    /// Watch the loop's events as they happen.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn AgentObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// The run's records, shareable before the run consumes the loop.
    ///
    /// [`RunStore`] is a handle, so a caller that clones it here still sees
    /// what the loop wrote once the run is over.
    pub fn runs(&self) -> &RunStore {
        &self.runs
    }

    /// The run's answer slot, shareable before the run consumes the loop.
    ///
    /// Symmetric with [`runs`](Self::runs), and the only way to recover a
    /// committed answer from a run that ends in
    /// [`CoordinatorRunError`](super::CoordinatorRunError) rather than an
    /// outcome.
    pub fn answer(&self) -> &TerminalSlot<FinalResponse> {
        &self.answer
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
    /// fails outright. A loop that stops for any reported reason, the turn
    /// budget included, is an outcome rather than an error.
    pub async fn run(self, query: &PinnedGoal) -> Result<CoordinatorOutcome, CoordinatorRunError> {
        let message = build_planning_wrapper(
            query.as_str(),
            self.worker_sections.roster_section(),
            self.worker_sections.guidelines(),
        );

        let config = AgentLoopConfig {
            max_tool_depth: self.budget.into(),
            ..AgentLoopConfig::default()
        };

        let mut agent = AgentLoop::new(&self.session).with_config(config);
        if let Some(observer) = &self.observer {
            agent = agent.with_observer(SharedObserver(Arc::clone(observer)));
        }

        let outcome = agent.run(message).await?;
        Ok(CoordinatorOutcome::interpret(
            outcome,
            &self.answer,
            &self.runs,
        ))
    }
}
