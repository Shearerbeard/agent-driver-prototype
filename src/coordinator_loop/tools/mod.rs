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

use super::driver::WorkerSections;

/// Metadata for one coordinator tool: its registered name, its definition
/// builder, and the two prompt summaries derived from it.
///
/// Keeping the summaries next to the definition builder means each prose
/// variant is stated once: the JSON description is the literal passed to
/// [`ToolDefinition::new`], and the prompt summaries are the exact prose the
/// pre-W2 hardcoded prompts used.
pub(crate) struct CoordinatorToolSpec {
    pub name: &'static str,
    pub build_definition: fn(&WorkerSections) -> ToolDefinition,
    pub preamble_summary: &'static str,
    pub planning_loop_summary: &'static str,
}

const COORDINATOR_TOOL_SPECS: &[CoordinatorToolSpec] = &[
    CoordinatorToolSpec {
        name: "create_plan",
        build_definition: create_plan::definition,
        preamble_summary: create_plan::PREAMBLE_SUMMARY,
        planning_loop_summary: create_plan::PLANNING_LOOP_SUMMARY,
    },
    CoordinatorToolSpec {
        name: "execute",
        build_definition: execute::definition,
        preamble_summary: execute::PREAMBLE_SUMMARY,
        planning_loop_summary: execute::PLANNING_LOOP_SUMMARY,
    },
    CoordinatorToolSpec {
        name: "inspect_run",
        build_definition: inspect_run::definition,
        preamble_summary: inspect_run::PREAMBLE_SUMMARY,
        planning_loop_summary: inspect_run::PLANNING_LOOP_SUMMARY,
    },
    CoordinatorToolSpec {
        name: "respond",
        build_definition: respond::definition,
        preamble_summary: respond::PREAMBLE_SUMMARY,
        planning_loop_summary: respond::PLANNING_LOOP_SUMMARY,
    },
];

/// The coordinator's registered tool definitions, in registration order.
///
/// This is the single source for the coordinator's tool surface: the live
/// driver's tools and any coordinator prompt that names tools derive from
/// these builders, so a prompt cannot claim a tool the loop never registered.
///
/// The unmounted form returns the four core coordinator tools. Use
/// [`coordinator_tool_definitions_with_workflow`] to include the optional
/// `propose_workflow` definition when the workflow seam is mounted.
pub fn coordinator_tool_definitions(sections: &WorkerSections) -> Vec<ToolDefinition> {
    coordinator_tool_definitions_with_workflow(sections, None)
}

/// Parameterized factory: the extra `propose_workflow` definition flows
/// through here, so the preamble's tools section, the planning template's
/// numbered list, and the attached definitions all name the same surface.
///
/// Nothing appends the extra tool outside this factory (S114 invariant).
pub fn coordinator_tool_definitions_with_workflow(
    sections: &WorkerSections,
    workflow: Option<&ToolDefinition>,
) -> Vec<ToolDefinition> {
    let mut definitions: Vec<_> = COORDINATOR_TOOL_SPECS
        .iter()
        .map(|spec| (spec.build_definition)(sections))
        .collect();
    if let Some(definition) = workflow {
        definitions.push(definition.clone());
    }
    definitions
}

/// The registered names of the base four, in registration order - the
/// unmounted claims source for callers without a live registration list.
pub fn coordinator_tool_names() -> Vec<&'static str> {
    COORDINATOR_TOOL_SPECS
        .iter()
        .map(|spec| spec.name)
        .collect()
}

/// Preamble-style (short) summary pairs for the coordinator preamble tools
/// section.
///
/// The registered names are the source: callers pass the names of the tools
/// a run actually registers (the driver derives them from its registered
/// instances), and the summary registry is keyed by those names. A name
/// with no summary fails loud rather than silently omitting a registered
/// tool from the claims - the exact divergence S114 exists to prevent.
pub fn coordinator_tool_preamble_pairs<'a>(registered: &[&'a str]) -> Vec<(&'a str, &'static str)> {
    registered
        .iter()
        .map(|name| (*name, preamble_summary_for(name)))
        .collect()
}

/// Planning-loop-style (longer) summary pairs for the loop planning wrapper's
/// tools section, derived from the registered names exactly as the preamble
/// pairs are.
pub fn coordinator_tool_planning_loop_pairs<'a>(
    registered: &[&'a str],
) -> Vec<(&'a str, &'static str)> {
    registered
        .iter()
        .map(|name| (*name, planning_loop_summary_for(name)))
        .collect()
}

fn preamble_summary_for(name: &str) -> &'static str {
    if name == "propose_workflow" {
        return crate::workflow::ProposeWorkflowTool::PREAMBLE_SUMMARY;
    }
    COORDINATOR_TOOL_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.preamble_summary)
        .unwrap_or_else(|| panic!("no preamble summary registered for coordinator tool {name}"))
}

fn planning_loop_summary_for(name: &str) -> &'static str {
    if name == "propose_workflow" {
        return crate::workflow::ProposeWorkflowTool::PLANNING_LOOP_SUMMARY;
    }
    COORDINATOR_TOOL_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.planning_loop_summary)
        .unwrap_or_else(|| {
            panic!("no planning-loop summary registered for coordinator tool {name}")
        })
}

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
