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
