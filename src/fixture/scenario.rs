//! Fixture scenario types: the typed-context backbone of the S2
//! golden-frame corpus.
//!
//! Each type here maps to exactly one business rule of the prompt-assembly
//! path at commit `9df96382`, and names the invalid state it forbids. The
//! full type -> rule -> forbidden-state table is `DESIGN.md` in the
//! aura source directory; the surface/branch coverage ledger is `MANIFEST.md`.
//!
//! Scenario types COMPOSE the existing context-module parse-don't-validate
//! types ([`PinnedGoal`], [`EvidenceText`], [`WorkerClaim`],
//! [`SpilledArtifact`], [`ResultPreview`]) and the production state types
//! ([`Plan`], [`FailureSummary`], [`ToolTraceEntry`], [`RunManifest`],
//! [`OrchestrationConfig`]); they never re-model what those already forbid.

use crate::config::{SkillConfig, ToolVisibility, VectorStoreConfig};
use crate::config::OrchestrationConfig;
use crate::context::{
    ContextError, EvidenceText, PinnedGoal, ResultPreview, SpilledArtifact, WorkerClaim,
};
use crate::persistence::{RunManifest, ToolTraceEntry};
use crate::types::{FailureCategory, FailureSummary, Plan, PlanningResponse};
use std::num::NonZeroUsize;

/// Why a fixture scenario failed to construct.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("context value rejected: {0}")]
    Context(#[from] ContextError),
    #[error("mid-thread decision must be create_plan; terminal decisions end the run")]
    TerminalDecisionMidThread,
    #[error("plan decision steps do not flatten into an executable plan")]
    UnflattenablePlan,
    #[error("iteration outcomes ({outcomes}) do not match the decision's task count ({tasks})")]
    OutcomeCountMismatch { tasks: usize, outcomes: usize },
    #[error("failure summary requires at least one failed or blocked task outcome")]
    FailureSummaryWithoutFailure,
    #[error("continuation thread has no completed iterations")]
    EmptyContinuationThread,
    #[error("{iterations} completed iterations leave no planning call within budget {budget}")]
    IterationsExhaustBudget { iterations: usize, budget: usize },
    #[error("planning budget is zero")]
    ZeroPlanningBudget,
    #[error("recon tools require tools_in_planning = none")]
    ReconRequiresUninlinedTools,
    #[error("session-history fixture has no prior-run manifests")]
    EmptySessionHistory,
    #[error("session-history manifests must be sorted most-recent-first")]
    SessionHistoryNotRecentFirst,
    #[error("task {task_id} completed under unknown worker '{worker}'")]
    CompletedTaskUnknownWorker { task_id: usize, worker: String },
    #[error("populated frame fixture has no completed ancestor for task {task_id}")]
    FrameHasNoCompletedAncestor { task_id: usize },
}

// ============================================================================
// Coordinator scenario
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanningBudget(NonZeroUsize);

impl PlanningBudget {
    pub(crate) fn new(max_planning_cycles: usize) -> Result<Self, FixtureError> {
        NonZeroUsize::new(max_planning_cycles)
            .map(Self)
            .ok_or(FixtureError::ZeroPlanningBudget)
    }

    pub(crate) fn get(&self) -> usize {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconTools {
    Included,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryTools {
    Included,
    Excluded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CoordinatorToolConfig {
    pub(crate) recon: ReconTools,
    pub(crate) history: HistoryTools,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionHistoryFixture(Vec<RunManifest>);

impl SessionHistoryFixture {
    pub(crate) fn new(manifests: Vec<RunManifest>) -> Result<Self, FixtureError> {
        if manifests.is_empty() {
            return Err(FixtureError::EmptySessionHistory);
        }
        let recent_first = manifests
            .windows(2)
            .all(|pair| pair[0].timestamp >= pair[1].timestamp);
        if !recent_first {
            return Err(FixtureError::SessionHistoryNotRecentFirst);
        }
        Ok(Self(manifests))
    }

    pub(crate) fn manifests(&self) -> &[RunManifest] {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreambleFixture {
    pub(crate) playbook: String,
    pub(crate) tools: CoordinatorToolConfig,
    pub(crate) skills: Vec<SkillConfig>,
    pub(crate) vector_stores: Vec<VectorStoreConfig>,
    pub(crate) session_history: Option<SessionHistoryFixture>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerRosterFixture {
    config: OrchestrationConfig,
    vector_catalog: Vec<VectorStoreConfig>,
}

impl WorkerRosterFixture {
    pub(crate) fn new(
        config: OrchestrationConfig,
        vector_catalog: Vec<VectorStoreConfig>,
    ) -> Self {
        Self {
            config,
            vector_catalog,
        }
    }

    pub(crate) fn config(&self) -> &OrchestrationConfig {
        &self.config
    }

    pub(crate) fn vector_catalog(&self) -> &[VectorStoreConfig] {
        &self.vector_catalog
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PlanDecision(PlanningResponse);

impl PlanDecision {
    pub(crate) fn new(decision: PlanningResponse) -> Result<Self, FixtureError> {
        match &decision {
            PlanningResponse::Direct { .. } | PlanningResponse::Clarification { .. } => {
                return Err(FixtureError::TerminalDecisionMidThread);
            }
            PlanningResponse::StepsPlan { .. } => {}
        }
        if decision.clone().into_plan().is_none() {
            return Err(FixtureError::UnflattenablePlan);
        }
        Ok(Self(decision))
    }

    pub(crate) fn as_response(&self) -> &PlanningResponse {
        &self.0
    }

    pub(crate) fn plan(&self) -> Plan {
        self.0
            .clone()
            .into_plan()
            .expect("flattenability was validated at construction")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpilledStandIn {
    ClaimEcho { claim: WorkerClaim },
    RawPreview {
        preview: ResultPreview,
        claim: Option<WorkerClaim>,
    },
    NoPreview,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompletedResultFixture {
    Inline {
        result: EvidenceText,
        claim: Option<WorkerClaim>,
    },
    Spilled {
        stand_in: SpilledStandIn,
        artifact: SpilledArtifact,
    },
}

impl CompletedResultFixture {
    pub(crate) fn raw_result(&self) -> String {
        match self {
            Self::Inline { result, .. } => result.as_str().to_owned(),
            Self::Spilled { stand_in, artifact } => {
                let prefix = match stand_in {
                    SpilledStandIn::ClaimEcho { claim } => claim.summary(),
                    SpilledStandIn::RawPreview { preview, .. } => preview.as_str(),
                    SpilledStandIn::NoPreview => "   ",
                };
                format!("{prefix}\n\n{artifact}")
            }
        }
    }

    pub(crate) fn claim(&self) -> Option<&WorkerClaim> {
        match self {
            Self::Inline { claim, .. } => claim.as_ref(),
            Self::Spilled { stand_in, .. } => match stand_in {
                SpilledStandIn::ClaimEcho { claim } => Some(claim),
                SpilledStandIn::RawPreview { claim, .. } => claim.as_ref(),
                SpilledStandIn::NoPreview => None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FailedResultFixture {
    Hard {
        error: String,
        category: FailureCategory,
    },
    Soft {
        claim: WorkerClaim,
        artifact: Option<SpilledArtifact>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum TaskOutcome {
    Complete {
        result: CompletedResultFixture,
        traces: Vec<ToolTraceEntry>,
    },
    Failed {
        report: FailedResultFixture,
        traces: Vec<ToolTraceEntry>,
    },
    Blocked,
}

#[derive(Debug, Clone)]
pub(crate) struct IterationFixture {
    decision: PlanDecision,
    outcomes: Vec<TaskOutcome>,
    failure_summary: Option<FailureSummary>,
}

impl IterationFixture {
    pub(crate) fn new(
        decision: PlanDecision,
        outcomes: Vec<TaskOutcome>,
        failure_summary: Option<FailureSummary>,
    ) -> Result<Self, FixtureError> {
        let tasks = decision.plan().tasks.len();
        if outcomes.len() != tasks {
            return Err(FixtureError::OutcomeCountMismatch {
                tasks,
                outcomes: outcomes.len(),
            });
        }
        let has_failure = outcomes
            .iter()
            .any(|o| matches!(o, TaskOutcome::Failed { .. } | TaskOutcome::Blocked));
        if failure_summary.is_some() && !has_failure {
            return Err(FixtureError::FailureSummaryWithoutFailure);
        }
        Ok(Self {
            decision,
            outcomes,
            failure_summary,
        })
    }

    pub(crate) fn decision(&self) -> &PlanDecision {
        &self.decision
    }

    pub(crate) fn outcomes(&self) -> &[TaskOutcome] {
        &self.outcomes
    }

    pub(crate) fn failure_summary(&self) -> Option<&FailureSummary> {
        self.failure_summary.as_ref()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContinuationThread(Vec<IterationFixture>);

impl ContinuationThread {
    pub(crate) fn new(iterations: Vec<IterationFixture>) -> Result<Self, FixtureError> {
        if iterations.is_empty() {
            return Err(FixtureError::EmptyContinuationThread);
        }
        Ok(Self(iterations))
    }

    pub(crate) fn iterations(&self) -> &[IterationFixture] {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CoordinatorCall {
    Initial,
    Continuation(ContinuationThread),
}

#[derive(Debug, Clone)]
pub(crate) struct CoordinatorScenario {
    preamble: PreambleFixture,
    query: PinnedGoal,
    roster: WorkerRosterFixture,
    budget: PlanningBudget,
    call: CoordinatorCall,
}

impl CoordinatorScenario {
    pub(crate) fn new(
        preamble: PreambleFixture,
        query: PinnedGoal,
        roster: WorkerRosterFixture,
        call: CoordinatorCall,
    ) -> Result<Self, FixtureError> {
        let budget = PlanningBudget::new(roster.config().max_planning_cycles)?;

        if preamble.tools.recon == ReconTools::Included
            && roster.config().tools_in_planning != ToolVisibility::None
        {
            return Err(FixtureError::ReconRequiresUninlinedTools);
        }

        if let CoordinatorCall::Continuation(thread) = &call {
            let iterations = thread.iterations().len();
            if iterations > budget.get() {
                return Err(FixtureError::IterationsExhaustBudget {
                    iterations,
                    budget: budget.get(),
                });
            }
            for iteration in thread.iterations() {
                let plan = iteration.decision().plan();
                for (task, outcome) in plan.tasks.iter().zip(iteration.outcomes()) {
                    if matches!(outcome, TaskOutcome::Complete { .. })
                        && let Some(worker) = task.worker.as_deref()
                        && roster.config().get_worker(worker).is_none()
                    {
                        return Err(FixtureError::CompletedTaskUnknownWorker {
                            task_id: task.id,
                            worker: worker.to_owned(),
                        });
                    }
                }
            }
        }

        Ok(Self {
            preamble,
            query,
            roster,
            budget,
            call,
        })
    }

    pub(crate) fn preamble(&self) -> &PreambleFixture {
        &self.preamble
    }

    pub(crate) fn query(&self) -> &PinnedGoal {
        &self.query
    }

    pub(crate) fn roster(&self) -> &WorkerRosterFixture {
        &self.roster
    }

    pub(crate) fn budget(&self) -> PlanningBudget {
        self.budget
    }

    pub(crate) fn call(&self) -> &CoordinatorCall {
        &self.call
    }
}

// ============================================================================
// Worker scenario
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScratchpadWiring {
    Wired,
    NotWired,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerPreambleAppends {
    pub(crate) scratchpad: ScratchpadWiring,
    pub(crate) skills: Vec<SkillConfig>,
}

#[derive(Debug, Clone)]
pub(crate) enum WorkerPreambleFixture {
    Role {
        role_preamble: String,
        vector_stores: Vec<VectorStoreConfig>,
        appends: WorkerPreambleAppends,
    },
    Generic {
        custom_prompt: Option<String>,
        appends: WorkerPreambleAppends,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct FrameGraph {
    plan: Plan,
    task_id: usize,
}

impl FrameGraph {
    pub(crate) fn new(plan: Plan, task_id: usize) -> Result<Self, FixtureError> {
        if crate::producers::build_task_context(&plan, task_id).is_none() {
            return Err(FixtureError::FrameHasNoCompletedAncestor { task_id });
        }
        Ok(Self { plan, task_id })
    }

    pub(crate) fn plan(&self) -> &Plan {
        &self.plan
    }

    pub(crate) fn task_id(&self) -> usize {
        self.task_id
    }
}

#[derive(Debug, Clone)]
pub(crate) enum WorkerFrameFixture {
    EmptyFirstTurn { task: String },
    EmptyReplanBoundary { task: String },
    Populated(FrameGraph),
}

impl WorkerFrameFixture {
    pub(crate) fn task_text(&self) -> &str {
        match self {
            Self::EmptyFirstTurn { task } | Self::EmptyReplanBoundary { task } => task,
            Self::Populated(graph) => graph
                .plan()
                .tasks
                .iter()
                .find(|t| t.id == graph.task_id())
                .map(|t| t.description.as_str())
                .expect("frame graph target task exists: validated at construction"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkerScenario {
    pub(crate) preamble: WorkerPreambleFixture,
    pub(crate) frame: WorkerFrameFixture,
}
