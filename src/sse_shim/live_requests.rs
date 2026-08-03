//! The registry of coordinator tasks that are still running, and the abort
//! the shutdown path uses to close their spans.
//!
//! A `chat.completions` span lives as long as the task it instruments. Nothing
//! ends that task on its own: the SSE stream holds the only `JoinHandle`, and
//! dropping a `JoinHandle` detaches the task rather than stopping it, so a
//! client that goes away mid-stream leaves the coordinator running with its
//! span open. An open span is never handed to the span processor, so the flush
//! at process exit has nothing of it to export and the trace is lost.
//!
//! Registering each task's [`AbortHandle`] here gives the shutdown path a way
//! to end those tasks — dropping their futures, and with them the span guards —
//! before the exporter flushes.

use std::sync::Mutex;
use std::time::Duration;

use tokio::task::AbortHandle;

/// How often [`LiveRequests::abort_and_settle`] rechecks whether the tasks it
/// aborted have gone.
///
/// An abort takes effect at the task's next yield, which for a coordinator
/// parked on a provider call is the very next scheduler pass. The interval only
/// has to be short against the settle window the caller allows.
const SETTLE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// What one shutdown abort did.
///
/// Reported so the caller can say which of the three shutdowns it got, and so a
/// test can tell a task that was aborted from one that had already finished on
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownAbort {
    /// No request task was still running; every span had already closed.
    NothingLive,
    /// Every aborted task dropped its future inside the settle window, so every
    /// span closed and joined the export queue.
    Settled {
        /// How many tasks the abort reached.
        aborted: usize,
    },
    /// The settle window elapsed with tasks still running, so their spans are
    /// still open and will not be in the flush.
    Unsettled {
        /// How many tasks the abort reached.
        aborted: usize,
        /// How many of them were still running when the window elapsed.
        still_running: usize,
    },
}

/// The abort handles of the coordinator tasks this process has started and not
/// yet seen finish.
#[derive(Debug, Default)]
pub struct LiveRequests {
    handles: Mutex<Vec<AbortHandle>>,
}

impl LiveRequests {
    /// Record `handle` as live.
    ///
    /// Handles of tasks that have since finished are dropped on the way past,
    /// which bounds the registry at what is actually in flight without a
    /// reaper of its own.
    pub fn register(&self, handle: AbortHandle) {
        let mut handles = self
            .handles
            .lock()
            .expect("the registry lock is only held for a push or a take");
        handles.retain(|live| !live.is_finished());
        handles.push(handle);
    }

    /// Abort every task still running and wait up to `window` for their futures
    /// to be dropped.
    ///
    /// Tokio drops an aborted task's future before it marks the task finished,
    /// so a handle reporting finished here is a span that has already closed
    /// and reached the span processor. That is the whole point of waiting: the
    /// caller's next move is the flush, and a span that has not closed yet is
    /// not in the queue the flush drains.
    pub async fn abort_and_settle(&self, window: Duration) -> ShutdownAbort {
        // The lock is released before the first await: it guards a `Vec`, never
        // an await point.
        let taken = std::mem::take(
            &mut *self
                .handles
                .lock()
                .expect("the registry lock is only held for a push or a take"),
        );
        let live: Vec<AbortHandle> = taken
            .into_iter()
            .filter(|handle| !handle.is_finished())
            .collect();
        if live.is_empty() {
            return ShutdownAbort::NothingLive;
        }
        for handle in &live {
            handle.abort();
        }

        let settled = tokio::time::timeout(window, async {
            while live.iter().any(|handle| !handle.is_finished()) {
                tokio::time::sleep(SETTLE_POLL_INTERVAL).await;
            }
        })
        .await
        .is_ok();

        let aborted = live.len();
        if settled {
            ShutdownAbort::Settled { aborted }
        } else {
            ShutdownAbort::Unsettled {
                aborted,
                still_running: live.iter().filter(|handle| !handle.is_finished()).count(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry with nothing in flight reports as much rather than claiming
    /// an abort it never made.
    #[tokio::test]
    async fn an_empty_registry_aborts_nothing() {
        let live = LiveRequests::default();

        assert_eq!(
            live.abort_and_settle(Duration::from_millis(50)).await,
            ShutdownAbort::NothingLive
        );
    }

    /// A task parked on an await is aborted and settles, which is the state a
    /// coordinator waiting on a provider call is in when the signal lands.
    #[tokio::test]
    async fn a_parked_task_is_aborted_and_settles() {
        let live = LiveRequests::default();
        let parked = tokio::spawn(std::future::pending::<()>());
        live.register(parked.abort_handle());

        let outcome = live.abort_and_settle(Duration::from_secs(1)).await;

        assert_eq!(outcome, ShutdownAbort::Settled { aborted: 1 });
        assert!(parked.is_finished(), "the aborted task is gone");
    }

    /// A task that finished on its own is not reported as aborted, so the count
    /// the caller logs is the number of runs the shutdown actually cut short.
    #[tokio::test]
    async fn a_task_that_already_finished_is_not_counted() {
        let live = LiveRequests::default();
        let done = tokio::spawn(std::future::ready(()));
        live.register(done.abort_handle());
        done.await.expect("the task completes");

        assert_eq!(
            live.abort_and_settle(Duration::from_millis(50)).await,
            ShutdownAbort::NothingLive
        );
    }

    /// Registering prunes the handles of finished tasks, so a process serving
    /// request after request does not accumulate them.
    #[tokio::test]
    async fn registering_drops_the_handles_of_finished_tasks() {
        let live = LiveRequests::default();
        for _ in 0_u8..3 {
            let done = tokio::spawn(std::future::ready(()));
            live.register(done.abort_handle());
            done.await.expect("the task completes");
        }
        let parked = tokio::spawn(std::future::pending::<()>());
        live.register(parked.abort_handle());

        assert_eq!(
            live.abort_and_settle(Duration::from_secs(1)).await,
            ShutdownAbort::Settled { aborted: 1 }
        );
    }

    /// A task that cannot reach a yield point inside the window is reported
    /// unsettled rather than silently treated as closed, because its span is
    /// still open when the flush runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_task_that_will_not_yield_is_reported_unsettled() {
        let live = LiveRequests::default();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let blocked = tokio::spawn(async move {
            let _ = started_tx.send(());
            // A blocking wait inside an async task: an abort landing now can
            // only set the cancelled bit, and the future is not dropped until
            // this poll returns.
            let _ = release_rx.recv();
        });
        // Without waiting for the poll to start, the abort would reach an idle
        // task and settle it at once, which is the opposite of what this covers.
        started_rx.await.expect("the task reaches its first poll");
        live.register(blocked.abort_handle());

        let outcome = live.abort_and_settle(Duration::from_millis(60)).await;

        assert_eq!(
            outcome,
            ShutdownAbort::Unsettled {
                aborted: 1,
                still_running: 1
            }
        );
        drop(release_tx);
    }
}
