//! The four worker tools: `keystrokes`, `capture-pane`, `read_artifact`.
//!
//! `submit_result` is reused from `coordinator_loop::tools::submit_result`;
//! this module declares the three new tools. The MCP pair (`keystrokes`,
//! `capture-pane`) forward through [`SidecarClient`]; `read_artifact` reads
//! from [`ArtifactStore`]. All three are mounted on worker sessions, not the
//! coordinator's.

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::types::ToolName;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::artifacts::{ArtifactFilename, ArtifactStore, RunId};
use crate::mcp_client::{SidecarClient, SidecarToolArgs, SidecarToolName};

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
// keystrokes
// ============================================================================

/// Arguments for the `keystrokes` tool, matching the sidecar's schema.
///
/// The schema carries `minLength: 1` on `keystrokes`; the tool body also
/// rejects an empty string at the parse boundary (body stays `todo!()` in
/// Phase 2a).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct KeystrokesArgs {
    pub keystrokes: String,
    #[serde(default)]
    pub wait_time_sec: Option<f64>,
    #[serde(default)]
    pub append_enter: Option<bool>,
}

/// Sends keystrokes to the tmux session behind the sidecar.
///
/// Forbidden invalid state: a keystrokes call with no `keystrokes` string;
/// the sidecar's schema marks it required, and the args type carries it as
/// a non-optional field.
pub struct KeystrokesTool {
    definition: ToolDefinition,
    sidecar: SidecarClient,
}

impl KeystrokesTool {
    /// Mount the keystrokes tool over a connected sidecar client.
    pub fn new(sidecar: SidecarClient) -> Self {
        Self {
            definition: worker_tool_definition(
                "keystrokes",
                "Send keystrokes to the tmux session. Use tmux-style escape \
                 sequences for special characters (e.g. C-c for ctrl-c). Set \
                 append_enter to execute a bash command.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "keystrokes": {
                            "type": "string",
                            "description": "Keystrokes to execute in the terminal.",
                            "minLength": 1
                        },
                        "wait_time_sec": {
                            "type": "number",
                            "description": "Seconds to wait for the command to complete.",
                            "default": 0.0
                        },
                        "append_enter": {
                            "type": "boolean",
                            "description": "Append a newline to execute the command.",
                            "default": false
                        }
                    },
                    "required": ["keystrokes"]
                }),
            ),
            sidecar,
        }
    }
}

#[async_trait]
impl Tool for KeystrokesTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: KeystrokesArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "keystrokes arguments did not parse: {error}"
                )));
            }
        };
        if args.keystrokes.trim().is_empty() {
            return Ok(ToolResult::error("keystrokes must not be empty"));
        }
        let name = SidecarToolName::new("keystrokes").expect("non-empty tool name");
        let sidecar_args = SidecarToolArgs::from_map(input.inner().clone());
        match self.sidecar.call_tool(&name, &sidecar_args).await {
            Ok(content) => Ok(ToolResult::text(content.as_str().to_owned())),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}

// ============================================================================
// capture-pane
// ============================================================================

/// Arguments for the `capture-pane` tool, matching the sidecar's schema.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CapturePaneArgs {
    #[serde(default)]
    pub wait_before_capture_sec: Option<f64>,
}

/// Captures the current pane content from the tmux session.
pub struct CapturePaneTool {
    definition: ToolDefinition,
    sidecar: SidecarClient,
}

impl CapturePaneTool {
    /// Mount the capture-pane tool over a connected sidecar client.
    pub fn new(sidecar: SidecarClient) -> Self {
        Self {
            definition: worker_tool_definition(
                "capture-pane",
                "Capture the current pane content of the tmux session. \
                 Useful after sending keystrokes to read the output.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "wait_before_capture_sec": {
                            "type": "number",
                            "description": "Seconds to wait before capturing.",
                            "default": 0.0
                        }
                    }
                }),
            ),
            sidecar,
        }
    }
}

#[async_trait]
impl Tool for CapturePaneTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let _args: CapturePaneArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "capture-pane arguments did not parse: {error}"
                )));
            }
        };
        let name = SidecarToolName::new("capture-pane").expect("non-empty tool name");
        let sidecar_args = SidecarToolArgs::from_map(input.inner().clone());
        match self.sidecar.call_tool(&name, &sidecar_args).await {
            Ok(content) => Ok(ToolResult::text(content.as_str().to_owned())),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
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
