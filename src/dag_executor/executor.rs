//! The `PlanExecutor` implementation the coordinator loop ships with.

use std::collections::VecDeque;
use std::sync::Arc;

use agent_driver_rs::tool::ToolContext;
use agent_driver_rs::SystemPrompt;
use async_trait::async_trait;

use crate::artifacts::{ArtifactFilename, ArtifactStore, InlineThreshold, SpilledBody};
use crate::bounding::ErrorPreviewWidth;
use crate::context::{
    ArtifactRef, CorrelationLabel, ErrorPreview, EvidenceEntry, TaskId, WorkerRole,
};
use crate::coordinator_loop::{
    Attempt, ExecutionObservation, PlanExecutor, RunStore, TaskObservation, TaskRecord,
  TerminalSlot, WorkerSections, WorkerSubmission,
};
use crate::mcp_client::SidecarClient;
use crate::types::{FailureCategory, Plan, Task, TaskState};

use super::worker::{WorkerLoop, WorkerLoopConfig, WorkerOutcome};

/// The real DAG executor.
///
/// Runs a plan by selecting ready tasks, dispatching each to a worker inner
/// loop on four tools, and propagating dependency failure to descendants.
/// Reports an [`ExecutionObservation`] rather than raising, so the
/// coordinator can replan against a failed execution.
///
/// Forbidden invalid state: an executor without a sidecar client or
/// artifact store, which would leave worker tools with no terminal to drive
/// and no artifact channel to spill through.
pub struct DagExecutor {
    sidecar: SidecarClient,
    artifacts: ArtifactStore,
    worker_config: WorkerLoopConfig,
    worker_sections: WorkerSections,
    runs: RunStore,
    inline_threshold: InlineThreshold,
}

impl DagExecutor {
    /// Assemble an executor from its dependencies.
    ///
    /// The `sidecar` is the connected MCP client; `artifacts` is the
    /// filename-addressed store; `worker_config` carries the provider,
    /// model, and budget for worker inner loops; `worker_sections` is the
    /// roster the executor reads worker preambles from; `runs` is the run
    /// store the executor files per-task records into; `inline_threshold`
    /// is the character bound at which worker results spill to artifacts.
    ///
    /// The `ExecuteTool` constructs the executor per dispatch from the
    /// [`RunStore`] it already owns, so the executor sees the run's current
    /// plan and task records. The executor allocates 1-indexed attempt
    /// numbers per task: the first attempt at a task is attempt 1, a retry
    /// is attempt 2, and so on.
    pub fn new(
        sidecar: SidecarClient,
        artifacts: ArtifactStore,
        worker_config: WorkerLoopConfig,
        worker_sections: WorkerSections,
        runs: RunStore,
        inline_threshold: InlineThreshold,
    ) -> Self {
        Self {
            sidecar,
            artifacts,
            worker_config,
            worker_sections,
            runs,
            inline_threshold,
        }
    }

    /// Resolve the system prompt for a task's assigned worker (R4).
    ///
    /// If the task has a worker assigned and the roster carries a non-empty
    /// preamble for that worker, use it. Otherwise fall back to the default
    /// system prompt from [`WorkerLoopConfig`].
    fn resolve_preamble(&self, task: &Task) -> SystemPrompt {
        if let Some(worker_name) = &task.worker
            && let Some(spec) = self
                .worker_sections
                .roster()
                .workers()
                .iter()
                .find(|spec| spec.role().as_str() == worker_name)
        {
            let preamble = spec.preamble();
            if !preamble.is_empty() {
                return SystemPrompt::new(preamble);
            }
        }
        self.worker_config.system_prompt.clone()
    }

    fn correlation_label(task: &Task) -> CorrelationLabel {
        CorrelationLabel {
            task: TaskId::new(task.id),
            worker: task
                .worker
                .as_deref()
                .and_then(|name| WorkerRole::new(name).ok()),
        }
    }

    fn execution_failed(
        message: &str,
        tasks_observed: Vec<TaskObservation>,
    ) -> ExecutionObservation {
        ExecutionObservation::Failed {
            category: FailureCategory::AgentError,
            message: ErrorPreview::new(message, ErrorPreviewWidth::DEFAULT),
            tasks_observed,
        }
    }
}

#[async_trait]
impl PlanExecutor for DagExecutor {
    async fn execute(&self, plan: &Plan, _ctx: &ToolContext) -> ExecutionObservation {
        let mut work_plan = plan.clone();
        let plan_id = match self.runs.latest_plan() {
            Some((id, _)) => id,
            None => {
                return Self::execution_failed(
                    "no plan was created before execution",
                    Vec::new(),
                )
            }
        };

        let attempt = Attempt::new(1).expect("1 is non-zero");
        let mut observations: Vec<Option<TaskObservation>> =
            vec![None; work_plan.tasks.len()];

        // Task ids are not assumed to be contiguous or ordered; build a
        // lookup once so the dispatch loop is O(n) overall, not O(n^2).
        let task_index: std::collections::HashMap<usize, usize> = work_plan
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id, index))
            .collect();

        while !work_plan.is_finished() {
            let ready = ready_tasks(&work_plan);
            if ready.is_empty() {
                break;
            }

            for task_id in ready {
                let index = *task_index.get(&task_id).expect("ready task id exists in plan");
                work_plan.tasks[index].start();

                let config = WorkerLoopConfig {
                    provider: Arc::clone(&self.worker_config.provider),
                    model: self.worker_config.model.clone(),
                    budget: self.worker_config.budget,
                    system_prompt: self.resolve_preamble(&work_plan.tasks[index]),
                };

                let slot: TerminalSlot<WorkerSubmission> = TerminalSlot::new();
                let worker = WorkerLoop::new(
                    config,
                    self.sidecar.clone(),
                    self.artifacts.clone(),
                );
                let outcome = worker
                    .run_task(&work_plan.tasks[index], slot.clone())
                    .await;

                let label = Self::correlation_label(&work_plan.tasks[index]);
                let (observation, new_state) = self
                    .map_outcome(&outcome, &label, &work_plan.tasks[index])
                    .await;
                observations[index] = Some(observation.clone());

                let record = TaskRecord::new(plan_id.clone(), attempt, observation);
                self.runs.record_task(record);

                work_plan.tasks[index].state = new_state;

                if matches!(work_plan.tasks[index].state, TaskState::Failed { .. }) {
                    fail_descendants_of(&mut work_plan, task_id);
                }
            }
        }

        let mut final_observations = Vec::with_capacity(work_plan.tasks.len());
        for (i, task) in work_plan.tasks.iter().enumerate() {
            match &observations[i] {
                Some(obs) => final_observations.push(obs.clone()),
                None => {
                    let label = Self::correlation_label(task);
                    let blocked = TaskObservation::Blocked { label };
                    let record =
                        TaskRecord::new(plan_id.clone(), attempt, blocked.clone());
                    self.runs.record_task(record);
                    final_observations.push(blocked);
                }
            }
        }

        match ExecutionObservation::completed(final_observations) {
            Ok(obs) => obs,
            Err(error) => Self::execution_failed(&error.to_string(), Vec::new()),
        }
    }
}

impl DagExecutor {
    /// Map a [`WorkerOutcome`] to a [`TaskObservation`] and the plan's next
    /// [`TaskState`], spilling the result body when it exceeds the inline
    /// threshold.
    async fn map_outcome(
        &self,
        outcome: &WorkerOutcome,
        label: &CorrelationLabel,
        task: &Task,
    ) -> (TaskObservation, TaskState) {
        match outcome {
            WorkerOutcome::Submitted(submission) => {
                let result_text = submission.result().as_str();
                let claim = submission.claim().clone();

                if self.inline_threshold.allows_inline(result_text) {
                    let evidence =
                        EvidenceEntry::from_completed_result(result_text, Some(claim)).expect(
                            "worker submission guarantees a non-blank result; \
                             inline text that fits the threshold carries no spill footer",
                        );
                    let observation = TaskObservation::Completed {
                        label: label.clone(),
                        evidence,
                        artifacts: Vec::new(),
                    };
                    let state = TaskState::Complete {
                        result: result_text.to_owned(),
                    };
                    (observation, state)
                } else {
                    let worker_name = task.worker.as_deref().unwrap_or("default");
                    let filename_str =
                        format!("task-{}-{}-result.txt", task.id, worker_name);
                    let spill_result = match ArtifactFilename::new(&filename_str) {
                        Ok(filename) => {
                            self.artifacts
                                .write_artifact(&filename, result_text)
                                .await
                                .map(|_| filename)
                        }
                        Err(error) => Err(error),
                    };

                    match spill_result {
                        Ok(filename) => {
                            let spilled_body = SpilledBody::new(
                                filename.clone(),
                                result_text.chars().count(),
                            );
                            let spilled_text =
                                format!("{}\n\n{spilled_body}", claim.summary());
                            let evidence = EvidenceEntry::from_completed_result(
                                &spilled_text,
                                Some(claim),
                            )
                            .expect(
                                "spilled text carries a footer with a non-blank claim \
                                 summary prefix; from_completed_result takes the \
                                 ArtifactPointer path which is infallible with a claim",
                            );
                            let artifact_ref = ArtifactRef::new(
                                filename.as_str(),
                                result_text.len() as u64,
                            )
                            .expect("filename validated by ArtifactFilename::new");
                            let observation = TaskObservation::Completed {
                                label: label.clone(),
                                evidence,
                                artifacts: vec![artifact_ref],
                            };
                            let state = TaskState::Complete {
                                result: spilled_text,
                            };
                            (observation, state)
                        }
                        Err(error) => {
                            let message = format!(
                                "result body exceeded the inline bound and the \
                                 artifact write failed: {error}"
                            );
                            let category = FailureCategory::AgentError;
                            let observation = TaskObservation::Failed {
                                label: label.clone(),
                                category,
                                error: ErrorPreview::new(
                                    &message,
                                    ErrorPreviewWidth::DEFAULT,
                                ),
                                artifacts: Vec::new(),
                            };
                            let state = TaskState::Failed {
                                error: message,
                                category,
                            };
                            (observation, state)
                        }
                    }
                }
            }
            WorkerOutcome::StoppedWithoutSubmission => {
                let category = FailureCategory::DepthExhausted;
                let observation = TaskObservation::Failed {
                    label: label.clone(),
                    category,
                    error: ErrorPreview::new(
                        "worker stopped without submitting a result",
                        ErrorPreviewWidth::DEFAULT,
                    ),
                    artifacts: Vec::new(),
                };
                let state = TaskState::Failed {
                    error: "worker stopped without submitting a result".to_owned(),
                    category,
                };
                (observation, state)
            }
            WorkerOutcome::BudgetExhausted => {
                let category = FailureCategory::DepthExhausted;
                let observation = TaskObservation::Failed {
                    label: label.clone(),
                    category,
                    error: ErrorPreview::new(
                        "worker exhausted its turn budget without submitting",
                        ErrorPreviewWidth::DEFAULT,
                    ),
                    artifacts: Vec::new(),
                };
                let state = TaskState::Failed {
                    error: "worker exhausted its turn budget without submitting".to_owned(),
                    category,
                };
                (observation, state)
            }
            WorkerOutcome::Interrupted(reason) => {
                let category = match reason {
                    crate::coordinator_loop::InterruptionReason::TokenLimit => {
                        FailureCategory::AgentTimeout
                    }
                    _ => FailureCategory::AgentError,
                };
                let message = reason.to_string();
                let observation = TaskObservation::Failed {
                    label: label.clone(),
                    category,
                    error: ErrorPreview::new(&message, ErrorPreviewWidth::DEFAULT),
                    artifacts: Vec::new(),
                };
                let state = TaskState::Failed {
                    error: message,
                    category,
                };
                (observation, state)
            }
            WorkerOutcome::Failed(category) => {
                let message = format!("worker failed: {category}");
                let observation = TaskObservation::Failed {
                    label: label.clone(),
                    category: *category,
                    error: ErrorPreview::new(&message, ErrorPreviewWidth::DEFAULT),
                    artifacts: Vec::new(),
                };
                let state = TaskState::Failed {
                    error: message,
                    category: *category,
                };
                (observation, state)
            }
        }
    }
}

/// Task ids that are ready to execute: Pending with all dependencies
/// Complete. Mirrors aura's `Plan::ready_tasks`.
fn ready_tasks(plan: &Plan) -> Vec<usize> {
    plan.tasks
        .iter()
        .filter(|task| {
            if !matches!(task.state, TaskState::Pending) {
                return false;
            }
            for dep_id in &task.dependencies {
                let dep = plan.tasks.iter().find(|t| t.id == *dep_id);
                match dep.map(|t| &t.state) {
                    Some(TaskState::Complete { .. }) => continue,
                    _ => return false,
                }
            }
            true
        })
        .map(|task| task.id)
        .collect()
}

/// Mark all transitive pending descendants of `failed_task_id` as Failed
/// with [`FailureCategory::DependencyFailed`]. Complete, Running, and
/// already-Failed descendants are skipped. Mirrors aura's
/// `Orchestrator::fail_descendants_of`.
fn fail_descendants_of(plan: &mut Plan, failed_task_id: usize) {
    let mut queue = VecDeque::new();
    queue.push_back(failed_task_id);
    while let Some(current_id) = queue.pop_front() {
        for task in plan.tasks.iter_mut() {
            if task.dependencies.contains(&current_id)
                && matches!(task.state, TaskState::Pending)
            {
                task.fail(
                    format!("ancestor task {failed_task_id} failed"),
                    FailureCategory::DependencyFailed,
                );
                queue.push_back(task.id);
            }
        }
    }
}

/// A shared handle to a `DagExecutor`, suitable for `CoordinatorLoopConfig`.
///
/// `CoordinatorLoopConfig` holds `Arc<dyn PlanExecutor>`, so the executor
/// is cheaply cloneable as an `Arc` without exposing the concrete type.
impl From<DagExecutor> for Arc<dyn PlanExecutor> {
    fn from(executor: DagExecutor) -> Self {
        Arc::new(executor)
    }
}
