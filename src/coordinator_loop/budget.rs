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
    pub fn new(turns: u32) -> Result<Self, CoordinatorLoopError> {
        NonZeroU32::new(turns)
            .map(Self)
            .ok_or(CoordinatorLoopError::ZeroTurnBudget)
    }

    /// The number of tool-calling turns the budget allows.
    pub fn turns(&self) -> u32 {
        self.0.get()
    }
}

impl std::fmt::Display for LoopBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The budget is enforced by the substrate rather than by a host-side
/// counter: it is the loop's tool-depth ceiling, which the driver reports as
/// a graceful stop instead of an error.
impl From<LoopBudget> for MaxToolDepth {
    fn from(budget: LoopBudget) -> Self {
        match MaxToolDepth::new(budget.turns()) {
            Ok(depth) => depth,
            Err(_) => unreachable!("a loop budget is non-zero by construction"),
        }
    }
}
