//! The shim's DAG lifecycle observer: translates task lifecycle events
//! from the `DagExecutor` into `aura.orchestrator.*` SSE events.
//!
//! Separate from the usage-metering provider wrapper (C1): lifecycle
//! events come from the DAG executor; usage comes from the provider
//! stream. Different concerns, different seams.

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;

use crate::dag_executor::DagLifecycleObserver;
use crate::sse_shim::events::AuraEvent;
use crate::sse_shim::session::ShimSessionId;

/// The shim's implementation of [`DagLifecycleObserver`].
///
/// One observer per `/v1/chat/completions` request, constructed by
/// `build_request` and plumbed into the per-request `DagExecutor`. The
/// observer holds the event channel sender and the session id needed to
/// build `TaskStartedPayload` and `TaskCompletedPayload`.
///
/// The observer emits `AuraEvent::TaskStarted` and `AuraEvent::TaskCompleted`
/// to the same event channel the coordinator's `ShimObserver` feeds.
#[allow(dead_code, reason = "type skeleton; constructed by build_request in the implementation phase")]
pub struct ShimDagObserver {
    session_id: ShimSessionId,
    event_tx: Sender<AuraEvent>,
}

impl ShimDagObserver {
    /// Construct a DAG lifecycle observer for one request.
    #[allow(dead_code, reason = "type skeleton; called by build_request in the implementation phase")]
    #[must_use]
    pub fn new(session_id: ShimSessionId, event_tx: Sender<AuraEvent>) -> Self {
        Self { session_id, event_tx }
    }

    /// The session id for correlation in lifecycle payloads.
    #[must_use]
    pub fn session_id(&self) -> ShimSessionId {
        self.session_id
    }
}

#[async_trait]
impl DagLifecycleObserver for ShimDagObserver {
    async fn on_task_started(
        &self,
        _task_id: usize,
        _description: &str,
        _worker_id: &str,
        _orchestrator_id: &str,
    ) {
        todo!("construct TaskStartedPayload, emit AuraEvent::TaskStarted")
    }

    async fn on_task_completed(
        &self,
        _task_id: usize,
        _success: bool,
        _duration_ms: u64,
        _result: Option<&str>,
    ) {
        todo!("construct TaskCompletedPayload, emit AuraEvent::TaskCompleted")
    }
}
