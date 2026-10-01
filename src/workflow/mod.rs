//! The workflow-proposal mechanism's plan types (W1): the shape of a
//! proposed pre-authorized DAG of MCP tool calls, the rules a valid one
//! satisfies before a human ever sees it, and the `$.`-path subset its
//! exports name. Types only — nothing mounts, nothing applies; the
//! `propose_workflow` tool that consumes these types is W2, the
//! deterministic executor W3.
//!
//! The design record lives in `DESIGN.md` beside this file: what each
//! public type forbids, which seams the next card replaces, and the
//! narrowings against a full JSON-Schema surface the card's scope
//! implies.

mod error;
mod plan;
mod render;
mod schema;
mod tool;

pub use error::WorkflowError;
pub use plan::{
    ArgValue, Bounds, ExportName, ExportRef, ExportSpec, RollbackSpec, StepId,
    ValidatedWorkflowSpec, WorkflowSpec, WorkflowStep,
};
pub use tool::ProposeWorkflowTool;

/// The single decision point for mounting the coordinator's workflow tool:
/// `[workflow]` enabled means the tool is constructed and registered, and
/// every surface that claims it (the preamble's tool list, the loop's
/// registration) derives from this same call, so claims and registration
/// cannot disagree (S114).
pub fn workflow_tool_for(
    section: &crate::shim_config::WorkflowSection,
    sidecar: &crate::mcp_client::SidecarClient,
) -> Option<std::sync::Arc<ProposeWorkflowTool>> {
    section
        .enabled
        .then(|| std::sync::Arc::new(ProposeWorkflowTool::new(sidecar.clone())))
}
