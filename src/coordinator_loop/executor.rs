//! The seam between the loop and whatever actually runs a plan's tasks.

use agent_driver_rs::tool::ToolContext;
use async_trait::async_trait;

use crate::types::Plan;

use super::observation::ExecutionObservation;

/// Runs a plan and reports what happened.
///
/// The loop treats execution as an ordinary tool call, so the thing behind
/// that tool is a single async seam. Execution reports rather than fails: a
/// plan that could not run comes back as [`ExecutionObservation::Failed`],
/// which the coordinator can replan against, instead of an error that would
/// break the conversation.
///
/// The tool context travels with the call so a real executor can observe
/// cancellation without this signature changing again.
#[async_trait]
pub trait PlanExecutor: Send + Sync {
    /// Run every task in the plan and observe the result.
    async fn execute(&self, plan: &Plan, ctx: &ToolContext) -> ExecutionObservation;
}
