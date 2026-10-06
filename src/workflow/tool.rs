//! The `propose_workflow` coordinator tool.
//!
//! When the `[workflow]` section has no `approval_url`, the tool is
//! propose-only: validate a workflow against the discovered MCP tool
//! inventory (W1 rules), render the human digest, and return it as the
//! tool observation.
//!
//! When `approval_url` is present, the tool additionally POSTs the notify
//! payload, blocks on the approval hold, and drives the W3 executor only
//! on [`ApprovalOutcome::Approved`].  Deny, timeout, and cancellation all
//! return as ordinary tool observations the coordinator replans against.

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult, ToolSchema};
use agent_driver_rs::types::ToolName;
use async_trait::async_trait;
use serde::Deserialize;

use crate::mcp_client::SidecarClient;

use super::approval::{ApprovalClient, ApprovalOutcome, ApprovalPayload, apply_authorized};
use super::plan::WorkflowSpec;
use super::render::render_digest;

/// The wire arguments for `propose_workflow`.
///
/// The workflow itself arrives as the `workflow` property so the tool schema
/// can describe it without conflating it with future tool-level options.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposeWorkflowArgs {
    pub workflow: WorkflowSpec,
}

/// Propose a workflow for human approval.
///
/// Holds the sidecar client so it can re-list the discovered inventory at
/// propose time; the W1 validator needs the full `tools/list` shape (name
/// plus `inputSchema`) to check step arguments.
pub struct ProposeWorkflowTool {
    definition: ToolDefinition,
    sidecar: SidecarClient,
    approval: Option<ApprovalClient>,
    session_id: String,
}

impl ProposeWorkflowTool {
    /// Short summary rendered in the coordinator preamble's tools section
    /// when the workflow seam is mounted.
    pub const PREAMBLE_SUMMARY: &'static str =
        "Propose a pre-authorized workflow of MCP tool calls for human approval.";

    /// Longer summary rendered in the loop planning wrapper's tools section
    /// when the workflow seam is mounted.
    pub const PLANNING_LOOP_SUMMARY: &'static str = "Propose a pre-authorized workflow of MCP tool calls for human approval. Call this when \
         the user request is a repeatable sequence of external operations.";

    /// Build the tool with the sidecar it lists tools through.
    pub fn new(sidecar: SidecarClient) -> Self {
        Self {
            definition: Self::definition(),
            sidecar,
            approval: None,
            session_id: "unknown".to_owned(),
        }
    }

    /// Attach the approval-wire client.
    ///
    /// This is the seam that turns propose-only mode into the blocking
    /// approval path.  Call sites that do not set an approval client keep
    /// the W2 propose-only behavior.
    pub fn with_approval(mut self, approval: ApprovalClient) -> Self {
        self.approval = Some(approval);
        self
    }

    /// Set the shim session id that appears in the notify payload.
    ///
    /// The default `"unknown"` is the seam for production call sites that
    /// know the request's session id.
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = session_id;
        self
    }

    /// The `propose_workflow` definition.
    pub fn definition() -> ToolDefinition {
        let name =
            ToolName::new("propose_workflow").expect("propose_workflow is a non-empty identifier");
        let schema = ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "workflow": {
                    "type": "object",
                    "description": "The workflow to propose: a goal and an ordered list of steps.",
                    "properties": {
                        "goal": { "type": "string" },
                        "steps": { "type": "array" }
                    },
                    "required": ["goal", "steps"]
                }
            },
            "required": ["workflow"]
        }))
        .expect("propose_workflow schema is a JSON object literal");
        ToolDefinition::new(
            name,
            "Propose a pre-authorized workflow of MCP tool calls for human approval. \
             Validates the workflow against the discovered tool inventory and returns \
             a rendered digest; nothing executes.",
            schema,
        )
    }
}

#[async_trait]
impl Tool for ProposeWorkflowTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, input: &ToolInput, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ProposeWorkflowArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "propose_workflow arguments did not parse: {error}"
                )));
            }
        };

        let tools = match self.sidecar.list_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "propose_workflow could not list the discovered tool inventory: {error}"
                )));
            }
        };

        let validated = match args.workflow.validate(&tools) {
            Ok(validated) => validated,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        let Some(client) = &self.approval else {
            // Propose-only mode: W2 behavior, unchanged.
            return Ok(ToolResult::text(render_digest(&validated)));
        };

        // Approval-gated mode: the human must approve this exact instance
        // before the W3 executor applies it.
        let rendered = render_digest(&validated);
        let payload = ApprovalPayload::new(
            validated.spec().clone(),
            rendered,
            self.session_id.clone(),
            client.decision_id_policy(),
        );

        // The notify POST rides under the request's cancellation: a
        // cancellation observed during the POST is a Cancelled observation
        // (never an unbounded wait on a stalled transport - the client's
        // request timeout bounds the wire independently).
        let hold = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Ok(ToolResult::error("workflow approval cancelled".to_owned()));
            }
            notified = client.notify(payload) => match notified {
                Ok(hold) => hold,
                Err(error) => return Ok(ToolResult::error(error.to_string())),
            },
        };

        // Cancellation-classification note: a cancel during the hold is an
        // error observation, while a cancel during the apply leg surfaces
        // as W3's RunRecord (RunOutcome::Cancelled) text observation.
        // The asymmetry is deliberate - the hold has no run to report,
        // the apply leg does.
        match hold.outcome(&ctx.cancellation).await {
            Ok(ApprovalOutcome::Approved(approved)) => {
                match apply_authorized(approved, &validated, &self.sidecar, &ctx.cancellation).await
                {
                    Ok(record) => Ok(ToolResult::text(format!("{record:?}"))),
                    Err(error) => Ok(ToolResult::error(error.to_string())),
                }
            }
            Ok(other) => Ok(ToolResult::error(approval_observation(other))),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

/// Render a non-approved terminal outcome as the coordinator observation.
fn approval_observation(outcome: ApprovalOutcome) -> String {
    match outcome {
        ApprovalOutcome::Approved(_) => unreachable!("the approved arm is handled by the witness"),
        ApprovalOutcome::Denied { reason } => match reason {
            Some(reason) => format!("workflow denied: {reason}"),
            None => "workflow denied: no reason given".to_owned(),
        },
        ApprovalOutcome::TimedOut => "workflow approval timed out".to_owned(),
        ApprovalOutcome::Cancelled => "workflow approval cancelled".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// A context carrying a fresh, never-cancelled token: execute reads
    /// nothing else from it.
    fn ctx() -> ToolContext {
        ToolContext::new(CancellationToken::new())
    }

    /// The wire arguments for one proposal.
    fn proposal(workflow: serde_json::Value) -> ToolInput {
        ToolInput::from_value(json!({ "workflow": workflow }))
            .expect("a proposal input is an object")
    }

    // ---- Offline: no live sidecar ---------------------------------------

    #[tokio::test]
    async fn malformed_arguments_reach_the_model_as_an_error_observation() {
        let tool = ProposeWorkflowTool::new(SidecarClient::disconnected());
        // No `workflow` property at all: the parse fails before anything
        // else runs.
        let input = ToolInput::from_value(json!({})).expect("an empty object is a ToolInput");

        let result = tool
            .execute(&input, &ctx())
            .await
            .expect("a malformed parse is an observation, never a ToolError");

        assert!(result.is_error(), "{result:?}");
        assert!(
            result
                .content()
                .contains("propose_workflow arguments did not parse"),
            "{}",
            result.content()
        );
    }

    #[tokio::test]
    async fn a_sidecar_failure_names_the_failure_in_the_observation() {
        let tool = ProposeWorkflowTool::new(SidecarClient::disconnected());
        // `disconnected()` fails every `tools/list`, so what reaches the
        // model is the sidecar failure, naming itself — not a workflow
        // verdict invented over an inventory nobody read. A workflow
        // rejection needs a live inventory; the rig tests below cover it.
        let input = proposal(json!({ "goal": "g", "steps": [] }));

        let result = tool
            .execute(&input, &ctx())
            .await
            .expect("a sidecar failure is an observation, never a ToolError");

        assert!(result.is_error(), "{result:?}");
        let observation = result.content();
        assert!(
            observation.contains("could not list the discovered tool inventory"),
            "{observation}"
        );
        assert!(
            observation.contains("sidecar connection failed"),
            "{observation}"
        );
        assert!(
            !observation.contains("no steps"),
            "validation never ran against a failed inventory: {observation}"
        );
    }

    // ---- Through the offline rig: a live discovered inventory -----------

    /// A scripted inventory server: `tools/list` answers with a fixed
    /// list; nothing else is served, because propose-only never calls a
    /// tool.
    struct ScriptedInventory {
        info: rmcp::model::ServerInfo,
        tools: Vec<rmcp::model::Tool>,
    }

    impl rmcp::ServerHandler for ScriptedInventory {
        fn get_info(&self) -> rmcp::model::ServerInfo {
            self.info.clone()
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
            Ok(rmcp::model::ListToolsResult {
                tools: self.tools.clone(),
                ..rmcp::model::ListToolsResult::default()
            })
        }
    }

    /// One `tools/list` entry: name, description, and `inputSchema`,
    /// exactly as a real server sends.
    fn tool_entry(name: &str, description: &str, schema: serde_json::Value) -> serde_json::Value {
        json!({
            "name": name,
            "description": description,
            "inputSchema": schema
        })
    }

    /// Serve the scripted inventory over one half of an in-memory duplex
    /// and connect a real client over the other. The returned handle
    /// keeps the server alive: dropping it drops the server session
    /// mid-handshake, which is why every caller binds it for the test's
    /// duration (the integration rig's same shape and same comment).
    async fn boot(
        entries: &[serde_json::Value],
    ) -> (
        SidecarClient,
        tokio::task::JoinHandle<rmcp::service::RunningService<rmcp::RoleServer, ScriptedInventory>>,
    ) {
        use rmcp::ServiceExt as _;
        use rmcp::transport::IntoTransport as _;

        let tools = entries
            .iter()
            .map(|entry| {
                serde_json::from_value(entry.clone())
                    .expect("a rig tool entry deserializes as an rmcp Tool")
            })
            .collect();
        let server = ScriptedInventory {
            info: rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            ),
            tools,
        };
        let (client_half, server_half) = tokio::io::duplex(4096);
        let served = tokio::spawn(async move {
            server
                .serve(server_half.into_transport())
                .await
                .expect("the rig server serves its half")
        });
        let client = SidecarClient::connect_stream(client_half)
            .await
            .expect("the rig client completes the handshake");
        (client, served)
    }

    #[tokio::test]
    async fn a_valid_proposal_returns_the_rendered_digest_as_the_observation() {
        let (sidecar, _served) = boot(&[tool_entry(
            "ops_get_cluster_state",
            "Read cluster state.",
            json!({ "type": "object" }),
        )])
        .await;
        let tool = ProposeWorkflowTool::new(sidecar);
        let input = proposal(json!({
            "goal": "Read the current cluster state",
            "steps": [{
                "id": "state",
                "tool": "ops_get_cluster_state",
                "args": { "app": "payments" }
            }]
        }));

        let result = tool
            .execute(&input, &ctx())
            .await
            .expect("propose_workflow returns every outcome as an observation");

        assert!(result.is_success(), "{result:?}");
        let digest = result.content();
        assert!(
            digest.contains("Workflow proposal: Read the current cluster state"),
            "{digest}"
        );
        assert!(digest.contains("Step 1: state"), "{digest}");
        assert!(
            digest.contains("undo: nothing to undo (rollback: null)"),
            "{digest}"
        );
    }

    #[tokio::test]
    async fn a_validation_rejection_reaches_the_model_as_an_error_observation() {
        let (sidecar, _served) = boot(&[tool_entry(
            "ops_get_cluster_state",
            "Read cluster state.",
            json!({ "type": "object" }),
        )])
        .await;
        let tool = ProposeWorkflowTool::new(sidecar);
        let input = proposal(json!({ "goal": "g", "steps": [] }));

        let result = tool
            .execute(&input, &ctx())
            .await
            .expect("a validation rejection is an observation, never a ToolError");

        assert!(result.is_error(), "{result:?}");
        assert!(
            result
                .content()
                .contains("workflow has no steps to authorize"),
            "{}",
            result.content()
        );
    }
}
