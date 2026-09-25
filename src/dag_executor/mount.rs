//! The worker tool mount: the one seam where the roster's advertised
//! tool names become the tools a worker session actually carries.
//!
//! One rule, owned here end to end: a worker session carries the two
//! structural tools (`read_artifact`, `submit_result`) plus, for each
//! name the worker's roster spec advertises that is NOT a structural
//! name, the tool the startup `tools/list` discovered under that name.
//! Two carve-outs fall out of the same rule: a structural name beats a
//! discovered collision (a shadowed `submit_result` would cut the
//! worker off from the only result channel the loop reads), and an
//! advertised name with no runtime backing mounts nothing — that is
//! the `vector_search_{store}` config-mirror case, deliberately
//! roster-only until S104's deferred vector-store work lands. The
//! former TerminalBench tmux pair is gone by board ruling 2026-09-25:
//! the worker surface is MCP plus the structural pair.
//!
//! Library layering (S110's interim policy): the discovered half rides
//! the S106 plain-JSON `SidecarClient` surface — no rmcp type crosses
//! this seam — and mirrors the agent-driver-rs `McpToolWrapper`
//! semantics (skip invalid names with a warning, default schema
//! fallback, biased cancellation race, text extraction) so S110 can
//! delete this module rather than migrate it. One recorded divergence:
//! the crate wrapper hard-errors on transport failure; the prototype's
//! tools soft-error so the model can read the failure and recover.

use std::sync::Arc;

use agent_driver_rs::DynTool;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult, ToolSchema};
use agent_driver_rs::types::ToolName;
use async_trait::async_trait;

use crate::artifacts::ArtifactStore;
use crate::coordinator_loop::{SubmitResultTool, TerminalSlot, WorkerSpec, WorkerSubmission};
use crate::mcp_client::{SidecarClient, SidecarTool, SidecarToolArgs, SidecarToolName};

use super::tools::ReadArtifactTool;

/// The structural tool names every worker session owns. A discovered
/// tool with one of these names does not mount: the structural
/// implementation wins, because `read_artifact` is the spill-read
/// channel and `submit_result` is the worker's only result path — a
/// sidecar advertising either would cut the worker off from the loop.
pub const STRUCTURAL_TOOL_NAMES: [&str; 2] = ["read_artifact", "submit_result"];

/// Builds a worker session's tool set from the startup discovery.
///
/// The mount is constructed once from the sidecar handshake's
/// `tools/list` result and cloned per task; the per-session submission
/// slot arrives as a `session_tools` argument.
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
    /// store the structural `read_artifact` tool reads, and the tool
    /// list the startup handshake discovered.
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
    /// The structural pair (`read_artifact`, `submit_result`) always
    /// mounts; everything else a worker carries is MCP: each name the
    /// spec advertises mounts the discovered tool under that name, in
    /// spec order, unless the name is structural (structural wins) or
    /// has no discovered backing (mounts nothing — the
    /// `vector_search_*` config mirrors, roster-only until S104).
    /// `None` (a task the roster names no worker for) mounts the
    /// structural pair alone.
    pub fn session_tools(
        &self,
        spec: Option<&WorkerSpec>,
        submission_slot: TerminalSlot<WorkerSubmission>,
    ) -> Vec<DynTool> {
        let mut tools: Vec<DynTool> = vec![
            Arc::new(ReadArtifactTool::new(self.artifacts.clone())),
            Arc::new(SubmitResultTool::new(submission_slot)),
        ];
        let Some(spec) = spec else {
            return tools;
        };
        for advertised in spec.tools() {
            let name = advertised.name();
            if STRUCTURAL_TOOL_NAMES.contains(&name) {
                continue;
            }
            let Some(discovered) = self.discovered.iter().find(|t| t.name().as_str() == name)
            else {
                continue;
            };
            if let Some(tool) = DiscoveredMcpTool::new(self.sidecar.clone(), discovered) {
                tools.push(Arc::new(tool));
            }
        }
        tools
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
        let name = match ToolName::new(tool.name().as_str()) {
            Ok(name) => name,
            Err(error) => {
                tracing::warn!(
                    tool = tool.name().as_str(),
                    "skipping discovered tool the ToolName gate rejects: {error}"
                );
                return None;
            }
        };
        let schema = ToolSchema::from_value(tool.input_schema().clone()).unwrap_or_default();
        let definition = ToolDefinition::new(name, tool.description().to_owned(), schema);
        Some(Self {
            definition,
            sidecar,
            // The wire name is the discovery's own name, not the gated
            // `ToolName` rendering: the server answers to what it
            // advertised.
            tool_name: tool.name().clone(),
        })
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
        let args = SidecarToolArgs::from_map(input.inner().clone());
        // Biased race, cancellation first — the crate `McpToolWrapper`
        // pattern, so a cancelled run never waits out a server.
        let content = tokio::select! {
            biased;
            _ = ctx.cancellation.cancelled() => {
                return Err(agent_driver_rs::ToolError::ExecutionFailed {
                    tool_name: self.definition.name.clone(),
                    message: "Tool execution cancelled".to_owned(),
                });
            }
            result = self.sidecar.call_tool(&self.tool_name, &args) => {
                match result {
                    Ok(content) => content,
                    // Soft error, matching the prototype's tool set: the
                    // model reads the failure and recovers. Recorded
                    // divergence from the crate wrapper's hard error, for
                    // S110 to collapse.
                    Err(error) => return Ok(ToolResult::error(error.to_string())),
                }
            }
        };
        Ok(ToolResult::text(content.as_str().to_owned()))
    }
}
