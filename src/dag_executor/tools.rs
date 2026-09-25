//! The structural worker tool: `read_artifact`.
//!
//! `submit_result` is reused from `coordinator_loop::tools::submit_result`;
//! this module declares `read_artifact`, which reads from
//! [`ArtifactStore`]. Everything else a worker carries is MCP, mounted
//! by [`WorkerToolMount`](super::mount::WorkerToolMount) from the
//! startup discovery. The former TerminalBench tmux pair
//! (`keystrokes`, `capture-pane`) is gone by board ruling 2026-09-25:
//! the worker surface is MCP plus the structural pair, nothing else.

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::types::ToolName;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::artifacts::{ArtifactFilename, ArtifactStore, RunId};

use agent_driver_rs::tool::ToolSchema;

/// Build a native tool definition from this module's literals.
fn worker_tool_definition(name: &str, description: &str, schema: JsonValue) -> ToolDefinition {
    let Ok(name) = ToolName::new(name) else {
        unreachable!("tool names in this module are non-empty identifiers")
    };
    let Some(schema) = ToolSchema::from_value(schema) else {
        unreachable!("tool schemas in this module are JSON object literals")
    };
    ToolDefinition::new(name, description, schema)
}

// ============================================================================
// read_artifact
// ============================================================================

/// Arguments for the `read_artifact` tool.
///
/// `filename` is a raw `String` on the wire; the execute body validates it
/// through [`ArtifactFilename::new`] before it reaches the store. An
/// optional `run_id` enables cross-run reads within the same session; the
/// execute body parses it into [`RunId`](crate::artifacts::RunId) before
/// calling [`ArtifactStore::read_artifact_cross_run`]. Both fields stay raw
/// on the wire so the type mirrors the schema; the parse-at-boundary
/// pattern matches `CreatePlanArgs`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReadArtifactArgs {
    pub filename: String,
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Reads a spilled result artifact by filename.
///
/// Mounted on worker sessions so a worker can pull a prior task's full
/// result on demand. The artifact store's cross-run guard prevents path
/// traversal.
pub struct ReadArtifactTool {
    definition: ToolDefinition,
    store: ArtifactStore,
}

impl ReadArtifactTool {
    /// Mount the read-artifact tool over an artifact store.
    pub fn new(store: ArtifactStore) -> Self {
        Self {
            definition: worker_tool_definition(
                "read_artifact",
                "Read the full content of a result artifact by filename. \
                 Supply run_id to read an artifact from a prior run in this \
                 session.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "The artifact filename."
                        },
                        "run_id": {
                            "type": "string",
                            "description": "Run ID for cross-run artifact access. \
                                           Omit to read from the current run."
                        }
                    },
                    "required": ["filename"]
                }),
            ),
            store,
        }
    }
}

#[async_trait]
impl Tool for ReadArtifactTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: ReadArtifactArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "read_artifact arguments did not parse: {error}"
                )));
            }
        };
        let filename = match ArtifactFilename::new(&args.filename) {
            Ok(f) => f,
            Err(error) => {
                return Ok(ToolResult::error(error.to_string()));
            }
        };
        let result = match &args.run_id {
            Some(raw_run_id) => match RunId::new(raw_run_id) {
                Ok(run_id) => self.store.read_artifact_cross_run(&filename, &run_id).await,
                Err(error) => return Ok(ToolResult::error(error.to_string())),
            },
            None => self.store.read_artifact(&filename).await,
        };
        match result {
            Ok(content) => Ok(ToolResult::text(content)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}
