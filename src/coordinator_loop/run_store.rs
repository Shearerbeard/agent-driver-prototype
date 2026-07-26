//! The run's in-memory record of what the coordinator created and executed.

use std::sync::{Arc, Mutex, PoisonError};

use crate::types::Plan;

use super::observation::ExecutionObservation;
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
}
