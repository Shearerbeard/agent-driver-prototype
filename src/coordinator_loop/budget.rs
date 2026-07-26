//! The turn-depth budget that breaks the coordinator loop.

use std::num::NonZeroU32;

use agent_driver_rs::agent::MaxToolDepth;

use super::error::CoordinatorLoopError;

/// How many tool-calling turns the coordinator may take before the substrate
/// stops the loop.
///
/// This is the loop's only host-side breaker. It replaces the bounded
/// router's planning-cycle cap: under a continuous loop there is no cycle to
/// count, and F1 observed that turn depth and the agent timeout are the only
/// breakers a worker ever hits in practice. The coordinator gets the same
/// shape rather than a second, richer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopBudget(NonZeroU32);

impl LoopBudget {
    /// Parse a turn-depth budget.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::ZeroTurnBudget`] when `turns` is zero.
    pub fn new(_turns: u32) -> Result<Self, CoordinatorLoopError> {
        todo!("S71 Phase 2")
    }

    /// The number of tool-calling turns the budget allows.
    pub fn turns(&self) -> u32 {
        todo!("S71 Phase 2")
    }
}

impl std::fmt::Display for LoopBudget {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("S71 Phase 2")
    }
}

/// The budget is enforced by the substrate rather than by a host-side
/// counter: it is the loop's tool-depth ceiling, which the driver reports as
/// a graceful stop instead of an error. The conversion cannot fail, because
/// the budget is non-zero by construction.
impl From<LoopBudget> for MaxToolDepth {
    fn from(_budget: LoopBudget) -> Self {
        todo!("S71 Phase 2")
    }
}
