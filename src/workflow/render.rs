//! Human digest renderer for a validated workflow proposal.
//!
//! The digest is what an approver sees: the workflow's goal, each step's
//! intent, the tool it calls, and how it can be undone. W2 fills the body;
//! Layer 1 leaves a `todo!()` skeleton so the type surface and seams can
//! compile before the prose quality loop starts.

#![allow(clippy::todo)]

use super::plan::ValidatedWorkflowSpec;

/// Render a validated workflow as a human-readable digest.
///
/// The returned string is the tool observation the coordinator receives;
/// it must name the goal, every step, and any rollback so the approver can
/// authorize exactly what validation vetted.
#[expect(dead_code, reason = "the fill's tool body calls it")]
pub fn render_digest(_validated: &ValidatedWorkflowSpec) -> String {
    todo!("W2: render the validated workflow as a human digest")
}
