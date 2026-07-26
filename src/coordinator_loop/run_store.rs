//! The run's in-memory record of what the coordinator created and executed.

use std::sync::{Arc, Mutex};

use crate::types::Plan;

use super::observation::ExecutionObservation;
use super::plan_id::PlanId;

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
/// from inside an async tool body.
#[derive(Debug, Clone, Default)]
pub struct RunStore(Arc<Mutex<RunRecords>>);

impl RunStore {
    /// Create an empty run record.
    pub fn new() -> Self {
        todo!("S71 Phase 2")
    }

    /// Record a created plan under its derived id.
    ///
    /// A plan whose id is already present keeps its original position: the
    /// id is a function of the plan, so a repeat is the same plan.
    pub fn record_plan(&self, _id: PlanId, _plan: Plan) {
        todo!("S71 Phase 2")
    }

    /// The plan a handle names, if this run created it.
    pub fn plan(&self, _id: &PlanId) -> Option<Plan> {
        todo!("S71 Phase 2")
    }

    /// The most recently created plan.
    pub fn latest_plan(&self) -> Option<(PlanId, Plan)> {
        todo!("S71 Phase 2")
    }

    /// Every plan handle this run created, in creation order.
    ///
    /// The host-authored fallback lists them when the run ended before
    /// anything was executed.
    pub fn plan_ids(&self) -> Vec<PlanId> {
        todo!("S71 Phase 2")
    }

    /// Record what an execution observed.
    pub fn record_execution(&self, _observation: ExecutionObservation) {
        todo!("S71 Phase 2")
    }

    /// The most recent execution observation, the freshest evidence the
    /// fallback can render from.
    pub fn latest_execution(&self) -> Option<ExecutionObservation> {
        todo!("S71 Phase 2")
    }
}
