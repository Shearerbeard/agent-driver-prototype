//! The worker tool mount: the one seam where the roster's advertised
//! tool names become the tools a worker session actually carries.
//!
//! Before S112 the worker inner loop hardcoded its four native tools
//! while the roster advertised each worker's `mcp_filter`-resolved MCP
//! tools in the coordinator's planning prompt — a discovered tool never
//! appeared in any completion request, so no model could call it. The
//! mount closes that loop: advertised == mounted == executable.
//!
//! One rule, owned here end to end: a worker session carries the four
//! native tools (`keystrokes`, `capture-pane`, `read_artifact`,
//! `submit_result`) plus, for each name the worker's roster spec
//! advertises that is NOT a native name, the tool the startup
//! `tools/list` discovered under that name. Two carve-outs fall out of
//! the same rule: a native name beats a discovered collision (the
//! hand-written descriptions and argument validation the golden corpus
//! pins stay on the wire), and an advertised name with no runtime
//! backing mounts nothing — that is the `vector_search_{store}`
//! config-mirror case, deliberately roster-only until S104's deferred
//! vector-store work lands.
//!
//! Library layering (S110's interim policy): the discovered half rides
//! the S106 plain-JSON `SidecarClient` surface — no rmcp type crosses
//! this seam — and mirrors the agent-driver-rs `McpToolWrapper`
//! semantics (skip invalid names with a warning, default schema
//! fallback, biased cancellation race, text extraction) so S110 can
//! delete this module rather than migrate it. One recorded divergence:
//! the crate wrapper hard-errors on transport failure; the prototype's
//! tools soft-error so the model can read the failure and recover.

use agent_driver_rs::DynTool;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use async_trait::async_trait;

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::{TerminalSlot, WorkerSpec, WorkerSubmission};
use crate::mcp_client::{SidecarClient, SidecarTool, SidecarToolName};

/// The names the native worker tools own. A discovered tool with one of
/// these names does not mount: the native implementation wins, keeping
/// the descriptions and argument validation the golden corpus pins.
/// `submit_result` is listed defensively — a sidecar advertising it
/// would shadow the worker's only result channel.
const NATIVE_NAMES: [&str; 4] = [
    "keystrokes",
    "capture-pane",
    "read_artifact",
    "submit_result",
];

/// Builds a worker session's tool set from the startup discovery.
///
/// The mount is constructed once from the sidecar handshake's
/// `tools/list` result and cloned per task; the per-task artifact store
/// and submission slot arrive with each session, so they are taken per
/// call rather than held.
///
/// Forbidden invalid state: a mount without a sidecar client or
/// artifact store, which would leave worker tools with no terminal to
/// drive and no artifact channel to spill through.
#[derive(Clone)]
pub struct WorkerToolMount {
    sidecar: SidecarClient,
    artifacts: ArtifactStore,
    discovered: Vec<SidecarTool>,
}

impl WorkerToolMount {
    /// Assemble a mount from the connected MCP client, the artifact
    /// store the native `read_artifact` tool reads, and the tool list
    /// the startup handshake discovered.
    pub fn new(
        sidecar: SidecarClient,
        artifacts: ArtifactStore,
        discovered: Vec<SidecarTool>,
    ) -> Self {
        Self {
            sidecar,
            artifacts,
            discovered,
        }
    }

    /// The full tool set for one worker session.
    ///
    /// The native quartet always mounts. Each further name the spec
    /// advertises mounts the discovered tool under that name, in spec
    /// order, unless the name is native (native wins) or has no
    /// discovered backing (mounts nothing — the `vector_search_*`
    /// config mirrors). `None` (a task the roster names no worker for)
    /// mounts the quartet alone.
    pub fn session_tools(
        &self,
        spec: Option<&WorkerSpec>,
        submission_slot: TerminalSlot<WorkerSubmission>,
    ) -> Vec<DynTool> {
        let _ = (spec, submission_slot);
        todo!("S112 Phase 2: resolve the advertised set against the discovery")
    }
}

/// One discovered MCP tool bridged onto a worker session.
///
/// The definition is built once from the `tools/list` entry; execution
/// forwards the raw argument map through [`SidecarClient::call_tool`].
struct DiscoveredMcpTool {
    definition: ToolDefinition,
    sidecar: SidecarClient,
    tool_name: SidecarToolName,
}

impl DiscoveredMcpTool {
    /// Bridge a discovered tool, or skip it.
    ///
    /// `None` (with a warning) when the advertised name does not parse
    /// as a [`ToolName`] — the crate's `McpToolWrapper` skips the same
    /// way, so a server naming a tool `ns/tool` costs that one tool,
    /// not the worker. A schema that does not parse as an object falls
    /// back to the empty schema, also matching the crate.
    fn new(sidecar: SidecarClient, tool: &SidecarTool) -> Option<Self> {
        let _ = (sidecar, tool);
        todo!("S112 Phase 2: build the definition from the tools/list entry")
    }
}

#[async_trait]
impl Tool for DiscoveredMcpTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        input: &ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolResult, agent_driver_rs::ToolError> {
        let _ = (input, ctx);
        todo!("S112 Phase 2: forward through the sidecar with a cancellation race")
    }
}
