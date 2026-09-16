//! The shim's DAG lifecycle observer: translates task lifecycle events
//! from the `DagExecutor` into `aura.orchestrator.*` SSE events, and mints
//! the per-task worker observers whose tool and reasoning events fill the
//! space between `task_started` and `task_completed`.
//!
//! Separate from the usage-metering provider wrapper (C1): lifecycle
//! events come from the DAG executor; usage comes from the provider
//! stream. Different concerns, different seams.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use agent_driver_rs::Provider;
use async_trait::async_trait;
use tokio::sync::mpsc::Sender;

use crate::dag_executor::{DagLifecycleObserver, WorkerLane, WorkerObserverFactory};
use crate::sse_shim::events::{AuraEvent, TaskCompletedPayload, TaskStartedPayload};
use crate::sse_shim::observer::ShimWorkerObserver;
use crate::sse_shim::session::{ShimSessionId, UsageAccumulator};
use crate::sse_shim::usage_metering::{FinalTurnCell, UsageMeteringProvider};

/// The shim's implementation of [`DagLifecycleObserver`].
///
/// One observer per `/v1/chat/completions` request, constructed by
/// `build_request` and plumbed into the per-request `DagExecutor`. The
/// observer holds the event channel sender and the session id needed to
/// build `TaskStartedPayload` and `TaskCompletedPayload`.
///
/// The observer emits `AuraEvent::TaskStarted` and `AuraEvent::TaskCompleted`
/// to the same event channel the coordinator's `ShimObserver` feeds.
///
/// The `DagLifecycleObserver::on_task_completed` trait does not carry the
/// `worker_id` or `orchestrator_id`, so the observer remembers the pair from
/// `on_task_started` (keyed by task id) and reuses it at completion, keeping
/// the two payloads' correlation fields consistent.
pub struct ShimDagObserver {
    session_id: ShimSessionId,
    event_tx: Sender<AuraEvent>,
    task_workers: Mutex<HashMap<usize, (String, String)>>,
}

impl ShimDagObserver {
    /// Construct a DAG lifecycle observer for one request.
    #[must_use]
    pub fn new(session_id: ShimSessionId, event_tx: Sender<AuraEvent>) -> Self {
        Self {
            session_id,
            event_tx,
            task_workers: Mutex::new(HashMap::new()),
        }
    }

    /// The session id for correlation in lifecycle payloads.
    #[must_use]
    pub fn session_id(&self) -> ShimSessionId {
        self.session_id
    }

    /// Send an event to the SSE stream, logging if the channel is closed
    /// (C10: bounded channel; disconnect is logged, not swallowed).
    async fn emit(&self, event: AuraEvent) {
        if self.event_tx.send(event).await.is_err() {
            tracing::warn!(
                session_id = %self.session_id,
                "SSE event channel closed; DAG lifecycle event dropped"
            );
        }
    }
}

#[async_trait]
impl DagLifecycleObserver for ShimDagObserver {
    async fn on_task_started(
        &self,
        task_id: usize,
        description: &str,
        worker_id: &str,
        orchestrator_id: &str,
    ) {
        // Remember the worker/orchestrator identity so the completed payload
        // (whose trait signature carries neither) stays consistent with this
        // started payload.
        self.task_workers
            .lock()
            .expect("task_workers lock poisoned")
            .insert(task_id, (worker_id.to_owned(), orchestrator_id.to_owned()));

        let payload = TaskStartedPayload::new(
            task_id,
            description,
            worker_id,
            orchestrator_id,
            worker_id,
            self.session_id.as_str(),
        )
        .expect("task description, worker_id, and orchestrator_id are non-empty: the coordinator plan and DAG executor supply them");
        self.emit(AuraEvent::TaskStarted(payload)).await;
    }

    async fn on_task_completed(
        &self,
        task_id: usize,
        success: bool,
        duration_ms: u64,
        result: Option<&str>,
    ) {
        let (worker_id, orchestrator_id) = self
            .task_workers
            .lock()
            .expect("task_workers lock poisoned")
            .remove(&task_id)
            .expect("on_task_completed called for a task with no matching task_started: the DAG executor always pairs started/completed");

        let payload = if success {
            let result_text = result.unwrap_or("");
            TaskCompletedPayload::success(
                task_id,
                duration_ms,
                &orchestrator_id,
                &worker_id,
                result_text,
                &worker_id,
                self.session_id.as_str(),
            )
        } else {
            TaskCompletedPayload::failure(
                task_id,
                duration_ms,
                &orchestrator_id,
                &worker_id,
                &worker_id,
                self.session_id.as_str(),
            )
        };
        self.emit(AuraEvent::TaskCompleted(payload)).await;
    }
}

/// The shim's [`WorkerObserverFactory`]: mints one worker lane per task
/// run - a [`ShimWorkerObserver`] plus the lane's metered provider - over
/// the request's session id, event channel, shared totals sink, and base
/// provider.
///
/// Constructed once per `/v1/chat/completions` request in
/// `build_request`; the worker loop runs on the lane's provider (so the
/// lane's final-turn cell tracks that task's own calls) and attaches the
/// lane's observer to its `AgentLoop`. The lane provider wraps the BASE
/// provider, never the coordinator's metered instance, so totals are
/// counted exactly once per call.
pub struct ShimWorkerObserverFactory {
    session_id: ShimSessionId,
    event_tx: Sender<AuraEvent>,
    usage: Arc<Mutex<UsageAccumulator>>,
    base_provider: Arc<dyn Provider>,
}

impl ShimWorkerObserverFactory {
    /// Construct the factory for one request.
    #[must_use]
    pub fn new(
        session_id: ShimSessionId,
        event_tx: Sender<AuraEvent>,
        usage: Arc<Mutex<UsageAccumulator>>,
        base_provider: Arc<dyn Provider>,
    ) -> Self {
        Self {
            session_id,
            event_tx,
            usage,
            base_provider,
        }
    }
}

impl WorkerObserverFactory for ShimWorkerObserverFactory {
    fn lane_for(&self, task_id: usize, worker_id: &str) -> WorkerLane {
        let final_turn: FinalTurnCell = Arc::new(Mutex::new(None));
        let provider = Arc::new(UsageMeteringProvider::new_lane(
            Arc::clone(&self.base_provider),
            Arc::clone(&self.usage),
            Arc::clone(&final_turn),
        )) as Arc<dyn Provider>;
        let observer = Arc::new(ShimWorkerObserver::new(
            self.session_id,
            task_id,
            worker_id,
            final_turn,
            self.event_tx.clone(),
        ));
        WorkerLane { observer, provider }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse_shim::events::{EVENT_TASK_COMPLETED, EVENT_TASK_STARTED};
    use crate::sse_shim::session::ShimSessionId;
    use tokio::sync::mpsc;

    async fn drain(rx: &mut mpsc::Receiver<AuraEvent>) -> Vec<AuraEvent> {
        let mut out = Vec::new();
        while let Some(event) = rx.recv().await {
            out.push(event);
        }
        out
    }

    #[tokio::test]
    async fn task_started_emits_payload_with_identity_fields() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        let observer = ShimDagObserver::new(ShimSessionId::generate(), tx);
        observer
            .on_task_started(7, "do the thing", "operations", "coordinator")
            .await;
        drop(observer);

        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 1);
        let data = events[0].sse_data();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["task_id"].as_u64(), Some(7));
        assert_eq!(v["description"].as_str(), Some("do the thing"));
        assert_eq!(v["worker_id"].as_str(), Some("operations"));
        assert_eq!(v["orchestrator_id"].as_str(), Some("coordinator"));
        assert_eq!(v["agent_id"].as_str(), Some("operations"));
        assert!(v["session_id"].as_str().is_some());
        assert_eq!(events[0].sse_event_name(), Some(EVENT_TASK_STARTED));
    }

    #[tokio::test]
    async fn task_completed_success_carries_result_and_duration() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        let observer = ShimDagObserver::new(ShimSessionId::generate(), tx);
        observer
            .on_task_started(3, "desc", "worker-a", "coordinator")
            .await;
        observer
            .on_task_completed(3, true, 42, Some("the evidence"))
            .await;
        drop(observer);

        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuraEvent::TaskStarted(_)));
        let data = events[1].sse_data();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["task_id"].as_u64(), Some(3));
        assert_eq!(v["success"].as_bool(), Some(true));
        assert_eq!(v["duration_ms"].as_u64(), Some(42));
        assert_eq!(v["result"].as_str(), Some("the evidence"));
        assert_eq!(v["worker_id"].as_str(), Some("worker-a"));
        assert_eq!(v["orchestrator_id"].as_str(), Some("coordinator"));
        assert!(v.get("error").is_none_or(|e| e.is_null()));
        assert_eq!(events[1].sse_event_name(), Some(EVENT_TASK_COMPLETED));
    }

    #[tokio::test]
    async fn task_completed_failure_omits_result_and_sets_success_false() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        let observer = ShimDagObserver::new(ShimSessionId::generate(), tx);
        observer
            .on_task_started(5, "desc", "worker-b", "coordinator")
            .await;
        observer.on_task_completed(5, false, 10, None).await;
        drop(observer);

        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 2);
        let data = events[1].sse_data();
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        assert_eq!(v["success"].as_bool(), Some(false));
        assert_eq!(v["duration_ms"].as_u64(), Some(10));
        assert_eq!(v["worker_id"].as_str(), Some("worker-b"));
        // The failure constructor produces no `result` field (it is
        // skip_serializing_if None).
        assert!(v.get("result").is_none());
    }

    /// S102: the worker observer factory mints observers that carry the
    /// task and worker identity into their first emitted event.
    #[tokio::test]
    async fn worker_observer_factory_carries_task_and_worker_identity() {
        let (tx, mut rx) = mpsc::channel::<AuraEvent>(16);
        {
            let factory = ShimWorkerObserverFactory::new(
                ShimSessionId::generate(),
                tx,
                crate::sse_shim::session::shared_accumulator(),
                Arc::new(agent_driver_rs::mock::MockProvider::new(Vec::new())),
            );
            let lane = factory.lane_for(9, "debugger");
            lane.observer
                .on_event(&agent_driver_rs::agent::AgentEvent::ToolCallStart {
                    id: agent_driver_rs::ToolCallId::new("t"),
                    name: agent_driver_rs::ToolName::new("capture-pane").unwrap(),
                    input: serde_json::Value::Null,
                })
                .await;
        }
        // The factory holds a channel clone, so `drain` only returns once
        // the scope above dropped every sender.

        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 1);
        let v: serde_json::Value = serde_json::from_str(&events[0].sse_data()).unwrap();
        assert_eq!(v["task_id"].as_u64(), Some(9));
        assert_eq!(v["worker_id"].as_str(), Some("debugger"));
        assert_eq!(v["agent_id"].as_str(), Some("debugger"));
        // Null tool arguments are omitted, not serialized as null.
        assert!(v.get("arguments").is_none());
    }
}
