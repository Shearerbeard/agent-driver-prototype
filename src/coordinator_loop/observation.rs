//! What a nonterminal loop tool reports back into the conversation.
//!
//! Observations are the loop's return channel. The substrate carries tool
//! results as strings, so every type here serializes to JSON on the way out;
//! the structured form is what the host reasons about, the JSON is what the
//! model reads.

use serde::{Serialize, Serializer};

use crate::context::{ArtifactRef, CorrelationLabel, ErrorPreview, EvidenceText, PlanShape};
use crate::tools::submit_result::Confidence;
use crate::types::{FailureCategory, TaskStatus};

use super::plan_id::PlanId;

/// What the coordinator learns when a plan is created.
///
/// Carries the handle and the plan's shape, never the task bodies: the
/// bodies reach the workers that execute them and stay retrievable through
/// run inspection, so the conversation holds one copy rather than two. Task
/// count is read off the shape instead of stored beside it, so the count and
/// the assignment list cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanObservation {
    plan_id: PlanId,
    shape: PlanShape,
}

impl PlanObservation {
    /// Report a created plan.
    pub fn new(_plan_id: PlanId, _shape: PlanShape) -> Self {
        todo!("S71 Phase 2")
    }

    /// The handle that names this plan in later tool calls.
    pub fn plan_id(&self) -> &PlanId {
        todo!("S71 Phase 2")
    }

    /// Per-task worker assignments, in plan order.
    pub fn shape(&self) -> &PlanShape {
        todo!("S71 Phase 2")
    }

    /// How many tasks the plan flattened into.
    pub fn task_count(&self) -> usize {
        todo!("S71 Phase 2")
    }
}

impl Serialize for PlanObservation {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        todo!("S71 Phase 2")
    }
}

/// One task's contribution to an execution observation.
///
/// The three cases are the three the continuation frame already
/// distinguishes: a task that produced evidence, a task that failed with a
/// category, and a task that never ran. Splitting them by variant is what
/// keeps a blocked task from carrying a confidence rating, or a completed
/// task from carrying a failure category.
///
/// Every case is keyed by a correlation label — task id plus assigned worker
/// — never by the coordinator's own imperative task text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskObservation {
    /// The worker finished and reported evidence.
    Completed {
        label: CorrelationLabel,
        evidence: EvidenceText,
        artifacts: Vec<ArtifactRef>,
        /// Present only when the worker attested a confidence level.
        confidence: Option<Confidence>,
    },
    /// The worker failed; the category is what the coordinator replans on.
    Failed {
        label: CorrelationLabel,
        category: FailureCategory,
        error: ErrorPreview,
        artifacts: Vec<ArtifactRef>,
    },
    /// The task never ran because an upstream dependency did not complete.
    Blocked { label: CorrelationLabel },
}

impl TaskObservation {
    /// The correlation label this observation is keyed by.
    pub fn label(&self) -> &CorrelationLabel {
        todo!("S71 Phase 2")
    }

    /// The ported task status this observation reports on the wire. A
    /// blocked task reports as pending: it is still waiting, not finished.
    pub fn status(&self) -> TaskStatus {
        todo!("S71 Phase 2")
    }
}

impl Serialize for TaskObservation {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        todo!("S71 Phase 2")
    }
}

/// The tally of task outcomes in one execution.
///
/// Derived from the task observations rather than accumulated alongside
/// them, so the summary the coordinator reads can never disagree with the
/// per-task detail underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OutcomeCounts {
    completed: usize,
    failed: usize,
    blocked: usize,
    soft_failed: usize,
}

impl OutcomeCounts {
    /// Count the outcomes in a task list.
    pub fn tally(_tasks: &[TaskObservation]) -> Self {
        todo!("S71 Phase 2")
    }

    /// Tasks that finished and reported evidence.
    pub fn completed(&self) -> usize {
        todo!("S71 Phase 2")
    }

    /// Tasks that failed, including soft failures.
    pub fn failed(&self) -> usize {
        todo!("S71 Phase 2")
    }

    /// Tasks that never ran because an upstream dependency did not complete.
    pub fn blocked(&self) -> usize {
        todo!("S71 Phase 2")
    }

    /// Failed tasks whose category is a worker-reported soft failure, the
    /// subset a replan can usually address.
    pub fn soft_failed(&self) -> usize {
        todo!("S71 Phase 2")
    }
}

/// What the coordinator learns when a plan is executed.
///
/// An enum because a failure that happens before any task is dispatched
/// leaves no per-task record to report, which a struct with a task list
/// could only express as an empty list indistinguishable from a plan of
/// zero tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionObservation {
    /// Every task in the plan reached a terminal state; individual tasks may
    /// still have failed.
    Completed { tasks: Vec<TaskObservation> },
    /// Execution itself failed, before or across task dispatch. The task
    /// list holds whatever finished first and may be empty.
    Failed {
        category: FailureCategory,
        message: String,
        tasks_completed: Vec<TaskObservation>,
    },
}

impl ExecutionObservation {
    /// The tasks this execution produced observations for.
    pub fn tasks(&self) -> &[TaskObservation] {
        todo!("S71 Phase 2")
    }

    /// The outcome tally over [`tasks`](Self::tasks).
    pub fn counts(&self) -> OutcomeCounts {
        todo!("S71 Phase 2")
    }
}

impl Serialize for ExecutionObservation {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        todo!("S71 Phase 2")
    }
}
