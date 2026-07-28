//! DAG task lifecycle observation seam.
//!
//! The `DagExecutor` calls [`DagLifecycleObserver`] around each task run,
//! emitting started/completed events. The shim implements this trait to
//! produce `aura.orchestrator.task_started` and `task_completed` SSE
//! events.
//!
//! This seam is separate from the usage-metering provider wrapper (C1):
//! lifecycle events come from the DAG executor; usage comes from the
//! provider stream. Different concerns, different seams.

use async_trait::async_trait;

/// Observes DAG task lifecycle events.
///
/// The `DagExecutor` calls `on_task_started` before a worker begins a task
/// and `on_task_completed` after the worker finishes. The implementor
/// converts these into the appropriate `aura.orchestrator.*` SSE events.
///
/// One observer per `/v1/chat/completions` request, plumbed through
/// `DagExecutor::new` as an optional parameter. Pass `None` when lifecycle
/// events are not needed (e.g. in integration tests).
#[async_trait]
pub trait DagLifecycleObserver: Send + Sync {
    /// A worker is starting task `task_id`.
    async fn on_task_started(
        &self,
        task_id: usize,
        description: &str,
        worker_id: &str,
        orchestrator_id: &str,
    );

    /// A worker has finished task `task_id`.
    ///
    /// `success` is true when the worker submitted a result; `result` is
    /// the submission text on success, `None` on failure. `duration_ms` is
    /// the wall-clock duration of the task run.
    async fn on_task_completed(
        &self,
        task_id: usize,
        success: bool,
        duration_ms: u64,
        result: Option<&str>,
    );
}
