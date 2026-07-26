//! The seam between the loop and whatever actually runs a plan's tasks.

use async_trait::async_trait;

use crate::types::Plan;

use super::observation::ExecutionObservation;

/// Runs a plan and reports what happened.
///
/// The loop treats execution as an ordinary tool call, so the thing behind
/// that tool is a single async seam. Execution reports rather than fails:
/// a plan that could not run comes back as
/// [`ExecutionObservation::Failed`], which the coordinator can replan
/// against, instead of an error that would break the conversation.
#[async_trait]
pub trait PlanExecutor: Send + Sync {
    /// Run every task in the plan and observe the result.
    async fn execute(&self, plan: &Plan) -> ExecutionObservation;
}

/// An executor that completes every task with canned evidence.
///
/// It exists so the loop's control flow can be exercised end to end before
/// the DAG core is wired in: the acceptance tests need a `create_plan ->
/// execute -> continue` round trip, not real worker output. Public rather
/// than test-only because it is the executor the loop ships with today —
/// gating it behind `cfg(test)` would leave `CoordinatorLoop` unconstructable
/// in a normal build.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubExecutor;

#[async_trait]
impl PlanExecutor for StubExecutor {
    async fn execute(&self, _plan: &Plan) -> ExecutionObservation {
        todo!("S71 Phase 2")
    }
}
