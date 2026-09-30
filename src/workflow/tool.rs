//! The `propose_workflow` coordinator tool.
//!
//! Propose-only: validate a workflow against the discovered MCP tool
//! inventory (W1 rules), render the human digest, and return it as the tool
//! observation. Nothing applies; the apply path is W3.

#![allow(clippy::todo)]

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult, ToolSchema};
use agent_driver_rs::types::ToolName;
use async_trait::async_trait;
use serde::Deserialize;

use crate::mcp_client::SidecarClient;

use super::plan::WorkflowSpec;
#[expect(unused_imports, reason = "the fill's execute body calls it")]
use super::render::render_digest;
/// The wire arguments for `propose_workflow`.
///
/// The workflow itself arrives as the `workflow` property so the tool schema
/// can describe it without conflating it with future tool-level options.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[expect(dead_code, reason = "the fill's execute body deserializes into it")]
pub struct ProposeWorkflowArgs {
    pub workflow: WorkflowSpec,
}

/// Propose a workflow for human approval.
///
/// Holds the sidecar client so it can re-list the discovered inventory at
/// propose time; the W1 validator needs the full `tools/list` shape (name
/// plus `inputSchema`) to check step arguments.
#[allow(dead_code)]
pub struct ProposeWorkflowTool {
    definition: ToolDefinition,
    sidecar: SidecarClient,
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
        }
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

    async fn execute(
        &self,
        _input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        todo!(
            "W2 Layer 2: parse args, list tools, validate via WorkflowSpec::validate, render digest"
        )
    }
}
