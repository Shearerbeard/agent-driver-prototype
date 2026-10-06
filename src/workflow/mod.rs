//! The workflow-proposal mechanism: W1 plan types, W2 `propose_workflow`
//! tool, W3 deterministic executor with its `$from` resolver, and W4 sync
//! approval wire.
//!
//! The design record for the plan types lives in `DESIGN.md` beside this
//! file: what each public type forbids, which seams the next card
//! replaces, and the narrowings against a full JSON-Schema surface the
//! card's scope implies.
//!
//! W4 note: the approval wire is gated on the `[workflow]` config section.
//! When `approval_url` is present the tool blocks on a human decision;
//! when it is absent the tool stays propose-only.  The `decision_id` vs
//! digest seam lives in `approval::DecisionId` and is adjudicated at the
//! `U(wire-contract)` gate, not by the executor.

mod approval;
mod error;
mod executor;
mod plan;
mod render;
mod resolve;
mod schema;
mod tool;

pub use approval::{
    ApprovalClient, ApprovalError, ApprovalHold, ApprovalOutcome, ApprovalPayload, Approved,
    DecisionId, POLL_INTERVAL_SECONDS, REQUEST_TIMEOUT_SECS, apply_authorized,
};
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

/// The single decision point for mounting the coordinator's workflow tool.
///
/// `[workflow] enabled` means the tool is constructed and registered, and
/// every surface that claims it (the preamble's tool list, the loop's
/// registration) derives from this same call, so claims and registration
/// cannot disagree (S114).
///
/// When `[workflow].approval_url` is present the tool is wired for the
/// blocking approval hold; otherwise it stays in W2 propose-only mode.
/// `session_id` is the proposing request's shim session id, threaded here
/// so the notify payload correlates with the conversation that proposed
/// (the mount is per-request, which is where the id lives).
pub fn workflow_tool_for(
    section: &crate::shim_config::WorkflowSection,
    sidecar: &crate::mcp_client::SidecarClient,
    session_id: &str,
) -> Option<std::sync::Arc<ProposeWorkflowTool>> {
    if !section.enabled {
        return None;
    }
    let mut tool = ProposeWorkflowTool::new(sidecar.clone()).with_session_id(session_id.to_owned());
    if let Some(url) = &section.approval_url {
        let hold_secs = section
            .hold_secs
            .expect("[workflow].hold_secs is required and validated when the section is enabled");
        let client = ApprovalClient::for_section(url, hold_secs, DecisionId::Digest)
            .expect("validated approval_url must parse into an approval client");
        tool = tool.with_approval(client);
    }
    Some(std::sync::Arc::new(tool))
}
