//! The seam between the loop and whatever actually runs a plan's tasks.

use agent_driver_rs::tool::ToolContext;
use async_trait::async_trait;

use crate::bounding::ErrorPreviewWidth;
use crate::context::{CorrelationLabel, EvidenceEntry, TaskId, WorkerRole};
use crate::types::{FailureCategory, Plan};

use super::observation::{ExecutionObservation, TaskObservation};

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

/// An executor that completes every task with canned evidence.
///
/// It exists so the loop's control flow can be exercised end to end before
/// the DAG core is wired in: the acceptance tests need a `create_plan ->
/// execute -> continue` round trip, not real worker output. Public rather
/// than test-only because it is the executor the loop ships with today;
/// gating it behind `cfg(test)` would leave `CoordinatorLoop`
/// unconstructable in a normal build.
///
/// The evidence it writes names the task id and nothing else. Echoing the
/// task description back would put coordinator-authored instruction text
/// where worker evidence belongs.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubExecutor;

#[async_trait]
impl PlanExecutor for StubExecutor {
    async fn execute(&self, plan: &Plan, _ctx: &ToolContext) -> ExecutionObservation {
        let mut tasks = Vec::with_capacity(plan.tasks.len());
        for task in &plan.tasks {
            let label = CorrelationLabel {
                task: TaskId::new(task.id),
                worker: task
                    .worker
                    .as_deref()
                    .and_then(|name| WorkerRole::new(name).ok()),
            };
            let evidence = match EvidenceEntry::from_completed_result(&stub_evidence(task.id), None)
            {
                Ok(evidence) => evidence,
                Err(error) => return stub_failure(&error.to_string(), tasks),
            };
            tasks.push(TaskObservation::Completed {
                label,
                evidence,
                artifacts: Vec::new(),
            });
        }

        match ExecutionObservation::completed(tasks) {
            Ok(observation) => observation,
            // A plan with no tasks never parsed, so this reports a plan that
            // reached execution by some path other than the create-plan
            // tool rather than a state the model produced.
            Err(error) => stub_failure(&error.to_string(), Vec::new()),
        }
    }
}

/// The canned evidence body for one stub-executed task.
fn stub_evidence(task_id: usize) -> String {
    format!("Task {task_id} completed. Stub executor; no worker ran.")
}

/// Report a stub execution that could not produce a completed observation.
fn stub_failure(message: &str, tasks_observed: Vec<TaskObservation>) -> ExecutionObservation {
    ExecutionObservation::Failed {
        category: FailureCategory::AgentError,
        message: crate::context::ErrorPreview::new(message, ErrorPreviewWidth::DEFAULT),
        tasks_observed,
    }
}
