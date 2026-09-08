//! Card S73 Phase 4: offline end-to-end SSE vocabulary proof.
//!
//! Runs a REAL HTTP exchange against the shim's axum router with the pin's
//! `MockProvider` driving both the coordinator and worker loops, and asserts
//! the full `aura.*` vocabulary over the wire. Offline: no Bedrock, no
//! network beyond the localhost listener, no Docker sidecar.
//!
//! The mock provider arithmetic is exact: the mock panics when its queue is
//! exhausted, so reaching the assertions proves the loop made exactly the
//! queued number of provider calls. The shim's `build_request` wraps the
//! base provider in `UsageMeteringProvider` and uses the metered provider
//! for BOTH the coordinator and the worker (it overrides
//! `worker_config.provider` with the metered base provider), so coordinator
//! and worker calls interleave in one queue.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_driver_rs::Provider;
use agent_driver_rs::provider::mock::{MockProvider, mock_text_response, mock_tool_call_response};
use agent_driver_rs::types::{ModelId, SystemPrompt};
use tokio_util::sync::CancellationToken;

use agent_driver_prototype::artifacts::InlineThreshold;
use agent_driver_prototype::bounding::ToolListLimit;
use agent_driver_prototype::config::{OrchestrationConfig, WorkerConfig};
use agent_driver_prototype::coordinator_loop::{
    CreatePlanArgs, LoopBudget, PlanId, WorkerRoster, WorkerSections,
};
use agent_driver_prototype::dag_executor::WorkerLoopConfig;
use agent_driver_prototype::mcp_client::SidecarClient;
use agent_driver_prototype::producers::ToolInventory;
use agent_driver_prototype::sse_shim::{ShimState, router};
use agent_driver_prototype::types::StepInput;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn model() -> ModelId {
    ModelId::new("mock-model").expect("valid model id")
}

/// A single-worker roster with the "operations" worker configured, mirroring
/// the `test_sections()` pattern in `tests/coordinator_loop.rs`. The plan's
/// worker assignment is validated against this roster, so the worker must be
/// present.
fn test_sections() -> WorkerSections {
    let mut workers = HashMap::new();
    workers.insert(
        "operations".to_owned(),
        WorkerConfig {
            description: "Logs, pipelines and metrics".to_owned(),
            preamble: String::new(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    let config = OrchestrationConfig {
        enabled: true,
        workers,
        ..Default::default()
    };
    WorkerSections::from_roster(
        WorkerRoster::from_config(
            &config,
            ToolListLimit::new(10),
            &[],
            &ToolInventory::empty(),
        )
        .expect("no worker configures a turn depth"),
    )
}

/// A one-task plan: a single `LeafTask` assigned to the "operations" worker.
fn one_task_plan_args() -> CreatePlanArgs {
    CreatePlanArgs {
        goal: "Count the errors by service".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "Collect the error counts".to_owned(),
            worker: Some("operations".to_owned()),
        }],
        planning_rationale: "One step".to_owned(),
    }
}

fn submit_result_json(summary: &str, result: &str, confidence: &str) -> String {
    format!(r#"{{"summary":"{summary}","result":"{result}","confidence":"{confidence}"}}"#)
}

/// The single shared `MockProvider` queue.
///
/// Six responses, six provider calls. The shim's `build_request` wraps the
/// base provider in `UsageMeteringProvider` and uses the metered provider
/// for both the coordinator and the worker, so the calls interleave:
///
///   1. `create_plan` (coordinator call 1)
///   2. `execute`      (coordinator call 2)
///   3. `submit_result` (worker call 1, during execute)
///   4. end-turn text   (worker call 2)
///   5. `respond`       (coordinator call 3, after execute)
///   6. end-turn text   (coordinator call 4)
///
/// A seventh call would panic the mock, so reaching the assertions is itself
/// proof the loop made exactly six calls.
fn shim_provider() -> Arc<dyn Provider> {
    let expected_id = PlanId::derive(&one_task_plan_args());
    let plan_args_json = serde_json::to_string(&one_task_plan_args()).expect("plan args serialize");

    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &plan_args_json),
        mock_tool_call_response(
            "c2",
            "execute",
            &format!(r#"{{"plan_id":"{expected_id}"}}"#),
        ),
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("Found 42 errors", "service-a: 42", "high"),
        ),
        mock_text_response(""),
        mock_tool_call_response(
            "c3",
            "respond",
            r#"{"response":"service-a produced 42 of yesterday's errors."}"#,
        ),
        mock_text_response(""),
    ];
    Arc::new(MockProvider::new(responses))
}

/// Build a `ShimState` whose base provider is the scripted `MockProvider`
/// and whose sidecar is disconnected. The `worker_config.provider` is
/// ignored by `build_request` (it overrides it with the metered base
/// provider), but the budget and system prompt are read, so they are set
/// to meaningful values.
fn shim_state(provider: Arc<dyn Provider>, artifact_root: PathBuf) -> Arc<ShimState> {
    let model = model();
    let worker_config = WorkerLoopConfig {
        provider: Arc::clone(&provider),
        model: model.clone(),
        budget: LoopBudget::new(8).expect("non-zero worker budget"),
        system_prompt: SystemPrompt::new("You are a worker. Submit your result."),
        cancellation: CancellationToken::new(),
        observer_factory: None,
    };
    Arc::new(ShimState::from_parts(
        provider,
        model,
        SystemPrompt::new("You coordinate one continuous loop."),
        LoopBudget::new(8).expect("non-zero coordinator budget"),
        SidecarClient::disconnected(),
        artifact_root,
        worker_config,
        test_sections(),
        InlineThreshold::DEFAULT,
        PathBuf::from("/tmp/sse-shim-integration-test.toml"),
    ))
}

// ---------------------------------------------------------------------------
// SSE frame parsing (line-ending-agnostic)
// ---------------------------------------------------------------------------

/// One decoded SSE frame: an optional `event:` name and the `data:` payload.
#[derive(Debug)]
struct SseFrame {
    event: Option<String>,
    data: String,
}

/// Parse an SSE body into frames, handling both LF and CRLF line endings.
///
/// Blank lines are block terminators. `event:` and `data:` field lines are
/// extracted; other fields (`id:`, `retry:`, comments) are ignored. A frame
/// is emitted at each blank line if at least one field was seen.
fn parse_sse_frames(body: &str) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    let mut event: Option<String> = None;
    let mut data: Option<String> = None;
    let mut has_content = false;

    for line in body.lines() {
        if line.is_empty() {
            if has_content {
                frames.push(SseFrame {
                    event: event.take(),
                    data: data.take().unwrap_or_default(),
                });
                has_content = false;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event: ") {
            event = Some(rest.to_owned());
            has_content = true;
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = Some(rest.to_owned());
            has_content = true;
        }
        // Other SSE fields are ignored.
    }
    if has_content {
        frames.push(SseFrame {
            event: event.take(),
            data: data.take().unwrap_or_default(),
        });
    }
    frames
}

/// A human-readable transcript of event names for the report. Data-only
/// frames are labeled `chat.completion.chunk` or `[DONE]`.
fn transcript_summary(frames: &[SseFrame]) -> Vec<String> {
    frames
        .iter()
        .map(|f| match &f.event {
            Some(name) => name.clone(),
            None => {
                if f.data == "[DONE]" {
                    "[DONE]".to_owned()
                } else {
                    "chat.completion.chunk".to_owned()
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Primary end-to-end test
// ---------------------------------------------------------------------------

/// A local chat-completions exchange through the shim emits the full
/// `aura.*` vocabulary and terminates with `[DONE]`.
///
/// The test uses soft asserts: it collects every broken acceptance criterion
/// into one `failures` list and panics once at the end with the full list and
/// the wire transcript. This gives maximum signal from a single run — a
/// missing `aura.session_info` does not mask whether the rest of the
/// vocabulary arrived.
#[tokio::test]
async fn chat_completions_emits_full_aura_vocabulary_and_terminates_with_done() {
    let provider = shim_provider();
    let dir = tempfile::TempDir::new().expect("temp dir for artifact root");
    let state = shim_state(provider, dir.path().to_path_buf());
    let app = router(state);

    // Bind to an ephemeral port on localhost and serve in a spawned task.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // POST the chat-completions request with stream: true.
    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "aura-terminalbench",
        "messages": [{"role": "user", "content": "Count the errors by service"}],
        "stream": true,
    });

    // Read the full response body to completion. The stream must terminate
    // on its own via [DONE]; a client-side timeout firing is a TEST FAILURE.
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&request_body)
            .send(),
    )
    .await
    .expect("request did not complete within 30s — stream failed to start or respond")
    .expect("POST /v1/chat/completions failed");
    assert!(
        response.status().is_success(),
        "HTTP status {} — expected 200",
        response.status()
    );

    let text = tokio::time::timeout(Duration::from_secs(30), response.text())
        .await
        .expect("body did not complete within 30s — stream did not terminate via [DONE]")
        .expect("reading response body failed");

    // Parse the SSE body into structured frames.
    let frames = parse_sse_frames(&text);
    let transcript = transcript_summary(&frames);
    eprintln!(
        "SSE event transcript ({n} frames): {transcript:?}",
        n = frames.len()
    );
    eprintln!("Raw SSE body:\n{text}");

    // Collect every broken assertion so one run shows the full picture.
    let mut failures: Vec<String> = Vec::new();

    // (a) aura.session_info present, carrying a non-empty session_id.
    let session_info_frames: Vec<&SseFrame> = frames
        .iter()
        .filter(|f| f.event.as_deref() == Some("aura.session_info"))
        .collect();
    if session_info_frames.is_empty() {
        failures.push("(a) aura.session_info not emitted on the wire".to_owned());
    } else {
        match serde_json::from_str::<serde_json::Value>(&session_info_frames[0].data) {
            Ok(v) => {
                let sid = v["session_id"].as_str();
                if sid.is_none() || sid.is_some_and(|s| s.is_empty()) {
                    failures.push(
                        "(a) aura.session_info payload has no non-empty session_id".to_owned(),
                    );
                }
            }
            Err(e) => {
                failures.push(format!(
                    "(a) aura.session_info payload is not valid JSON: {e}"
                ));
            }
        }
    }

    // (b) aura.tool_start and aura.tool_complete present for the
    //     coordinator's create_plan and execute calls (names visible).
    for tool_name in ["create_plan", "execute"] {
        let has_start = frames.iter().any(|f| {
            f.event.as_deref() == Some("aura.tool_start")
                && serde_json::from_str::<serde_json::Value>(&f.data)
                    .is_ok_and(|v| v["tool_name"].as_str() == Some(tool_name))
        });
        if !has_start {
            failures.push(format!("(b) aura.tool_start for '{tool_name}' not found"));
        }
        let has_complete = frames.iter().any(|f| {
            f.event.as_deref() == Some("aura.tool_complete")
                && serde_json::from_str::<serde_json::Value>(&f.data)
                    .is_ok_and(|v| v["tool_name"].as_str() == Some(tool_name))
        });
        if !has_complete {
            failures.push(format!(
                "(b) aura.tool_complete for '{tool_name}' not found"
            ));
        }
    }

    // (c) aura.orchestrator.task_started and aura.orchestrator.task_completed
    //     present, task_completed with success: true.
    let has_task_started = frames
        .iter()
        .any(|f| f.event.as_deref() == Some("aura.orchestrator.task_started"));
    if !has_task_started {
        failures.push("(c) aura.orchestrator.task_started not emitted".to_owned());
    }
    let task_completed_success = frames.iter().any(|f| {
        f.event.as_deref() == Some("aura.orchestrator.task_completed")
            && serde_json::from_str::<serde_json::Value>(&f.data)
                .is_ok_and(|v| v["success"].as_bool() == Some(true))
    });
    if !task_completed_success {
        failures
            .push("(c) aura.orchestrator.task_completed with success:true not found".to_owned());
    }

    // (d) aura.usage present with integer prompt_tokens / completion_tokens.
    //     The pin's MockProvider emits usage: None in its Completed metadata,
    //     so the metering decorator adds nothing and the totals are zero.
    //     We assert the fields exist as integers; the >0 check is recorded
    //     as a structured-only finding in the report.
    let usage_frames: Vec<&SseFrame> = frames
        .iter()
        .filter(|f| f.event.as_deref() == Some("aura.usage"))
        .collect();
    if usage_frames.is_empty() {
        failures.push("(d) aura.usage not emitted".to_owned());
    } else {
        match serde_json::from_str::<serde_json::Value>(&usage_frames[0].data) {
            Ok(v) => {
                let prompt = v["prompt_tokens"].as_u64();
                let completion = v["completion_tokens"].as_u64();
                if prompt.is_none() || completion.is_none() {
                    failures.push(
                        "(d) aura.usage payload missing integer prompt_tokens or completion_tokens"
                            .to_owned(),
                    );
                } else {
                    eprintln!(
                        "aura.usage: prompt_tokens={prompt:?}, completion_tokens={completion:?} \
                         (MockProvider emits usage: None — structured-only proof)"
                    );
                }
            }
            Err(e) => {
                failures.push(format!("(d) aura.usage payload is not valid JSON: {e}"));
            }
        }
    }

    // (e) a data-only chat.completion.chunk with finish_reason present
    //     appears before the end.
    let done_index = frames
        .iter()
        .position(|f| f.event.is_none() && f.data == "[DONE]");
    let finish_chunk_before_done = frames.iter().enumerate().any(|(i, f)| {
        f.event.is_none()
            && f.data != "[DONE]"
            && serde_json::from_str::<serde_json::Value>(&f.data)
                .is_ok_and(|v| !v["choices"][0]["finish_reason"].is_null())
            && done_index.is_some_and(|di| i < di)
    });
    if !finish_chunk_before_done {
        failures.push(
            "(e) no data-only chat.completion.chunk with finish_reason present before [DONE]"
                .to_owned(),
        );
    }

    // (f) the terminal frame is data: [DONE] and nothing follows it.
    match frames.last() {
        Some(f) if f.event.is_none() && f.data == "[DONE]" => {}
        other => failures.push(format!(
            "(f) terminal frame is not data: [DONE] — got: {other:?}"
        )),
    }

    // (g) every aura.* data payload is valid JSON.
    for f in &frames {
        if f.event.as_deref().is_some_and(|e| e.starts_with("aura."))
            && serde_json::from_str::<serde_json::Value>(&f.data).is_err()
        {
            failures.push(format!(
                "(g) aura.* payload for '{}' is not valid JSON: {}",
                f.event.as_deref().unwrap_or("?"),
                &f.data[..f.data.len().min(120)]
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "SSE vocabulary assertions failed ({n} failures):\n---\n{fails}\n---\n\
         Wire transcript: {transcript:?}\n\
         Raw body:\n{text}",
        n = failures.len(),
        fails = failures
            .iter()
            .map(|f| format!("  - {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
        transcript = transcript,
        text = text,
    );
}

// ---------------------------------------------------------------------------
// Health check test
// ---------------------------------------------------------------------------

/// `GET /health` returns 200 OK.
#[tokio::test]
async fn health_returns_200() {
    let provider: Arc<dyn Provider> = Arc::new(MockProvider::new(vec![]));
    let dir = tempfile::TempDir::new().expect("temp dir");
    let state = shim_state(provider, dir.path().to_path_buf());
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .expect("health request completes");
    assert!(
        response.status().is_success(),
        "GET /health returned {} — expected 200",
        response.status()
    );
}

// ---------------------------------------------------------------------------
// S90 burn-window test: a client disconnect must stop the provider calls
// ---------------------------------------------------------------------------

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_driver_prototype::sse_shim::ShutdownAbort;
use agent_driver_rs::provider::{ModelInfo, ProviderKind};
use agent_driver_rs::{
    CompletionRequest, CompletionStream, ProviderCapabilities, ProviderContext, ProviderError,
    ProviderInfo, StreamHandle,
};
use futures::StreamExt as _;

/// A provider that answers every call with a further `create_plan` tool call
/// and counts the calls it has served.
///
/// Non-exhausting by construction - a `MockProvider` queue panics on
/// exhaustion, which pre-fix would have ended the detached task and faked a
/// pass. The `Arc<AtomicUsize>` counter from [`BurnProvider::new`] is the
/// observable the assertion reads.
struct BurnProvider {
    calls: Arc<AtomicUsize>,
    info: ProviderInfo,
}

impl BurnProvider {
    /// Build the provider together with the call counter it increments.
    fn new() -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        // `ProviderCapabilities` is `#[non_exhaustive]`, so it cannot be
        // built with a struct literal outside the pin; start from the
        // derived `Default` and flip the capabilities the burn path needs.
        let mut capabilities = ProviderCapabilities::default();
        capabilities.streaming = true;
        capabilities.tools = true;
        (
            Self {
                calls: Arc::clone(&calls),
                info: ProviderInfo {
                    kind: ProviderKind::Anthropic,
                    name: "Burn",
                    capabilities,
                },
            },
            calls,
        )
    }
}

impl Provider for BurnProvider {
    fn info(&self) -> &ProviderInfo {
        &self.info
    }

    fn complete_stream(
        &self,
        _request: CompletionRequest,
        ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamHandle, ProviderError>> + Send + '_>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let events =
            mock_tool_call_response(&format!("c{n}"), "create_plan", &burn_plan_args_json(n));
        Box::pin(async move {
            let stream: CompletionStream =
                Box::pin(futures::stream::iter(events.into_iter().map(Ok)));
            Ok(StreamHandle::new(
                stream,
                ctx.cancellation,
                ctx.correlation_id,
            ))
        })
    }

    fn list_models(
        &self,
        _ctx: ProviderContext,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send + '_>> {
        Box::pin(async { Ok(vec![]) })
    }
}

/// The `create_plan` arguments for burn call `n`, serialized: one leaf task
/// on the configured worker so the plan validates against the roster; the
/// goal embeds `n` so plan ids stay distinct however long the loop burns.
fn burn_plan_args_json(n: usize) -> String {
    let args = CreatePlanArgs {
        goal: format!("burn window probe {n}"),
        steps: vec![StepInput::LeafTask {
            task: format!("Burn turn {n}"),
            worker: Some("operations".to_owned()),
        }],
        planning_rationale: "Keep the loop calling so the burn window stays observable".to_owned(),
    };
    serde_json::to_string(&args).expect("plan args serialize")
}

/// A mirror of [`shim_state`] with budgets no run can reach: the shared
/// fixture's budget of 8 would stop the loop gracefully and fake a pass.
fn burn_state(provider: Arc<dyn Provider>, artifact_root: PathBuf) -> Arc<ShimState> {
    let model = model();
    let worker_config = WorkerLoopConfig {
        provider: Arc::clone(&provider),
        model: model.clone(),
        budget: LoopBudget::new(1_000_000).expect("non-zero worker budget"),
        system_prompt: SystemPrompt::new("You are a worker. Submit your result."),
        cancellation: CancellationToken::new(),
        observer_factory: None,
    };
    Arc::new(ShimState::from_parts(
        provider,
        model,
        SystemPrompt::new("You coordinate one continuous loop."),
        LoopBudget::new(1_000_000).expect("non-zero coordinator budget"),
        SidecarClient::disconnected(),
        artifact_root,
        worker_config,
        test_sections(),
        InlineThreshold::DEFAULT,
        PathBuf::from("/tmp/sse-shim-integration-test.toml"),
    ))
}

/// A client disconnect mid-SSE must stop the coordinator within a bounded
/// window: no further provider calls, and no task left live.
#[tokio::test]
async fn client_disconnect_stops_provider_calls_within_the_bounded_window() {
    let (provider, calls) = BurnProvider::new();
    let provider: Arc<dyn Provider> = Arc::new(provider);
    let dir = tempfile::TempDir::new().expect("temp dir for artifact root");
    let state = burn_state(provider, dir.path().to_path_buf());

    // Clone the live-requests handle BEFORE `router(state)` consumes the
    // state: it is the observable for "the loop already ended on its own".
    let live = Arc::clone(state.live_requests());
    let app = router(state);

    // Bind to an ephemeral port on localhost and serve in a spawned task.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // `pool_max_idle_per_host(0)`: a pooled idle connection would keep the
    // body alive past the drop. The disconnect below must close the
    // connection outright.
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("reqwest client builds");
    let request_body = serde_json::json!({
        "model": "aura-terminalbench",
        "messages": [{"role": "user", "content": "Count the errors by service"}],
        "stream": true,
    });

    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&request_body)
            .send(),
    )
    .await
    .expect("request did not start within 30s — stream failed to respond")
    .expect("POST /v1/chat/completions failed");
    assert!(
        response.status().is_success(),
        "HTTP status {} — expected 200",
        response.status()
    );

    // Sync point: read the body INCREMENTALLY until the first coordinator
    // `aura.tool_start` for `create_plan` arrives, so the loop is provably
    // mid-run before the client drops. Each chunk is appended to one buffer
    // and the accumulated text re-parsed — simple and correct for a single
    // sync point.
    let mut stream = response.bytes_stream();
    let mut accumulated = String::new();
    let saw_create_plan_start = tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("reading response body chunk failed");
            accumulated.push_str(&String::from_utf8_lossy(&chunk));
            let reached = parse_sse_frames(&accumulated).iter().any(|f| {
                f.event.as_deref() == Some("aura.tool_start")
                    && serde_json::from_str::<serde_json::Value>(&f.data)
                        .is_ok_and(|v| v["tool_name"].as_str() == Some("create_plan"))
            });
            if reached {
                return true;
            }
        }
        false
    })
    .await
    .expect("sync point did not arrive within 30s");
    assert!(
        saw_create_plan_start,
        "the stream ended before the first aura.tool_start for 'create_plan'"
    );

    // Sanity prelude: the loop is provably running.
    let at_sync = calls.load(Ordering::SeqCst);
    assert!(
        at_sync >= 1,
        "provider call counter was {at_sync} at the sync point — the loop is not running"
    );

    // The disconnect: stop reading and drop the stream AND the response.
    // `bytes_stream` consumes the response, so the stream IS the response
    // client-side; dropping it drops both. With no pooled idle connection,
    // this closes the connection outright.
    drop(stream);

    // Observation window: two samples, sized for loaded CI. A loop that is
    // still live keeps calling the provider and the count climbs; a loop
    // that stopped on the disconnect freezes the count.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after_window = calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(750)).await;
    let after_gap = calls.load(Ordering::SeqCst);
    assert_eq!(
        after_window, after_gap,
        "the provider kept being called after the client disconnect \
         (count at the 2s sample: {after_window}, \
         count at the 750ms-later sample: {after_gap})"
    );

    // The coordinator task must have ended ON ITS OWN within the burn
    // window: `NothingLive`. Pre-fix this is `Settled { aborted: 1 }` — the
    // task was still live after the disconnect, which IS the burn window.
    assert_eq!(
        live.abort_and_settle(Duration::from_secs(2)).await,
        ShutdownAbort::NothingLive,
        "the coordinator task was still live 2s after the client disconnect — \
         the burn window never closed"
    );
}

// ---------------------------------------------------------------------------
// S102 named-event surface: worker tool events, reasoning, plan, context
// ---------------------------------------------------------------------------

use agent_driver_rs::TokenUsage;
use agent_driver_rs::streaming::{
    CompletionMetadata, ContentBlockType, StopReason, StreamDelta, StreamEvent,
};

/// A no-tool text response that carries a thinking delta before the text and
/// reports token usage on its terminal `Completed` event, so the metering
/// decorator has a per-call usage to record (the stock mock helpers leave
/// `usage: None`).
fn thinking_text_response_with_usage(
    thinking: &str,
    text: &str,
    usage: TokenUsage,
) -> Vec<StreamEvent> {
    let mut started = CompletionMetadata::default();
    started.stop_reason = None;
    let mut completed = CompletionMetadata::default();
    completed.stop_reason = Some(StopReason::EndTurn);
    completed.usage = Some(usage);
    vec![
        StreamEvent::Started { metadata: started },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Thinking,
        },
        StreamEvent::Delta(StreamDelta::ThinkingDelta {
            thinking: thinking.to_owned(),
        }),
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::ContentBlockStart {
            index: 1,
            block_type: ContentBlockType::Text,
        },
        StreamEvent::Delta(StreamDelta::TextDelta {
            text: text.to_owned(),
        }),
        StreamEvent::ContentBlockStop { index: 1 },
        StreamEvent::Completed {
            metadata: completed,
        },
    ]
}

/// The worker's final-turn usage (input 60 / output 6) and the coordinator's
/// (70 / 9): the per-agent `aura.context_usage` values the test asserts.
const WORKER_FINAL_USAGE: TokenUsage = TokenUsage {
    input_tokens: 60,
    output_tokens: 6,
};
const COORDINATOR_FINAL_USAGE: TokenUsage = TokenUsage {
    input_tokens: 70,
    output_tokens: 9,
};

const WORKER_THINKING: &str = "operations is checking the service logs";
const COORDINATOR_THINKING: &str = "coordinator frames the answer";

/// The S102 provider script: same six-call shape as [`shim_provider`], with
/// the two end-turn responses replaced by thinking-plus-usage-bearing ones
/// so the reasoning and context-usage paths have live data to carry.
fn s102_provider() -> Arc<dyn Provider> {
    let expected_id = PlanId::derive(&one_task_plan_args());
    let plan_args_json = serde_json::to_string(&one_task_plan_args()).expect("plan args serialize");
    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &plan_args_json),
        mock_tool_call_response(
            "c2",
            "execute",
            &format!(r#"{{"plan_id":"{expected_id}"}}"#),
        ),
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("Found 42 errors", "service-a: 42", "high"),
        ),
        thinking_text_response_with_usage(WORKER_THINKING, "", WORKER_FINAL_USAGE),
        mock_tool_call_response(
            "c3",
            "respond",
            r#"{"response":"service-a produced 42 of yesterday's errors."}"#,
        ),
        thinking_text_response_with_usage(COORDINATOR_THINKING, "done.", COORDINATOR_FINAL_USAGE),
    ];
    Arc::new(MockProvider::new(responses))
}

/// The S102 card's named-event surface on the wire: worker tool calls under
/// their task with the worker's agent id, coordinator and worker reasoning
/// as named events, `plan_created` when `create_plan` completes, and
/// per-agent `context_usage` at each agent's final turn — with the assistant
/// answer stream staying byte-clean (C3).
#[tokio::test]
async fn s102_named_events_reach_the_stream() {
    let provider = s102_provider();
    let dir = tempfile::TempDir::new().expect("temp dir for artifact root");
    let state = shim_state(provider, dir.path().to_path_buf());
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::new();
    let request_body = serde_json::json!({
        "model": "aura-terminalbench",
        "messages": [{"role": "user", "content": "Count the errors by service"}],
        "stream": true,
    });
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
            .json(&request_body)
            .send(),
    )
    .await
    .expect("request did not complete within 30s")
    .expect("POST /v1/chat/completions failed");
    assert!(response.status().is_success());

    let text = tokio::time::timeout(Duration::from_secs(30), response.text())
        .await
        .expect("body did not complete within 30s — stream did not terminate via [DONE]")
        .expect("reading response body failed");

    let frames = parse_sse_frames(&text);
    let transcript = transcript_summary(&frames);
    let json = |f: &SseFrame| serde_json::from_str::<serde_json::Value>(&f.data);
    let position = |name: &str, pred: &dyn Fn(&serde_json::Value) -> bool| {
        frames
            .iter()
            .position(|f| f.event.as_deref() == Some(name) && json(f).is_ok_and(|v| pred(&v)))
    };
    let mut failures: Vec<String> = Vec::new();

    // (a) plan_created fires when create_plan completes: mapped goal,
    //     flattened task count, planning rationale, routing always
    //     orchestrated, no planning_response field, agent "main". It lands
    //     after the coordinator's tool_complete for create_plan and before
    //     the first task_started.
    let plan_pos = position("aura.orchestrator.plan_created", &|v| {
        v["goal"].as_str() == Some("Count the errors by service")
            && v["task_count"].as_u64() == Some(1)
            && v["routing_mode"].as_str() == Some("orchestrated")
            && v["routing_rationale"].as_str() == Some("One step")
            && v.get("planning_response").is_none()
            && v["agent_id"].as_str() == Some("main")
    });
    if plan_pos.is_none() {
        failures.push(
            "(a) no aura.orchestrator.plan_created with goal/task_count/routing fields".to_owned(),
        );
    }
    let create_plan_complete = position("aura.tool_complete", &|v| {
        v["tool_name"].as_str() == Some("create_plan")
    });
    let task_started = position("aura.orchestrator.task_started", &|_| true);
    if let (Some(plan), Some(complete), Some(started)) =
        (plan_pos, create_plan_complete, task_started)
        && !(complete < plan && plan < started)
    {
        failures.push(format!(
            "(a) plan_created ordering wrong: tool_complete@{complete}, plan@{plan}, task_started@{started}"
        ));
    }

    // (b) worker tool events carry the worker's agent id and task id; the
    //     completed payload carries NO tool_name and NO worker_id (the
    //     aura-events shape), and both sit inside the task's window.
    let worker_tool_start = position("aura.orchestrator.tool_call_started", &|v| {
        v["tool_name"].as_str() == Some("submit_result")
            && v["worker_id"].as_str() == Some("operations")
            && v["task_id"].as_u64() == Some(0)
            && v["agent_id"].as_str() == Some("operations")
            && v["arguments"].is_object()
    });
    if worker_tool_start.is_none() {
        failures.push(
            "(b) no aura.orchestrator.tool_call_started for submit_result with worker identity"
                .to_owned(),
        );
    }
    let worker_tool_complete = position("aura.orchestrator.tool_call_completed", &|v| {
        v["tool_call_id"].is_string()
            && v["success"].as_bool() == Some(true)
            && v["result"].is_string()
            && v["duration_ms"].is_u64()
            && v["task_id"].as_u64() == Some(0)
            && v["agent_id"].as_str() == Some("operations")
    });
    match worker_tool_complete {
        None => failures.push(
            "(b) no aura.orchestrator.tool_call_completed with success/result/duration".to_owned(),
        ),
        Some(pos) => {
            let v = json(&frames[pos]).expect("checked by position predicate");
            if v.get("tool_name").is_some() || v.get("worker_id").is_some() {
                failures.push(
                    "(b) tool_call_completed carries tool_name/worker_id — aura-events shape has neither"
                        .to_owned(),
                );
            }
        }
    }
    let task_completed = position("aura.orchestrator.task_completed", &|_| true);
    if let (Some(start), Some(complete), Some(t_started), Some(t_completed)) = (
        worker_tool_start,
        worker_tool_complete,
        task_started,
        task_completed,
    ) && !(t_started < start && start < complete && complete < t_completed)
    {
        failures.push(format!(
            "(b) worker tool events outside the task window: task_started@{t_started}, start@{start}, complete@{complete}, task_completed@{t_completed}"
        ));
    }

    // (c) reasoning surfaces as NAMED events: worker thinking under
    //     worker_reasoning with task and worker identity, coordinator
    //     thinking under aura.reasoning with agent "main".
    let worker_reasoning = position("aura.orchestrator.worker_reasoning", &|v| {
        v["task_id"].as_u64() == Some(0)
            && v["worker_id"].as_str() == Some("operations")
            && v["content"]
                .as_str()
                .is_some_and(|c| c.contains("service logs"))
            && v["agent_id"].as_str() == Some("operations")
    });
    if worker_reasoning.is_none() {
        failures.push("(c) no worker_reasoning carrying the worker's thinking".to_owned());
    }
    let coordinator_reasoning = position("aura.reasoning", &|v| {
        v["agent_id"].as_str() == Some("main")
            && v["content"]
                .as_str()
                .is_some_and(|c| c.contains("frames the answer"))
    });
    if coordinator_reasoning.is_none() {
        failures.push("(c) no aura.reasoning carrying the coordinator's thinking".to_owned());
    }

    // (d) context_usage reports each agent's final-turn occupancy: the
    //     worker's from its end turn, "main"'s after aura.usage at stream
    //     end. No context_window field (the shim configures none).
    let worker_ctx = position("aura.context_usage", &|v| {
        v["agent_id"].as_str() == Some("operations")
            && v["context_tokens"].as_u64() == Some(60)
            && v["response_tokens"].as_u64() == Some(6)
            && v.get("context_window").is_none()
    });
    if worker_ctx.is_none() {
        failures.push("(d) no context_usage for the worker's final turn (60/6)".to_owned());
    }
    let usage_pos = position("aura.usage", &|_| true);
    let main_ctx = position("aura.context_usage", &|v| {
        v["agent_id"].as_str() == Some("main")
            && v["context_tokens"].as_u64() == Some(70)
            && v["response_tokens"].as_u64() == Some(9)
    });
    match (main_ctx, usage_pos) {
        (None, _) => failures.push("(d) no context_usage for main's final turn (70/9)".to_owned()),
        (Some(ctx), Some(usage)) if ctx < usage => failures.push(
            "(d) main's context_usage landed before aura.usage — the terminal order is usage first"
                .to_owned(),
        ),
        _ => {}
    }

    // (e) C3: the answer stream stays byte-clean. No chat chunk's content
    //     carries a thinking marker; the plain answer text does arrive.
    for f in &frames {
        if f.event.is_none()
            && f.data != "[DONE]"
            && let Ok(v) = json(f)
        {
            let content = v["choices"][0]["delta"]["content"].as_str().unwrap_or("");
            if content.contains(WORKER_THINKING) || content.contains(COORDINATOR_THINKING) {
                failures.push("(e) reasoning leaked into choices[0].delta.content".to_owned());
            }
        }
    }
    let answer_arrived = frames.iter().any(|f| {
        f.event.is_none()
            && json(f).is_ok_and(|v| v["choices"][0]["delta"]["content"].as_str() == Some("done."))
    });
    if !answer_arrived {
        failures.push("(e) the plain answer text never arrived as a content chunk".to_owned());
    }

    assert!(
        failures.is_empty(),
        "S102 named-event assertions failed ({n} failures):\n---\n{fails}\n---\n\
         Wire transcript: {transcript:?}\nRaw body:\n{text}",
        n = failures.len(),
        fails = failures
            .iter()
            .map(|f| format!("  - {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
