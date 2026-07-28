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

use agent_driver_rs::error::ProviderError;
use agent_driver_rs::provider::{CompletionRequest, ModelInfo, ProviderContext, ProviderInfo};
use agent_driver_rs::streaming::StreamHandle;
use agent_driver_rs::Provider;

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
        _request: CompletionRequest,
        _ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamHandle, ProviderError>> + Send + '_>> {
        // The implementation phase wraps the inner StreamHandle to
        // intercept the terminal StreamEvent::Completed metadata,
        // extracting CompletionMetadata::usage and adding it to the sink.
        todo!("wrap inner stream to intercept Completed usage, add to sink")
    }

    fn list_models(
        &self,
        _ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send + '_>> {
        todo!("delegate to inner provider")
    }
}
