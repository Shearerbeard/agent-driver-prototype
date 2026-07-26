//! How a finished substrate run is read back as a coordinator result.

use agent_driver_rs::agent::{AgentOutcome, LoopStopReason};

use super::run_store::RunStore;
use super::terminal::{FinalResponse, TerminalSlot};

/// What a coordinator run produced.
///
/// The substrate reports why its loop stopped; that alone does not say what
/// the user gets. This enum is the join of the stop reason with the answer
/// slot, and it separates the cases the caller must treat differently: an
/// answer the coordinator wrote, an answer the host wrote because the budget
/// ran out, a run that stopped with nothing written, and a run that failed.
#[derive(Debug, Clone)]
pub enum CoordinatorOutcome {
    /// The coordinator wrote the answer and the loop ended on its own.
    Responded { action: FinalResponse, turns: u32 },
    /// The turn budget stopped the loop before the coordinator wrote an
    /// answer, so the host wrote one from the run's freshest evidence.
    BudgetExhausted { fallback: FinalResponse, turns: u32 },
    /// The model stopped calling tools without writing an answer. The last
    /// thing it said is all there is.
    StoppedWithoutResponse { last_text: String, turns: u32 },
    /// The run ended on a substrate failure — a tool error the loop refused
    /// to continue past, a provider stream failure, or cancellation.
    Failed { reason: LoopStopReason, turns: u32 },
}

impl CoordinatorOutcome {
    /// Read a finished substrate run as a coordinator result.
    ///
    /// A recorded answer outranks the budget: the coordinator can write the
    /// answer on the same turn that exhausts its depth, and the answer it
    /// wrote is better than the fallback the host would write for it.
    pub fn interpret(
        _outcome: AgentOutcome,
        _answer: &TerminalSlot<FinalResponse>,
        _runs: &RunStore,
    ) -> Self {
        todo!("S71 Phase 2")
    }

    /// How many tool-calling turns the run took.
    pub fn turns(&self) -> u32 {
        todo!("S71 Phase 2")
    }
}

impl FinalResponse {
    /// Write the answer the coordinator did not.
    ///
    /// Rendered from the most recent execution observation, which is the
    /// freshest evidence the run holds — the user gets the work already paid
    /// for. When nothing was executed there are no results to salvage, and
    /// the answer instead states that the run ended before any plan ran and
    /// lists the plans that were created but never executed.
    pub fn host_fallback(_runs: &RunStore) -> Self {
        todo!("S71 Phase 2")
    }
}
