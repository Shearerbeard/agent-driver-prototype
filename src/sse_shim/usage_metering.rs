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
use tokio::sync::Mutex;

use super::session::UsageAccumulator;

/// A provider decorator that meters token usage into a request-scoped sink.
///
/// One `UsageMeteringProvider` per `/v1/chat/completions` request, wrapping
/// the shared base provider from `ShimState`. The sink is the same
/// `Arc<Mutex<UsageAccumulator>>` the observer reads at `LoopComplete`.
///
/// The decorator is separate from the DAG-lifecycle sink (C2): usage
/// metering intercepts the provider stream; lifecycle events come from the
/// DAG executor. Different concerns, different seams.
pub struct UsageMeteringProvider {
    inner: Arc<dyn Provider>,
    sink: Arc<Mutex<UsageAccumulator>>,
}

impl UsageMeteringProvider {
    /// Wrap a base provider with a request-scoped usage sink.
    #[must_use]
    pub fn new(inner: Arc<dyn Provider>, sink: Arc<Mutex<UsageAccumulator>>) -> Self {
        Self { inner, sink }
    }

    /// The request-scoped usage sink. The observer reads this at
    /// `LoopComplete` to emit the terminal `aura.usage` event.
    #[must_use]
    pub fn sink(&self) -> &Arc<Mutex<UsageAccumulator>> {
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
        Box::pin(async move {
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
/// into the request-scoped sink before forwarding the event.
///
/// Metering happens at the point usage metadata actually arrives (the
/// `Completed` event), once per stream, with no estimation and no
/// double-counting: each stream emits exactly one `Completed`, and the
/// `Started` metadata (which some providers also populate) is deliberately
/// not read.
///
/// The sink is a `tokio::sync::Mutex`, but the write site is the synchronous
/// `poll_next`. The lock is acquired with `try_lock`: the coordinator and
/// worker loops drive provider streams sequentially (the `DagExecutor`'s
/// ready-task loop awaits each worker, and the coordinator's own stream
/// completes before any tool runs), so at most one stream is active per
/// request, and the observer reads the sink only at `LoopComplete` after the
/// stream has ended. The lock is therefore never contended at the write site;
/// the `expect` names that invariant so a future parallelization that breaks
/// it fails loud rather than silently undercounting.
struct MeteredStream {
    inner: CompletionStream,
    sink: Arc<Mutex<UsageAccumulator>>,
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
                    let mut acc = self.sink.try_lock().expect(
                        "usage sink uncontended: provider streams run sequentially within a \
                         request and the observer reads only at LoopComplete",
                    );
                    acc.add(usage);
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

        let acc = sink.lock().await;
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

        let acc = sink.lock().await;
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

        let acc = sink.lock().await;
        assert_eq!(acc.prompt_tokens(), 0);
        assert_eq!(acc.completion_tokens(), 0);
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
