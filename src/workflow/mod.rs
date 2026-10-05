//! The workflow-proposal mechanism: W1 plan types, W2 `propose_workflow`
//! tool, and W3 deterministic executor with its `$from` resolver.
//!
//! The design record for the plan types lives in `DESIGN.md` beside this
//! file: what each public type forbids, which seams the next card
//! replaces, and the narrowings against a full JSON-Schema surface the
//! card's scope implies.

mod error;
mod executor;
mod plan;
mod render;
mod resolve;
mod schema;
mod tool;

pub use error::WorkflowError;
pub use executor::{
    ExecuteError, RollbackOutcome, RunOutcome, RunRecord, StepRecord, StepStatus, execute_workflow,
};
pub use plan::{
    ArgValue, Bounds, ExportName, ExportRef, ExportSpec, RollbackSpec, StepId,
    ValidatedWorkflowSpec, WorkflowSpec, WorkflowStep,
};
pub use resolve::{
    ExportEnvironment, ResolveError, capture_exports, parse_tool_result, resolve_arguments,
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
