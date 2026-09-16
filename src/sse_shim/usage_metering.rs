//! Per-request provider decorator that meters token usage.
//!
//! The pin's `AgentLoop` emits `IterationComplete` only for continuation
//! responses — the initial response and the terminal no-tool response
//! never produce one (`driver.rs:214-224` calls `complete_loop` on the
//! no-tool path without firing `IterationComplete`; `driver.rs:371-378`
//! emits it only after tool execution). An observer that accumulates
//! usage from `IterationComplete` therefore undercounts.
//!
//! This decorator wraps the real provider and intercepts every
//! `complete_stream` call, capturing the `CompletionMetadata::usage` from
//! the stream's terminal `Completed` event. The request-scoped
//! [`UsageAccumulator`] is the single source of token totals; the
//! observer reads it at `LoopComplete` to emit `aura.usage` and does NOT
//! accumulate usage itself (no double-counting).
//!
//! See `DESIGN.md` §C1 for the finding and rationale.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use agent_driver_rs::Provider;
use agent_driver_rs::error::{ProviderError, StreamError};
use agent_driver_rs::provider::{CompletionRequest, ModelInfo, ProviderContext, ProviderInfo};
use agent_driver_rs::streaming::{CompletionStream, StreamEvent, StreamHandle};

use futures::Stream;

use super::session::UsageAccumulator;

/// The per-agent final-turn usage cell one metering lane reports:
/// `(context_tokens, response_tokens)`, or `None` before that lane's
/// first usage-bearing call.
pub type FinalTurnCell = Arc<std::sync::Mutex<Option<(u64, u64)>>>;

/// A provider decorator that meters token usage into a request-scoped sink
/// and, on lanes created with [`new_lane`](Self::new_lane), records the
/// lane's own final-turn usage in a private cell.
///
/// One `UsageMeteringProvider` per LANE (the coordinator's loop, or one
/// worker task run), wrapping the shared base provider from `ShimState`.
/// The sink is the request-wide `Arc<std::sync::Mutex<UsageAccumulator>>` every lane
/// adds its totals to (the observer reads it at `LoopComplete` for
/// `aura.usage`); the final-turn cell is private to the lane, so an
/// agent's `aura.context_usage` reports that agent's own last call even
/// when sibling tools run concurrently (the pin's `join_all`) or when a
/// later agent's call reports no usage.
///
/// The decorator is separate from the DAG-lifecycle sink (C2): usage
/// metering intercepts the provider stream; lifecycle events come from the
/// DAG executor. Different concerns, different seams.
pub struct UsageMeteringProvider {
    inner: Arc<dyn Provider>,
    sink: Arc<std::sync::Mutex<UsageAccumulator>>,
    last: Option<FinalTurnCell>,
}

impl UsageMeteringProvider {
    /// Wrap a base provider with a request-scoped usage sink, recording
    /// totals only (no lane cell). Used where no agent reads final-turn
    /// occupancy.
    #[must_use]
    pub fn new(inner: Arc<dyn Provider>, sink: Arc<std::sync::Mutex<UsageAccumulator>>) -> Self {
        Self {
            inner,
            sink,
            last: None,
        }
    }

    /// Wrap a base provider for one agent lane: totals still flow to the
    /// shared sink, and the lane's final-turn usage lands in `last`, which
    /// the lane's observer reads at its loop completion for
    /// `aura.context_usage`.
    #[must_use]
    pub fn new_lane(
        inner: Arc<dyn Provider>,
        sink: Arc<std::sync::Mutex<UsageAccumulator>>,
        last: FinalTurnCell,
    ) -> Self {
        Self {
            inner,
            sink,
            last: Some(last),
        }
    }

    /// The request-scoped usage sink. The observer reads this at
    /// `LoopComplete` to emit the terminal `aura.usage` event.
    #[must_use]
    pub fn sink(&self) -> &Arc<std::sync::Mutex<UsageAccumulator>> {
        &self.sink
    }
}

impl Provider for UsageMeteringProvider {
    fn info(&self) -> &ProviderInfo {
        self.inner.info()
    }

    fn complete_stream(
        &self,
        request: CompletionRequest,
        ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamHandle, ProviderError>> + Send + '_>> {
        let inner = Arc::clone(&self.inner);
        let sink = Arc::clone(&self.sink);
        let last = self.last.clone();
        Box::pin(async move {
            // N4 (Gate A round 2): a lane's cell describes THIS call only.
            // Clear it at the start of every call, then let the terminal
            // `Completed` populate it. A final turn that reports no usage -
            // or a stream that fails before completing - therefore reads
            // as unknown occupancy (None), never an earlier turn's numbers.
            if let Some(last) = &last {
                *last
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            }
            let handle = inner.complete_stream(request, ctx).await?;
            // Preserve the inner stream's cancellation token and correlation
            // id so the returned handle behaves identically to the inner one,
            // except that the terminal `Completed` metadata is metered.
            let cancellation = handle.cancellation_token().clone();
            let correlation_id = handle.correlation_id();
            let stream = handle.into_stream();
            let metered = MeteredStream {
                inner: stream,
                sink,
                last,
            };
            Ok(StreamHandle::new(
                Box::pin(metered),
                cancellation,
                correlation_id,
            ))
        })
    }

    fn list_models(
        &self,
        ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send + '_>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move { inner.list_models(ctx).await })
    }
}

/// A stream wrapper that meters the terminal `StreamEvent::Completed` usage
/// into the request-scoped sink (and the lane's final-turn cell, when the
/// lane carries one) before forwarding the event.
///
/// Metering happens at the point usage metadata actually arrives (the
/// `Completed` event), once per stream, with no estimation and no
/// double-counting: each stream emits exactly one `Completed`, and the
/// `Started` metadata (which some providers also populate) is deliberately
/// not read.
///
/// The sink is a std mutex written from this synchronous `poll_next`,
/// where sibling lanes may poll concurrently (the pin's `join_all`); the
/// guard covers a field update only and is never held across an await.
/// A call whose `Completed` carries no usage leaves the lane cell empty
/// (cleared at the call's start): the final turn's occupancy is unknown,
/// reported as zero, and never an earlier turn's numbers masquerading as
/// the final turn (Gate A round-2 finding N4).
struct MeteredStream {
    inner: CompletionStream,
    sink: Arc<std::sync::Mutex<UsageAccumulator>>,
    last: Option<FinalTurnCell>,
}

impl Stream for MeteredStream {
    type Item = Result<StreamEvent, StreamError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(error))),
            Poll::Ready(Some(Ok(StreamEvent::Completed { metadata }))) => {
                if let Some(usage) = metadata.usage {
                    {
                        let mut acc = self
                            .sink
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        acc.add(usage);
                    }
                    if let Some(last) = &self.last {
                        *last
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((
                            u64::from(usage.input_tokens),
                            u64::from(usage.output_tokens),
                        ));
                    }
                }
                Poll::Ready(Some(Ok(StreamEvent::Completed { metadata })))
            }
            Poll::Ready(Some(Ok(other))) => Poll::Ready(Some(Ok(other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse_shim::session::shared_accumulator;

    use agent_driver_rs::mock::MockProvider;
    use agent_driver_rs::provider::{CompletionRequest, ProviderContext};
    use agent_driver_rs::streaming::{
        CompletionMetadata, ContentBlockType, StopReason, StreamDelta,
    };
    use agent_driver_rs::types::{CorrelationId, ModelId};
    use agent_driver_rs::{Provider, TokenUsage};
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    /// A no-tool text response whose `Completed` event carries token usage,
    /// so the metered provider can intercept it. The pin's
    /// `mock_text_response` sets `usage: None`; this helper sets a real
    /// `TokenUsage` to exercise the metering path.
    fn text_response_with_usage(text: &str, usage: TokenUsage) -> Vec<StreamEvent> {
        let mut start = CompletionMetadata::default();
        start.stop_reason = None;
        let mut completed = CompletionMetadata::default();
        completed.stop_reason = Some(StopReason::EndTurn);
        completed.usage = Some(usage);
        vec![
            StreamEvent::Started { metadata: start },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
            },
            StreamEvent::Delta(StreamDelta::TextDelta {
                text: text.to_owned(),
            }),
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::Completed {
                metadata: completed,
            },
        ]
    }

    fn ctx() -> ProviderContext {
        ProviderContext::new(
            CorrelationId::generate(),
            CancellationToken::new(),
            TaskTracker::new(),
        )
    }

    fn request() -> CompletionRequest {
        CompletionRequest::new(ModelId::new("mock-model").unwrap(), Vec::new())
    }

    /// C1 regression: a no-tool response (no `IterationComplete` ever fires)
    /// still has its usage counted because the decorator intercepts the
    /// terminal `Completed` event.
    #[tokio::test]
    async fn metered_provider_counts_no_tool_response_usage() {
        let mock = MockProvider::new(vec![text_response_with_usage(
            "hi",
            TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        )]);
        let sink = shared_accumulator();
        let metered = UsageMeteringProvider::new(Arc::new(mock), Arc::clone(&sink));

        let handle = metered.complete_stream(request(), ctx()).await.unwrap();
        // Drain the stream so the terminal `Completed` event is polled and
        // metered.
        let _ = handle.collect().await.unwrap();

        let acc = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(acc.prompt_tokens(), 10);
        assert_eq!(acc.completion_tokens(), 5);
        assert_eq!(acc.total_tokens(), 15);
    }

    /// Usage accumulates across multiple `complete_stream` calls (coordinator
    /// plus worker loops), proving the sink is the single running total.
    #[tokio::test]
    async fn metered_provider_accumulates_across_calls() {
        let mock = MockProvider::new(vec![
            text_response_with_usage(
                "first",
                TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            ),
            text_response_with_usage(
                "second",
                TokenUsage {
                    input_tokens: 20,
                    output_tokens: 8,
                },
            ),
        ]);
        let sink = shared_accumulator();
        let metered = UsageMeteringProvider::new(Arc::new(mock), Arc::clone(&sink));

        let h1 = metered.complete_stream(request(), ctx()).await.unwrap();
        let _ = h1.collect().await.unwrap();
        let h2 = metered.complete_stream(request(), ctx()).await.unwrap();
        let _ = h2.collect().await.unwrap();

        let acc = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(acc.prompt_tokens(), 30);
        assert_eq!(acc.completion_tokens(), 13);
    }

    /// A response whose `Completed` carries no usage contributes zero, so a
    /// provider that does not report usage does not corrupt the total.
    #[tokio::test]
    async fn metered_provider_skips_when_usage_none() {
        let mock = MockProvider::new(vec![agent_driver_rs::mock::mock_text_response(
            "no usage here",
        )]);
        let sink = shared_accumulator();
        let metered = UsageMeteringProvider::new(Arc::new(mock), Arc::clone(&sink));

        let handle = metered.complete_stream(request(), ctx()).await.unwrap();
        let _ = handle.collect().await.unwrap();

        let acc = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(acc.prompt_tokens(), 0);
        assert_eq!(acc.completion_tokens(), 0);
    }

    /// N1 regression (Gate A round 1): two lanes over one shared sink keep
    /// their own final-turn cells - a worker lane's usage never lands in
    /// the coordinator lane's cell - while the shared totals carry both.
    #[tokio::test]
    async fn lanes_keep_separate_final_turn_cells_over_one_sink() {
        let mock = MockProvider::new(vec![
            text_response_with_usage(
                "coordinator turn",
                TokenUsage {
                    input_tokens: 30,
                    output_tokens: 3,
                },
            ),
            text_response_with_usage(
                "worker turn",
                TokenUsage {
                    input_tokens: 60,
                    output_tokens: 6,
                },
            ),
            agent_driver_rs::mock::mock_text_response("no usage on this turn"),
        ]);
        let base: Arc<dyn Provider> = Arc::new(mock);
        let sink = shared_accumulator();
        let coordinator_cell: FinalTurnCell = Arc::new(std::sync::Mutex::new(None));
        let worker_cell: FinalTurnCell = Arc::new(std::sync::Mutex::new(None));

        let coordinator = UsageMeteringProvider::new_lane(
            Arc::clone(&base),
            Arc::clone(&sink),
            Arc::clone(&coordinator_cell),
        );
        let worker = UsageMeteringProvider::new_lane(
            Arc::clone(&base),
            Arc::clone(&sink),
            Arc::clone(&worker_cell),
        );

        // Coordinator's turn, then the worker's - interleaved lanes over
        // one sink, as sibling tool calls produce.
        let h1 = coordinator.complete_stream(request(), ctx()).await.unwrap();
        let _ = h1.collect().await.unwrap();
        let h2 = worker.complete_stream(request(), ctx()).await.unwrap();
        let _ = h2.collect().await.unwrap();

        assert_eq!(
            *coordinator_cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((30, 3)),
            "the coordinator lane holds its own call"
        );
        assert_eq!(
            *worker_cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((60, 6)),
            "the worker lane holds its own call, not the coordinator's"
        );

        // N4: a call whose Completed carries no usage CLEARS the lane's
        // cell (cleared at call start, populated only by this call's
        // usage) - the final turn's occupancy reads unknown, never the
        // earlier turn's numbers.
        let h3 = worker.complete_stream(request(), ctx()).await.unwrap();
        let _ = h3.collect().await.unwrap();
        assert_eq!(
            *worker_cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            None,
            "a usage-less final turn leaves the lane cell empty"
        );
        // The coordinator lane is untouched by the worker's calls.
        assert_eq!(
            *coordinator_cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((30, 3))
        );

        let acc = sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(acc.prompt_tokens(), 90);
        assert_eq!(acc.completion_tokens(), 9);
    }

    /// N4 (Gate A round 2): a stream that never reaches `Completed`
    /// (dropped or failed mid-call) also leaves the lane cell empty - the
    /// loop's completion then reports unknown occupancy, not the previous
    /// turn's.
    #[tokio::test]
    async fn a_dropped_stream_clears_the_lane_cell() {
        let mock = MockProvider::new(vec![
            text_response_with_usage(
                "first",
                TokenUsage {
                    input_tokens: 60,
                    output_tokens: 6,
                },
            ),
            text_response_with_usage(
                "never drained",
                TokenUsage {
                    input_tokens: 10,
                    output_tokens: 1,
                },
            ),
        ]);
        let sink = shared_accumulator();
        let cell: FinalTurnCell = Arc::new(std::sync::Mutex::new(None));
        let lane =
            UsageMeteringProvider::new_lane(Arc::new(mock), Arc::clone(&sink), Arc::clone(&cell));

        let h1 = lane.complete_stream(request(), ctx()).await.unwrap();
        let _ = h1.collect().await.unwrap();
        assert_eq!(
            *cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some((60, 6))
        );

        // The second call starts (cell cleared at entry) but its stream is
        // dropped before the terminal Completed is ever polled.
        let h2 = lane.complete_stream(request(), ctx()).await.unwrap();
        drop(h2);
        assert_eq!(
            *cell
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            None,
            "a call that never completed leaves the lane cell empty"
        );
    }

    /// `list_models` delegates to the inner provider.
    #[tokio::test]
    async fn metered_provider_list_models_delegates() {
        let mock = MockProvider::new(vec![]);
        let sink = shared_accumulator();
        let metered = UsageMeteringProvider::new(Arc::new(mock), Arc::clone(&sink));

        let models = metered.list_models(ctx()).await.unwrap();
        assert!(models.is_empty());
    }
}
