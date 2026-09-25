//! S112: the worker tool mount. Which tools a worker session carries is
//! a function of the roster's advertised names and the startup
//! discovery — pinned here at the mount's seam, cheapest first:
//!
//! 1. Resolution against hand-built discovery (disconnected client;
//!    nothing executes).
//! 2. Execution through the in-process rmcp rig: a real client session
//!    (handshake, `tools/list`, `tools/call`) against a scripted
//!    server over an in-memory duplex. No network, no live provider.

use std::collections::HashMap;
use std::sync::Arc;

use agent_driver_prototype::artifacts::ArtifactStore;
use agent_driver_prototype::coordinator_loop::{TerminalSlot, WorkerRoster};
use agent_driver_prototype::dag_executor::WorkerToolMount;
use agent_driver_prototype::mcp_client::{SidecarClient, SidecarTool, SidecarToolName};
use agent_driver_prototype::producers::ToolInventory;
use agent_driver_prototype::{
    bounding::ToolListLimit,
    config::{OrchestrationConfig, WorkerConfig},
};
use agent_driver_rs::tool::{Tool, ToolContext, ToolFormat, ToolInput};
use serde_json::{Value as JsonValue, json};
use tokio_util::sync::CancellationToken;

// ============================================================================
// Fixtures
// ============================================================================

/// The advertised schema surface the rig and the resolution tests share.
fn echo_schema() -> JsonValue {
    json!({
        "type": "object",
        "properties": {
            "text": { "type": "string", "minLength": 1 }
        },
        "required": ["text"]
    })
}

fn discovered_tool(name: &str, description: &str) -> SidecarTool {
    SidecarTool::new(
        SidecarToolName::new(name).expect("non-empty fixture name"),
        description.to_owned(),
        echo_schema(),
    )
}

/// A roster spec for a single worker whose `mcp_filter` is exactly
/// `names`, so the roster resolves (and advertises) those names.
fn spec_advertising(names: &[&str]) -> agent_driver_prototype::coordinator_loop::WorkerSpec {
    let mut workers = HashMap::new();
    workers.insert(
        "w".to_owned(),
        WorkerConfig {
            description: "rig worker".to_owned(),
            preamble: String::new(),
            mcp_filter: names.iter().map(|n| n.to_string()).collect(),
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
    let roster = WorkerRoster::from_config(
        &config,
        ToolListLimit::new(names.len().max(1)),
        &[],
        &ToolInventory::from_names(names.iter().copied()),
    )
    .expect("valid worker config");
    roster
        .workers()
        .first()
        .expect("one configured worker")
        .clone()
}

/// A mount over a disconnected client and a scratch artifact store.
fn mount_over(discovered: Vec<SidecarTool>) -> WorkerToolMount {
    WorkerToolMount::new(
        SidecarClient::disconnected(),
        ArtifactStore::new(std::path::PathBuf::from(
            "/tmp/agent-driver-prototype-mount-unused",
        )),
        discovered,
    )
}

fn tool_names(tools: &[Arc<dyn Tool>]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.definition().name.as_str().to_owned())
        .collect()
}

// ============================================================================
// Resolution
// ============================================================================

const QUARTET: [&str; 4] = [
    "keystrokes",
    "capture-pane",
    "read_artifact",
    "submit_result",
];

/// A task the roster names no worker for mounts the native quartet and
/// nothing else: with no spec there is no filter, and unfiltered MCP
/// access is a coordinator-side concern (S103), never a worker default.
#[test]
fn an_unassigned_task_mounts_the_quartet_alone() {
    let mount = mount_over(vec![discovered_tool("search_logs", "Search logs.")]);
    let tools = mount.session_tools(None, TerminalSlot::new());
    assert_eq!(tool_names(&tools), QUARTET);
}

/// A filter-matched discovered tool mounts after the quartet, in spec
/// order, carrying the description and schema the server advertised.
#[test]
fn a_matched_discovered_tool_mounts_with_its_advertised_surface() {
    let mount = mount_over(vec![
        discovered_tool("search_logs", "Full-text log search."),
        discovered_tool("tail_logs", "Tail recent log lines."),
    ]);
    let spec = spec_advertising(&["search_logs", "tail_logs"]);

    let tools = mount.session_tools(Some(&spec), TerminalSlot::new());

    assert_eq!(
        tool_names(&tools),
        [
            "keystrokes",
            "capture-pane",
            "read_artifact",
            "submit_result",
            "search_logs",
            "tail_logs"
        ],
        "natives first, then the advertised set in spec order"
    );

    let tail = &tools[5];
    let json = ToolFormat::Claude.serialize_tool(tail.definition());
    assert_eq!(json["name"], "tail_logs");
    assert_eq!(json["description"], "Tail recent log lines.");
    assert_eq!(json["input_schema"], echo_schema());
}

/// A native name beats a discovered collision: the hand-written
/// description and argument validation stay on the wire, byte-identical
/// to the pre-S112 quartet.
#[test]
fn a_native_name_beats_a_discovered_collision() {
    let mount = mount_over(vec![discovered_tool(
        "keystrokes",
        "A re-described keystrokes that must not win.",
    )]);
    let spec = spec_advertising(&["keystrokes"]);

    let tools = mount.session_tools(Some(&spec), TerminalSlot::new());

    assert_eq!(tool_names(&tools), QUARTET, "no duplicate, no shadowing");
    let json = ToolFormat::Claude.serialize_tool(tools[0].definition());
    assert_eq!(
        json["description"],
        "Send keystrokes to the tmux session. Use tmux-style escape sequences for special characters (e.g. C-c for ctrl-c). Set append_enter to execute a bash command.",
        "the native description wins over the discovered one"
    );
}

/// An advertised name with no runtime backing mounts nothing: the
/// `vector_search_{store}` config mirrors are roster-only until S104's
/// deferred vector-store work lands.
#[test]
fn an_unbacked_advertised_name_mounts_nothing() {
    let mount = mount_over(Vec::new());
    let spec = spec_advertising(&["vector_search_docs"]);
    let tools = mount.session_tools(Some(&spec), TerminalSlot::new());
    assert_eq!(tool_names(&tools), QUARTET);
}

/// A discovered name the crate's `ToolName` gate rejects is skipped with
/// the rest intact — the same failure containment the library's
/// `McpToolWrapper` applies.
#[test]
fn an_invalid_discovered_name_is_skipped() {
    let mount = mount_over(vec![
        discovered_tool("ns/tool", "Rejected by the ToolName gate."),
        discovered_tool("search_logs", "Search logs."),
    ]);
    let spec = spec_advertising(&["ns/tool", "search_logs"]);

    let tools = mount.session_tools(Some(&spec), TerminalSlot::new());

    assert_eq!(
        tool_names(&tools),
        [
            "keystrokes",
            "capture-pane",
            "read_artifact",
            "submit_result",
            "search_logs"
        ],
        "the invalid name costs only itself"
    );
}

// ============================================================================
// The rig: an in-process rmcp server over an in-memory duplex
// ============================================================================

mod rig {
    use std::sync::Arc;

    use rmcp::{
        ErrorData as McpError, RoleServer, ServerHandler, ServiceExt as _,
        model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
            ServerCapabilities, ServerInfo,
        },
        service::RequestContext,
    };
    use serde_json::{Value as JsonValue, json};
    use tokio::sync::Mutex;

    use agent_driver_prototype::mcp_client::SidecarClient;

    /// A call the rig server received: tool name and argument map.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RecordedCall {
        pub name: String,
        pub arguments: JsonValue,
    }

    /// A scripted MCP server: advertises a fixed tool list, answers every
    /// `tools/call` with an echo of the recorded call.
    #[derive(Clone)]
    pub struct EchoServer {
        info: ServerInfo,
        tools: Vec<JsonValue>,
        calls: Arc<Mutex<Vec<RecordedCall>>>,
    }

    impl EchoServer {
        pub fn new(tools: Vec<JsonValue>) -> Self {
            Self {
                info: ServerInfo::new(ServerCapabilities::builder().enable_tools().build()),
                tools,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub async fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().await.clone()
        }
    }

    impl ServerHandler for EchoServer {
        fn get_info(&self) -> ServerInfo {
            self.info.clone()
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, McpError> {
            let tools = self
                .tools
                .iter()
                .map(|raw| {
                    serde_json::from_value(raw.clone())
                        .expect("rig tool fixtures deserialize as rmcp Tool")
                })
                .collect();
            Ok(ListToolsResult {
                tools,
                ..ListToolsResult::default()
            })
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, McpError> {
            let arguments = request.arguments.clone().unwrap_or_default();
            self.calls.lock().await.push(RecordedCall {
                name: request.name.to_string(),
                arguments: serde_json::to_value(arguments).unwrap_or(JsonValue::Null),
            });
            Ok(
                CallToolResult::success(vec![ContentBlock::text(format!("echo:{}", request.name))])
                    .into(),
            )
        }
    }

    /// Serve the rig and connect a real client over the other half.
    ///
    /// The server half is spawned, not awaited: `serve` only completes
    /// once the client's initialize round trip runs, so awaiting it
    /// before connecting deadlocks the pair. The returned handle keeps
    /// the server alive; dropping or ignoring it ends it with the test.
    pub async fn boot(
        server: EchoServer,
    ) -> (
        SidecarClient,
        tokio::task::JoinHandle<rmcp::service::RunningService<rmcp::RoleServer, EchoServer>>,
    ) {
        use rmcp::transport::IntoTransport as _;

        let (client_half, server_half) = SidecarClient::duplex_pair(4096);
        let served = tokio::spawn(async move {
            server
                .serve(server_half.into_transport())
                .await
                .expect("rig server serves its half")
        });
        let client = SidecarClient::connect_stream(client_half)
            .await
            .expect("rig client completes the handshake");
        (client, served)
    }

    /// The `tools/list` entry shape the rig advertises: name,
    /// description, and `inputSchema`, exactly as a real server sends.
    pub fn tool_entry(name: &str, description: &str, schema: JsonValue) -> JsonValue {
        json!({ "name": name, "description": description, "inputSchema": schema })
    }
}

// ============================================================================
// Execution through the rig
// ============================================================================

/// A discovered tool executes end to end through the real client path:
/// the rig's `tools/list` is the mount's discovery, and the model-side
/// call round-trips to a recorded `tools/call` with the arguments intact.
#[tokio::test]
async fn a_discovered_tool_executes_through_the_rmcp_pair() {
    let server = rig::EchoServer::new(vec![rig::tool_entry("echo_tool", "Echo.", echo_schema())]);
    let (client, _served) = rig::boot(server.clone()).await;

    let discovered = client.list_tools().await.expect("tools/list over the rig");
    assert_eq!(
        discovered.len(),
        1,
        "the mount's discovery is the rig's list"
    );

    let mount = WorkerToolMount::new(
        client.clone(),
        ArtifactStore::new(std::path::PathBuf::from(
            "/tmp/agent-driver-prototype-mount-unused",
        )),
        discovered,
    );
    let spec = spec_advertising(&["echo_tool"]);
    let tools = mount.session_tools(Some(&spec), TerminalSlot::new());
    assert_eq!(
        tool_names(&tools).last().map(String::as_str),
        Some("echo_tool")
    );

    let echo = &tools[4];
    let input = ToolInput::from_value(json!({ "text": "hello" })).expect("valid object input");
    let result = echo
        .execute(&input, &ToolContext::new(CancellationToken::new()))
        .await
        .expect("the call completes");

    assert!(
        matches!(&result,
            agent_driver_rs::ToolResult::Success { content } if content == "echo:echo_tool"),
        "the rig's text answer reaches the caller as tool content, got: {result:?}"
    );
    assert_eq!(
        server.calls().await,
        vec![rig::RecordedCall {
            name: "echo_tool".to_owned(),
            arguments: json!({ "text": "hello" }),
        }],
        "the wire saw the tool name and the unmangled argument map"
    );
}

/// A cancelled context errors the call before any server round trip:
/// the biased select in the wrapper checks cancellation first, matching
/// the library `McpToolWrapper`'s race.
#[tokio::test]
async fn a_cancelled_context_errors_the_call_before_the_wire() {
    let server = rig::EchoServer::new(vec![rig::tool_entry("echo_tool", "Echo.", echo_schema())]);
    let (client, _served) = rig::boot(server.clone()).await;
    let discovered = client.list_tools().await.expect("tools/list over the rig");

    let mount = WorkerToolMount::new(
        client,
        ArtifactStore::new(std::path::PathBuf::from(
            "/tmp/agent-driver-prototype-mount-unused",
        )),
        discovered,
    );

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let input = ToolInput::from_value(json!({ "text": "late" })).expect("valid object input");
    let result = mount
        .session_tools(Some(&spec_advertising(&["echo_tool"])), TerminalSlot::new())
        .into_iter()
        .find(|tool| tool.definition().name.as_str() == "echo_tool")
        .expect("the mount carries the discovered tool")
        .execute(&input, &ToolContext::new(cancelled))
        .await;

    assert!(
        result.is_err(),
        "cancellation is a hard error, not a soft result"
    );
    assert!(
        server.calls().await.is_empty(),
        "a cancelled call must not reach the wire"
    );
}
