//! Offline integration tests for the deterministic workflow executor.
//!
//! Six scenarios over the `test-support` scripted-server rig.

#![expect(dead_code, unused_imports)]
#![allow(clippy::unused_async)]

use agent_driver_prototype::mcp_client::{SidecarClient, SidecarTool};
use agent_driver_prototype::workflow::{
    ExportEnvironment, RunOutcome, RunRecord, StepStatus, ValidatedWorkflowSpec, WorkflowSpec,
    execute_workflow,
};
use serde_json::{Value as JsonValue, json};
use tokio_util::sync::CancellationToken;

// ============================================================================
// The rig: an in-process rmcp server over an in-memory duplex
// ============================================================================

mod rig {
    use std::sync::Arc;

    use agent_driver_prototype::mcp_client::SidecarClient;
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

    /// The scripted answer function: tool name and args in, JSON result
    /// or error string out.
    type Script = Arc<dyn Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync>;

    /// A scripted MCP server: advertises a fixed tool list and answers
    /// `tools/call` according to the supplied script.
    #[derive(Clone)]
    pub struct ScriptedServer {
        info: ServerInfo,
        tools: Vec<JsonValue>,
        script: Script,
        calls: Arc<Mutex<Vec<(String, JsonValue)>>>,
    }

    impl ScriptedServer {
        pub fn new(
            tools: Vec<JsonValue>,
            script: impl Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync + 'static,
        ) -> Self {
            Self {
                info: ServerInfo::new(ServerCapabilities::builder().enable_tools().build()),
                tools,
                script: Arc::new(script),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub async fn calls(&self) -> Vec<(String, JsonValue)> {
            self.calls.lock().await.clone()
        }
    }

    impl ServerHandler for ScriptedServer {
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
                        .expect("rig tool fixture deserializes as rmcp Tool")
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
            let args = serde_json::to_value(arguments).unwrap_or(JsonValue::Null);
            self.calls
                .lock()
                .await
                .push((request.name.to_string(), args.clone()));
            match (self.script)(&request.name, &args) {
                Ok(value) => {
                    Ok(CallToolResult::success(vec![ContentBlock::text(value.to_string())]).into())
                }
                Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into()),
            }
        }
    }

    /// Serve the rig and connect a real client over the other half.
    ///
    /// The server half is spawned, not awaited: `serve` only completes
    /// once the client's initialize round trip runs, so awaiting it
    /// before connecting deadlocks the pair. The returned handle keeps
    /// the server alive; dropping or ignoring it ends it with the test.
    pub async fn boot(
        server: ScriptedServer,
    ) -> (
        SidecarClient,
        tokio::task::JoinHandle<rmcp::service::RunningService<rmcp::RoleServer, ScriptedServer>>,
    ) {
        use rmcp::transport::IntoTransport as _;

        let (client_half, server_half) = tokio::io::duplex(4096);
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
// Helpers
// ============================================================================

fn permissive_schema() -> JsonValue {
    json!({ "type": "object" })
}

fn validate(workflow: JsonValue, tools: &[SidecarTool]) -> ValidatedWorkflowSpec {
    let spec: WorkflowSpec = serde_json::from_value(workflow).expect("workflow parses");
    spec.validate(tools).expect("workflow validates")
}

// ============================================================================
// Acceptance scenarios
// ============================================================================

#[tokio::test]
async fn success_steps_apply_in_order_and_exports_captured() {
    todo!()
}

#[tokio::test]
async fn step_failure_runs_reverse_order_unwind() {
    todo!()
}

#[tokio::test]
async fn rollback_failure_reports_both_failures_and_residual_set() {
    todo!()
}

#[tokio::test]
async fn bounds_rejection_unwinds_prior_steps() {
    todo!()
}

#[tokio::test]
async fn binding_miss_missing_path_is_a_step_failure() {
    todo!()
}

#[tokio::test]
async fn cancellation_mid_apply_halts_and_records_residual_state() {
    todo!()
}
