//! The native tool surface the coordinator loop drives.
//!
//! Planning, execution and run inspection are ordinary tools that return an
//! observation and leave the loop running. `respond` records the run's
//! answer and also leaves the loop running: the substrate has no
//! terminal-tool concept, so the run ends when the model stops calling tools
//! or the turn budget fires, and the recorded answer is what the outcome is
//! read from.
//!
//! Every rejection a tool can produce is a `ToolResult::error`, which the
//! loop delivers as an observation the coordinator can revise against. None
//! of them is a `ToolError`, which would risk ending the conversation.

mod create_plan;
mod execute;
mod inspect_run;
mod respond;
mod submit_result;

pub use create_plan::{CreatePlanArgs, CreatePlanTool};
pub use execute::{ExecuteArgs, ExecuteTool};
pub use inspect_run::{InspectRunArgs, InspectRunTool, RunSelector};
pub use respond::{RespondArgs, RespondTool};
pub use submit_result::{SubmitResultArgs, SubmitResultTool};

use agent_driver_rs::tool::{ToolDefinition, ToolResult, ToolSchema};
use agent_driver_rs::types::ToolName;
use serde::Serialize;
use serde_json::Value as JsonValue;

/// Build a native tool definition from this module's literals.
///
/// Both conversions are fallible in general and infallible here: the names
/// are non-empty identifier text and the schemas are object literals, so a
/// rejection would mean the literal below it was edited into something that
/// is not a tool definition at all.
fn native_definition(name: &str, description: &str, schema: JsonValue) -> ToolDefinition {
    let Ok(name) = ToolName::new(name) else {
        unreachable!("tool names in this module are non-empty identifiers")
    };
    let Some(schema) = ToolSchema::from_value(schema) else {
        unreachable!("tool schemas in this module are JSON object literals")
    };
    ToolDefinition::new(name, description, schema)
}

/// Carry an observation back to the model as the tool result string.
///
/// A serialization failure would mean an observation type stopped being
/// plain JSON data, which is a defect in this crate rather than something
/// the coordinator can act on, so it is reported as a tool error the loop
/// still survives.
fn observation_result<T: Serialize>(observation: &T) -> ToolResult {
    match serde_json::to_string(observation) {
        Ok(json) => ToolResult::text(json),
        Err(error) => ToolResult::error(format!("observation could not be serialized: {error}")),
    }
}
