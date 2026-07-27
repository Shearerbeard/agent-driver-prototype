//! The run's in-memory record of what the coordinator created and executed.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};

use crate::types::Plan;

use super::observation::{ExecutionObservation, TaskObservation};
use super::plan_id::PlanId;
use super::tools::CreatePlanArgs;

/// Everything one run has accumulated, in the order it happened.
///
/// Both collections are ordered sequences rather than maps: the run
/// inspection tools and the host-authored fallback both read "the latest",
/// and a hash map's iteration order would make that answer depend on hashing
/// rather than on the run.
#[derive(Debug, Default)]
struct RunRecords {
    plans: Vec<(PlanId, Plan)>,
    executions: Vec<ExecutionObservation>,
}

/// A shared, ordered view of the run that every loop tool reads and writes.
///
/// Cloning shares the record rather than copying it, so the create-plan,
/// execute and inspect tools all see one run. Reads hand back owned copies
/// so no lock guard survives the call, which is what keeps the store usable
/// from inside an async tool body. A poisoned lock is recovered rather than
/// propagated: it means a tool body already panicked and the run is over.
#[derive(Debug, Clone, Default)]
pub struct RunStore(Arc<Mutex<RunRecords>>);

impl RunStore {
    /// Create an empty run record.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a created plan, deriving its handle from the arguments that
    /// created it.
    ///
    /// The store owns the derivation so a caller cannot file a plan under an
    /// id that does not describe it. A plan whose id is already present
    /// keeps its original position: the id is a function of the plan, so a
    /// repeat is the same plan.
    pub fn record_plan(&self, args: &CreatePlanArgs, plan: Plan) -> PlanId {
        let id = PlanId::derive(args);
        let mut records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !records.plans.iter().any(|(known, _)| known == &id) {
            records.plans.push((id.clone(), plan));
        }
        id
    }

    /// The plan a handle names, if this run created it.
    pub fn plan(&self, id: &PlanId) -> Option<Plan> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .plans
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, plan)| plan.clone())
    }

    /// The most recently created plan.
    pub fn latest_plan(&self) -> Option<(PlanId, Plan)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .plans
            .last()
            .cloned()
    }

    /// Every plan handle this run created, in creation order.
    ///
    /// The host-authored fallback lists them when the run ended before
    /// anything was executed.
    pub fn plan_ids(&self) -> Vec<PlanId> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .plans
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Record what an execution observed.
    pub fn record_execution(&self, observation: ExecutionObservation) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .executions
            .push(observation);
    }

    /// The most recent execution observation, the freshest evidence the
    /// fallback can render from.
    pub fn latest_execution(&self) -> Option<ExecutionObservation> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .executions
            .last()
            .cloned()
    }

    /// How many executions this run recorded.
    pub fn execution_count(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .executions
            .len()
    }

    /// Record a per-task execution observation, keyed by plan, task id, and
    /// attempt.
    ///
    /// The S72 run journal backs this with persisted task records; the
    /// skeleton declares the seam so `inspect_run` can address it.
    pub fn record_task(&self, record: TaskRecord) {
        let _ = record;
        todo!("Phase 2: file the task record under (plan_id, task_id, attempt)")
    }

    /// The task record for `(plan_id, task_id, attempt)`, if this run
    /// produced one.
    pub fn task(&self, plan_id: &PlanId, task_id: usize, attempt: Attempt) -> Option<TaskRecord> {
        let _ = (plan_id, task_id, attempt);
        todo!("Phase 2: look up the task record by (plan_id, task_id, attempt)")
    }
}

/// A 1-indexed attempt number for a task execution.
///
/// The first attempt at a task is attempt 1. The newtype prevents zero
/// from reaching the run journal's task records or the `inspect_run`
/// selector.
///
/// Forbidden invalid state: an attempt number of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Attempt(NonZeroUsize);

impl Attempt {
    /// Parse a 1-indexed attempt number.
    ///
    /// # Errors
    ///
    /// Returns [`AttemptZero`] when `attempt` is zero.
    pub fn new(attempt: usize) -> Result<Self, AttemptZero> {
        NonZeroUsize::new(attempt)
            .map(Self)
            .ok_or(AttemptZero)
    }

    /// The 1-indexed attempt number.
    pub fn get(&self) -> usize {
        self.0.get()
    }
}

impl std::fmt::Display for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl serde::Serialize for Attempt {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.get().serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for Attempt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = usize::deserialize(d)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

/// Why an attempt number was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("attempt number is zero; attempts are 1-indexed")]
pub struct AttemptZero;

/// A per-task execution record, keyed by plan, task id, and attempt together.
///
/// The attempt is 1-indexed: the first attempt at a task is attempt 1. The
/// key is the triple `(plan_id, task_id, attempt)` because a revised plan
/// restarts task ids at zero, so `(task_id, attempt)` alone is not
/// run-unique.
///
/// The fields are private and the [`new`](Self::new) constructor derives
/// `task_id` from the observation's correlation label, so a record cannot
/// carry a `task_id` that disagrees with the observation it files.
///
/// Forbidden invalid state: a task record whose observation is a blocked
/// task carrying a failure category, or a completed task carrying an error
/// preview — the invariants live on [`TaskObservation`], not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    plan_id: PlanId,
    task_id: usize,
    attempt: Attempt,
    observation: TaskObservation,
}

impl TaskRecord {
    /// Construct a task record, deriving `task_id` from the observation's
    /// correlation label.
    pub fn new(plan_id: PlanId, attempt: Attempt, observation: TaskObservation) -> Self {
        Self {
            task_id: observation.label().task.get(),
            plan_id,
            attempt,
            observation,
        }
    }

    /// The plan this record belongs to.
    pub fn plan_id(&self) -> &PlanId {
        &self.plan_id
    }

    /// The task this record observes.
    pub fn task_id(&self) -> usize {
        self.task_id
    }

    /// The 1-indexed attempt number.
    pub fn attempt(&self) -> Attempt {
        self.attempt
    }

    /// The observation the attempt produced.
    pub fn observation(&self) -> &TaskObservation {
        &self.observation
    }
}
