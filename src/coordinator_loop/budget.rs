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
    /// The provisional TerminalBench default, twelve turns.
    ///
    /// Derived from the depth the program has been testing rather than from
    /// the substrate's own default of twenty-five. The canonical benchmark
    /// config caps the bounded router at `max_planning_cycles = 4`
    /// (`configs/sre-shell-orchestrated.toml:123` in the adapter repo). One
    /// of those cycles maps to a `create_plan` and `execute` pair of loop
    /// turns, so four cycles cost `4 * 2` turns. Writing the answer costs one
    /// more, and three turns of `inspect_run` slack cover the pull-on-demand
    /// reads the bounded router never had: `4 * 2 + 1 + 3 = 12`.
    ///
    /// This is the value for now. S74 and S75 consume it, and any caller
    /// that wants a different depth builds one with [`new`](Self::new).
    pub const CANONICAL: Self = Self(match NonZeroU32::new(12) {
        Some(turns) => turns,
        None => panic!("the canonical turn budget must be non-zero"),
    });

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_budget_matches_the_benchmark_derivation() {
        assert_eq!(LoopBudget::CANONICAL.turns(), 12);
    }
}
