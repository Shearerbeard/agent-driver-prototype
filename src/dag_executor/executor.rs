//! The `PlanExecutor` implementation that replaces `StubExecutor`.

use std::sync::Arc;

use agent_driver_rs::tool::ToolContext;
use async_trait::async_trait;

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::PlanExecutor;
use crate::coordinator_loop::RunStore;
use crate::coordinator_loop::WorkerSections;
use crate::coordinator_loop::ExecutionObservation;
use crate::mcp_client::SidecarClient;
use crate::types::Plan;

use super::worker::WorkerLoopConfig;

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
    #[allow(dead_code)]
    sidecar: SidecarClient,
    #[allow(dead_code)]
    artifacts: ArtifactStore,
    #[allow(dead_code)]
    worker_config: WorkerLoopConfig,
    #[allow(dead_code)]
    worker_sections: WorkerSections,
    #[allow(dead_code)]
    runs: RunStore,
}

impl DagExecutor {
    /// Assemble an executor from its dependencies.
    ///
    /// The `sidecar` is the connected MCP client; `artifacts` is the
    /// filename-addressed store; `worker_config` carries the provider,
    /// model, and budget for worker inner loops; `worker_sections` is the
    /// roster the executor reads worker preambles from; `runs` is the run
    /// store the executor files per-task records into.
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
    ) -> Self {
        Self {
            sidecar,
            artifacts,
            worker_config,
            worker_sections,
            runs,
        }
    }
}

#[async_trait]
impl PlanExecutor for DagExecutor {
    async fn execute(&self, plan: &Plan, ctx: &ToolContext) -> ExecutionObservation {
        let _ = (plan, ctx);
        todo!(
            "Phase 2: flatten plan, select ready tasks, dispatch workers, \
             propagate failure to descendants, assemble the review packet"
        )
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
