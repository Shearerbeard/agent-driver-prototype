//! The handle that names one created plan for the rest of the run.

use serde::{Deserialize, Serialize};

use super::error::CoordinatorLoopError;
use super::tools::CreatePlanArgs;

/// A plan's identity, derived from the arguments that created it.
///
/// The id is a pure function of the normalized plan arguments, so the same
/// plan proposed twice collapses to one entry instead of accumulating
/// near-duplicate revisions, and a test can precompute the id it expects to
/// see in an observation. It is derived and never authored: parsing rejects
/// anything outside the derived alphabet, so an id the model invents cannot
/// masquerade as a plan the run created.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlanId(String);

impl PlanId {
    /// Character width of the derived digest.
    pub const HEX_LEN: usize = 16;

    /// Derive the id of the plan these arguments describe.
    ///
    /// Normalization is the point: two argument sets that describe the same
    /// plan derive the same id regardless of incidental whitespace.
    pub fn derive(_args: &CreatePlanArgs) -> Self {
        todo!("S71 Phase 2")
    }

    /// Parse an id the model echoed back.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::MalformedPlanId`] when the value is
    /// not [`HEX_LEN`](Self::HEX_LEN) lowercase hex characters.
    pub fn parse(_raw: &str) -> Result<Self, CoordinatorLoopError> {
        todo!("S71 Phase 2")
    }

    /// The id as it appears in observations and tool arguments.
    pub fn as_str(&self) -> &str {
        todo!("S71 Phase 2")
    }
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("S71 Phase 2")
    }
}

impl TryFrom<String> for PlanId {
    type Error = CoordinatorLoopError;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::parse(&raw)
    }
}

impl From<PlanId> for String {
    fn from(id: PlanId) -> Self {
        id.0
    }
}
