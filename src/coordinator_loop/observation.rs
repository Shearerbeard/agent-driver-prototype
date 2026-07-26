//! What a nonterminal loop tool reports back into the conversation.
//!
//! Observations are the loop's return channel. The substrate carries tool
//! results as strings, so every type here serializes to JSON on the way out;
//! the structured form is what the host reasons about, the JSON is what the
//! model reads. Serialization is per-status: a field that does not apply to
//! the reported status is absent, never present and null.

use serde::ser::{SerializeMap, SerializeSeq as _};
use serde::{Serialize, Serializer};

use crate::context::{ArtifactRef, CorrelationLabel, ErrorPreview, EvidenceEntry, PlanShape};
use crate::types::{FailureCategory, Plan, TaskStatus};

use super::error::CoordinatorLoopError;
use super::plan_id::PlanId;

/// What the coordinator learns when a plan is created.
///
/// Carries the handle and the plan's shape, never the task bodies: the
/// bodies reach the workers that execute them and stay retrievable through
/// run inspection, so the conversation holds one copy rather than two. Task
/// count is read off the shape instead of stored beside it, so the count and
/// the assignment list cannot disagree.
///
/// Serializes as `{"plan_id", "task_count", "shape"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanObservation {
    plan_id: PlanId,
    shape: PlanShape,
}

impl PlanObservation {
    /// Report a created plan.
    pub fn new(plan_id: PlanId, shape: PlanShape) -> Self {
        Self { plan_id, shape }
    }

    /// Report a created plan by reading its shape off the plan itself.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::UnusableObservation`] when the plan
    /// has no tasks to describe.
    pub fn from_plan(plan_id: PlanId, plan: &Plan) -> Result<Self, CoordinatorLoopError> {
        let assignments = plan
            .tasks
            .iter()
            .map(|task| {
                task.worker
                    .as_deref()
                    .and_then(|name| crate::context::WorkerRole::new(name).ok())
            })
            .collect();
        let shape =
            PlanShape::new(assignments).map_err(CoordinatorLoopError::UnusableObservation)?;
        Ok(Self::new(plan_id, shape))
    }

    /// The handle that names this plan in later tool calls.
    pub fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    /// Per-task worker assignments, in plan order.
    pub fn shape(&self) -> &PlanShape {
        &self.shape
    }

    /// How many tasks the plan flattened into.
    pub fn task_count(&self) -> usize {
        self.shape.assignments().len()
    }
}

impl Serialize for PlanObservation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("plan_id", self.plan_id.as_str())?;
        map.serialize_entry("task_count", &self.task_count())?;
        map.serialize_entry("shape", &self.shape.to_string())?;
        map.end()
    }
}

/// One task's contribution to an execution observation.
///
/// The three cases are the three the continuation frame already
/// distinguishes: a task that produced evidence, a task that failed with a
/// category, and a task that never ran. Splitting them by variant is what
/// keeps a blocked task from carrying a failure category, or a completed
/// task from carrying an error preview.
///
/// Every case is keyed by a correlation label, task id plus assigned worker,
/// never by the coordinator's own imperative task text. Completed work
/// carries the ported evidence entry rather than a bare text slot, so the
/// worker's attested summary and its result body stay one value and a
/// confidence rating cannot exist without the summary it rates.
///
/// Serializes with `task_id` and `status` always present, `worker` when the
/// task was assigned, `artifacts` when the task produced any, `summary` and
/// `confidence` when the worker attested a claim, and `evidence` or
/// `failure_category` plus `error` according to status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskObservation {
    /// The worker finished and reported evidence.
    Completed {
        label: CorrelationLabel,
        evidence: EvidenceEntry,
        artifacts: Vec<ArtifactRef>,
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
        match self {
            Self::Completed { label, .. }
            | Self::Failed { label, .. }
            | Self::Blocked { label } => label,
        }
    }

    /// The ported task status this observation reports on the wire. A
    /// blocked task reports as pending: it is still waiting, not finished.
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Completed { .. } => TaskStatus::Complete,
            Self::Failed { .. } => TaskStatus::Failed,
            Self::Blocked { .. } => TaskStatus::Pending,
        }
    }

    /// The failure category, for a task that failed.
    pub fn failure_category(&self) -> Option<FailureCategory> {
        match self {
            Self::Failed { category, .. } => Some(*category),
            Self::Completed { .. } | Self::Blocked { .. } => None,
        }
    }
}

impl Serialize for TaskObservation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        let label = self.label();
        map.serialize_entry("task_id", &label.task.get())?;
        if let Some(worker) = &label.worker {
            map.serialize_entry("worker", worker.as_str())?;
        }
        map.serialize_entry("status", &self.status())?;
        match self {
            Self::Completed {
                evidence,
                artifacts,
                ..
            } => {
                map.serialize_entry("evidence", &evidence.render_body())?;
                if let Some(claim) = evidence.claim() {
                    map.serialize_entry("summary", claim.summary())?;
                    map.serialize_entry("confidence", &claim.confidence())?;
                }
                serialize_artifacts(&mut map, artifacts)?;
            }
            Self::Failed {
                category,
                error,
                artifacts,
                ..
            } => {
                map.serialize_entry("failure_category", category)?;
                map.serialize_entry("error", &error.to_string())?;
                serialize_artifacts(&mut map, artifacts)?;
            }
            Self::Blocked { .. } => {}
        }
        map.end()
    }
}

/// Emit the artifact inventory, omitting the key when the task produced no
/// artifacts.
fn serialize_artifacts<M: SerializeMap>(
    map: &mut M,
    artifacts: &[ArtifactRef],
) -> Result<(), M::Error> {
    if artifacts.is_empty() {
        return Ok(());
    }
    map.serialize_entry("artifacts", &ArtifactInventory(artifacts))
}

/// The artifact inventory as `[{"filename", "bytes"}]`.
struct ArtifactInventory<'a>(&'a [ArtifactRef]);

impl Serialize for ArtifactInventory<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for artifact in self.0 {
            seq.serialize_element(&ArtifactEntry(artifact))?;
        }
        seq.end()
    }
}

struct ArtifactEntry<'a>(&'a ArtifactRef);

impl Serialize for ArtifactEntry<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("filename", self.0.filename())?;
        map.serialize_entry("bytes", &self.0.bytes())?;
        map.end()
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
    pub fn tally(tasks: &[TaskObservation]) -> Self {
        let mut counts = Self {
            completed: 0,
            failed: 0,
            blocked: 0,
            soft_failed: 0,
        };
        for task in tasks {
            match task {
                TaskObservation::Completed { .. } => counts.completed += 1,
                TaskObservation::Failed { category, .. } => {
                    counts.failed += 1;
                    if matches!(category, FailureCategory::SoftFailure) {
                        counts.soft_failed += 1;
                    }
                }
                TaskObservation::Blocked { .. } => counts.blocked += 1,
            }
        }
        counts
    }

    /// Tasks that finished and reported evidence.
    pub fn completed(&self) -> usize {
        self.completed
    }

    /// Tasks that failed, including soft failures.
    pub fn failed(&self) -> usize {
        self.failed
    }

    /// Tasks that never ran because an upstream dependency did not complete.
    pub fn blocked(&self) -> usize {
        self.blocked
    }

    /// Failed tasks whose category is a worker-reported soft failure, the
    /// subset a replan can usually address.
    pub fn soft_failed(&self) -> usize {
        self.soft_failed
    }
}

/// A completed execution's task list, non-empty by construction.
///
/// An execution that ran a plan to completion observed at least one task,
/// because a plan with no tasks never parsed. The newtype is what makes the
/// empty case unconstructable rather than merely unexpected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskObservations(Vec<TaskObservation>);

impl TaskObservations {
    /// Parse a completed execution's task list.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::EmptyTaskObservations`] when the list
    /// is empty.
    pub fn new(tasks: Vec<TaskObservation>) -> Result<Self, CoordinatorLoopError> {
        if tasks.is_empty() {
            return Err(CoordinatorLoopError::EmptyTaskObservations);
        }
        Ok(Self(tasks))
    }

    /// The observations, in plan order.
    pub fn as_slice(&self) -> &[TaskObservation] {
        &self.0
    }
}

/// What the coordinator learns when a plan is executed.
///
/// An enum because a failure that happens before any task is dispatched
/// leaves no per-task record to report, which a struct with a task list
/// could only express as an empty list indistinguishable from a plan of
/// zero tasks.
///
/// Serializes with `status` and `outcome` always present, `tasks` on the
/// completed status, and `failure_category`, `message` and `tasks_observed`
/// on the failed status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionObservation {
    /// Every task in the plan reached a terminal state; individual tasks may
    /// still have failed.
    Completed { tasks: TaskObservations },
    /// Execution itself failed, before or across task dispatch. The task
    /// list holds whatever finished first and may be empty.
    Failed {
        category: FailureCategory,
        message: ErrorPreview,
        tasks_observed: Vec<TaskObservation>,
    },
}

impl ExecutionObservation {
    /// Report a plan that ran to completion.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::EmptyTaskObservations`] when no task
    /// was observed.
    pub fn completed(tasks: Vec<TaskObservation>) -> Result<Self, CoordinatorLoopError> {
        Ok(Self::Completed {
            tasks: TaskObservations::new(tasks)?,
        })
    }

    /// The tasks this execution produced observations for.
    pub fn tasks(&self) -> &[TaskObservation] {
        match self {
            Self::Completed { tasks } => tasks.as_slice(),
            Self::Failed { tasks_observed, .. } => tasks_observed,
        }
    }

    /// The outcome tally over [`tasks`](Self::tasks).
    pub fn counts(&self) -> OutcomeCounts {
        OutcomeCounts::tally(self.tasks())
    }
}

impl Serialize for ExecutionObservation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Completed { tasks } => {
                map.serialize_entry("status", "completed")?;
                map.serialize_entry("outcome", &self.counts())?;
                map.serialize_entry("tasks", tasks.as_slice())?;
            }
            Self::Failed {
                category,
                message,
                tasks_observed,
            } => {
                map.serialize_entry("status", "failed")?;
                map.serialize_entry("failure_category", category)?;
                map.serialize_entry("message", &message.to_string())?;
                map.serialize_entry("outcome", &self.counts())?;
                map.serialize_entry("tasks_observed", tasks_observed)?;
            }
        }
        map.end()
    }
}
