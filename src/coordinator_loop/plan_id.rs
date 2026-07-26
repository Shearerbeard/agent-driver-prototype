//! The handle that names one created plan for the rest of the run.

use serde::{Deserialize, Serialize};

use crate::types::StepInput;

use super::error::CoordinatorLoopError;
use super::tools::CreatePlanArgs;

/// FNV-1a parameters. A named, stable algorithm is used in place of
/// `DefaultHasher`, whose output is documented as unstable across releases
/// and would change every derived id when the toolchain moves.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A plan's identity, derived from what the plan will do.
///
/// The digest covers the goal and the step tree, and nothing else: the
/// planning rationale is commentary the model may rephrase freely, so
/// digesting it would give two ids to one plan. The same plan proposed twice
/// collapses to one entry, and a test can precompute the id it expects to
/// see in an observation.
///
/// The id is derived and never authored: parsing rejects anything outside
/// the derived alphabet, so an id the model invents cannot masquerade as a
/// plan the run created.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlanId(String);

/// The digest input, in the field order the derivation depends on.
#[derive(Serialize)]
struct PlanDigestInput<'a> {
    goal: &'a str,
    steps: &'a [StepInput],
}

impl PlanId {
    /// Character width of the derived digest.
    pub const HEX_LEN: usize = 16;

    /// Derive the id of the plan these arguments describe.
    pub fn derive(args: &CreatePlanArgs) -> Self {
        let input = PlanDigestInput {
            goal: args.goal.trim(),
            steps: &args.steps,
        };
        // Serialization of a struct of owned scalars and the ported step
        // tree cannot fail; an unserializable plan could not have parsed.
        let Ok(canonical) = serde_json::to_vec(&input) else {
            unreachable!("plan digest input is plain JSON data")
        };
        Self::from_digest(fnv1a(&canonical))
    }

    /// The one place a digest becomes an id, so every derived value is
    /// zero-padded to the width `parse` demands.
    fn from_digest(digest: u64) -> Self {
        Self(format!("{digest:016x}"))
    }

    /// Parse an id the model echoed back.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::MalformedPlanId`] when the value is
    /// not [`HEX_LEN`](Self::HEX_LEN) lowercase hex characters.
    pub fn parse(raw: &str) -> Result<Self, CoordinatorLoopError> {
        let well_formed = raw.len() == Self::HEX_LEN
            && raw
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if well_formed {
            Ok(Self(raw.to_owned()))
        } else {
            Err(CoordinatorLoopError::MalformedPlanId)
        }
    }

    /// The id as it appears in observations and tool arguments.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Digest bytes with FNV-1a.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
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
