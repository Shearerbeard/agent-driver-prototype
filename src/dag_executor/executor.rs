//! The `PlanExecutor` implementation the coordinator loop ships with.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use agent_driver_rs::SystemPrompt;
use agent_driver_rs::tool::ToolContext;
use async_trait::async_trait;

use crate::artifacts::{ArtifactFilename, ArtifactStore, InlineThreshold, SpilledBody};
use crate::bounding::ErrorPreviewWidth;
use crate::context::{
    ArtifactRef, CorrelationLabel, ErrorPreview, EvidenceEntry, TaskId, WorkerRole,
};
use crate::coordinator_loop::{
    Attempt, ExecutionObservation, LoopBudget, PlanExecutor, RunStore, TaskObservation, TaskRecord,
    TerminalSlot, WorkerSections, WorkerSpec, WorkerSubmission,
};
use crate::mcp_client::SidecarClient;
use crate::types::{FailureCategory, Plan, Task, TaskState};

use super::lifecycle::DagLifecycleObserver;
use super::worker::{WorkerLoop, WorkerLoopConfig, WorkerOutcome};

/// The orchestrator identity the executor reports to lifecycle observers.
/// The executor owns no session id (the shim's `ShimDagObserver` carries that
/// separately), so it reports a fixed coordinator label.
const ORCHESTRATOR_ID: &str = "coordinator";

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
    /// Optional DAG-lifecycle observer (C2). When present, the executor
    /// emits `on_task_started` / `on_task_completed` around each task run.
    /// The shim's `build_request` plumbs a `ShimDagObserver` here; tests
    /// pass `None`.
    lifecycle: Option<Arc<dyn DagLifecycleObserver>>,
}

impl DagExecutor {
    /// Assemble an executor from its dependencies.
    ///
    /// The `sidecar` is the connected MCP client; `artifacts` is the
    /// filename-addressed store; `worker_config` carries the provider,
    /// model, and budget for worker inner loops; `worker_sections` is the
    /// roster the executor reads worker preambles from; `runs` is the run
    /// store the executor files per-task records into; `inline_threshold`
    /// is the character bound at which worker results spill to artifacts;
    /// `lifecycle` is an optional DAG-lifecycle observer (C2) that
    /// receives `on_task_started` / `on_task_completed` around each task
    /// run — pass `None` when lifecycle events are not needed.
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
        lifecycle: Option<Arc<dyn DagLifecycleObserver>>,
    ) -> Self {
        Self {
            sidecar,
            artifacts,
            worker_config,
            worker_sections,
            runs,
            inline_threshold,
            lifecycle,
        }
    }

    /// The roster spec for a task's assigned worker, when the task names one
    /// the roster carries.
    fn spec_for(&self, task: &Task) -> Option<&WorkerSpec> {
        let worker_name = task.worker.as_deref()?;
        self.worker_sections
            .roster()
            .workers()
            .iter()
            .find(|spec| spec.role().as_str() == worker_name)
    }

    /// Resolve the system prompt for a task's assigned worker (R4).
    ///
    /// A worker's configured preamble is the `%%WORKER_SYSTEM_PROMPT%%`
    /// substitution into the shared worker template, not a replacement for
    /// it: the template is where the mandatory `submit_result` contract, the
    /// single-task scope, and the `read_artifact` guidance live, and a worker
    /// that never sees them cannot report a result through the only channel
    /// [`WorkerOutcome::Submitted`](super::worker::WorkerOutcome::Submitted)
    /// has. The S70 golden corpus pins this composition
    /// (`worker_role_frame_*` snapshots).
    ///
    /// Falls back to the default system prompt from [`WorkerLoopConfig`]
    /// when the task names no worker the roster carries, or when that
    /// worker's configured preamble is empty.
    fn resolve_preamble(&self, task: &Task) -> SystemPrompt {
        match self.spec_for(task).map(WorkerSpec::preamble) {
            Some(preamble) if !preamble.is_empty() => SystemPrompt::new(
                crate::templates::render_worker_preamble(&crate::templates::WorkerPreambleVars {
                    worker_system_prompt: preamble,
                }),
            ),
            _ => self.worker_config.system_prompt.clone(),
        }
    }

    /// Resolve the turn-depth budget for a task's assigned worker.
    ///
    /// Same per-task resolution as [`resolve_preamble`](Self::resolve_preamble):
    /// a worker that configured its own `turn_depth` spends that, and
    /// everything else falls back to the run-wide budget from
    /// [`WorkerLoopConfig`]. A roster-wide budget would give a verifier the
    /// debugger's depth and vice versa.
    fn resolve_budget(&self, task: &Task) -> LoopBudget {
        self.spec_for(task)
            .and_then(WorkerSpec::budget)
            .unwrap_or(self.worker_config.budget)
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
                return Self::execution_failed("no plan was created before execution", Vec::new());
            }
        };

        let attempt = Attempt::new(1).expect("1 is non-zero");
        let mut observations: Vec<Option<TaskObservation>> = vec![None; work_plan.tasks.len()];

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
                let index = *task_index
                    .get(&task_id)
                    .expect("ready task id exists in plan");
                work_plan.tasks[index].start();

                // C2: notify the lifecycle observer before the worker loop
                // begins. The description and worker identity are read here
                // so the borrow ends before the worker runs.
                let description = work_plan.tasks[index].description.as_str();
                let worker_id = work_plan.tasks[index]
                    .worker
                    .as_deref()
                    .unwrap_or("default");
                let task_start = Instant::now();
                if let Some(observer) = self.lifecycle.as_ref() {
                    observer
                        .on_task_started(task_id, description, worker_id, ORCHESTRATOR_ID)
                        .await;
                }

                let config = WorkerLoopConfig {
                    provider: Arc::clone(&self.worker_config.provider),
                    model: self.worker_config.model.clone(),
                    budget: self.resolve_budget(&work_plan.tasks[index]),
                    system_prompt: self.resolve_preamble(&work_plan.tasks[index]),
                };

                let slot: TerminalSlot<WorkerSubmission> = TerminalSlot::new();
                let worker = WorkerLoop::new(config, self.sidecar.clone(), self.artifacts.clone());
                let outcome = worker.run_task(&work_plan.tasks[index], slot.clone()).await;

                let label = Self::correlation_label(&work_plan.tasks[index]);
                let (observation, new_state) = self
                    .map_outcome(&outcome, &label, &work_plan.tasks[index])
                    .await;
                observations[index] = Some(observation.clone());

                // C2/R5: notify the lifecycle observer after the task settles.
                // The duration is the wall-clock task run; success and result
                // are read from the resolved plan state.
                let success = matches!(new_state, TaskState::Complete { .. });
                let result_text = match &new_state {
                    TaskState::Complete { result } => Some(result.as_str()),
                    _ => None,
                };
                let duration_ms = task_start.elapsed().as_millis() as u64;
                if let Some(observer) = self.lifecycle.as_ref() {
                    observer
                        .on_task_completed(task_id, success, duration_ms, result_text)
                        .await;
                }

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
                    let record = TaskRecord::new(plan_id.clone(), attempt, blocked.clone());
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
                    let evidence = EvidenceEntry::from_completed_result(result_text, Some(claim))
                        .expect(
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
                    let filename_str = format!("task-{}-{}-result.txt", task.id, worker_name);
                    let spill_result = match ArtifactFilename::new(&filename_str) {
                        Ok(filename) => self
                            .artifacts
                            .write_artifact(&filename, result_text)
                            .await
                            .map(|_| filename),
                        Err(error) => Err(error),
                    };

                    match spill_result {
                        Ok(filename) => {
                            let spilled_body =
                                SpilledBody::new(filename.clone(), result_text.chars().count());
                            let spilled_text = format!("{}\n\n{spilled_body}", claim.summary());
                            let evidence =
                                EvidenceEntry::from_completed_result(&spilled_text, Some(claim))
                                    .expect(
                                        "spilled text carries a footer with a non-blank claim \
                                 summary prefix; from_completed_result takes the \
                                 ArtifactPointer path which is infallible with a claim",
                                    );
                            let artifact_ref =
                                ArtifactRef::new(filename.as_str(), result_text.len() as u64)
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
                                error: ErrorPreview::new(&message, ErrorPreviewWidth::DEFAULT),
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
            if task.dependencies.contains(&current_id) && matches!(task.state, TaskState::Pending) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use agent_driver_rs::ModelId;
    use agent_driver_rs::provider::mock::MockProvider;

    use crate::artifacts::ArtifactStore;
    use crate::bounding::ToolListLimit;
    use crate::config::{OrchestrationConfig, WorkerConfig};
    use crate::coordinator_loop::WorkerRoster;
    use crate::producers::ToolInventory;

    use super::*;

    const RUN_WIDE_TURNS: u32 = 6;

    fn worker(preamble: &str, turn_depth: Option<usize>) -> WorkerConfig {
        WorkerConfig {
            description: "Terminal work".to_owned(),
            preamble: preamble.to_owned(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth,
            llm: None,
            scratchpad: None,
            skills: None,
        }
    }

    /// An executor over a roster of two workers: `operator` carries a
    /// configured preamble and its own turn depth, `analyst` carries
    /// neither.
    fn executor() -> DagExecutor {
        let mut workers = HashMap::new();
        workers.insert(
            "operator".to_owned(),
            worker("You are a Terminal Operator.", Some(24)),
        );
        workers.insert("analyst".to_owned(), worker("", None));
        let config = OrchestrationConfig {
            enabled: true,
            workers,
            ..Default::default()
        };
        let roster = WorkerRoster::from_config(
            &config,
            ToolListLimit::new(10),
            &[],
            &ToolInventory::empty(),
        )
        .expect("24 is a spendable turn depth");

        DagExecutor::new(
            crate::mcp_client::SidecarClient::disconnected(),
            ArtifactStore::new(std::path::PathBuf::from(
                "/tmp/agent-driver-prototype-unused",
            )),
            WorkerLoopConfig {
                provider: Arc::new(MockProvider::new(Vec::new())),
                model: ModelId::new("mock-model").expect("valid model id"),
                budget: LoopBudget::new(RUN_WIDE_TURNS).expect("non-zero budget"),
                system_prompt: SystemPrompt::new("run-wide worker prompt"),
            },
            WorkerSections::from_roster(roster),
            RunStore::new(),
            InlineThreshold::DEFAULT,
            None,
        )
    }

    fn task_for(worker: Option<&str>) -> Task {
        let mut task = Task::new(0, "Create the file", "Create /tmp/s74_hello.txt");
        task.worker = worker.map(str::to_owned);
        task
    }

    /// A configured worker preamble is composed into the shared worker
    /// template rather than replacing it, so the mandatory `submit_result`
    /// contract still reaches the worker. The S70 golden corpus pins this
    /// composition; replacing the template is what left the live shim's
    /// operator with no way to report a result.
    #[test]
    fn configured_preamble_composes_into_the_worker_template() {
        let prompt = executor().resolve_preamble(&task_for(Some("operator")));
        let text = prompt.as_str();

        assert!(
            text.contains("You are a Terminal Operator."),
            "the configured preamble is the template's system-prompt slot"
        );
        assert!(
            text.starts_with("# Worker Agent"),
            "the composed prompt opens with the worker template header, got: {text}"
        );
        assert!(
            text.contains("You MUST call the `submit_result` tool"),
            "the mandatory submit_result contract survives composition"
        );
    }

    /// A task naming a worker with no configured preamble, and a task naming
    /// no worker at all, both fall back to the run-wide system prompt.
    #[test]
    fn absent_preamble_falls_back_to_the_run_wide_prompt() {
        let executor = executor();
        assert_eq!(
            executor
                .resolve_preamble(&task_for(Some("analyst")))
                .as_str(),
            "run-wide worker prompt"
        );
        assert_eq!(
            executor.resolve_preamble(&task_for(None)).as_str(),
            "run-wide worker prompt"
        );
    }

    /// A worker spends its own configured turn depth; a worker without one
    /// spends the run-wide budget. Before this resolution every worker ran
    /// at the top-level `[agent].turn_depth`, so a 24-turn operator was
    /// capped at the coordinator's 6.
    #[test]
    fn budget_resolves_per_worker_with_a_run_wide_fallback() {
        let executor = executor();
        assert_eq!(
            executor.resolve_budget(&task_for(Some("operator"))).turns(),
            24,
            "the worker's own turn depth wins"
        );
        assert_eq!(
            executor.resolve_budget(&task_for(Some("analyst"))).turns(),
            RUN_WIDE_TURNS,
            "a worker with no configured depth spends the run-wide budget"
        );
        assert_eq!(
            executor.resolve_budget(&task_for(None)).turns(),
            RUN_WIDE_TURNS,
            "an unassigned task spends the run-wide budget"
        );
        assert_eq!(
            executor
                .resolve_budget(&task_for(Some("not-in-roster")))
                .turns(),
            RUN_WIDE_TURNS,
            "a worker the roster does not carry spends the run-wide budget"
        );
    }
}
