//! How a finished substrate run is read back as a coordinator result.

use agent_driver_rs::agent::{AgentOutcome, LoopStopReason};

use super::run_store::RunStore;
use super::terminal::{FinalResponse, TerminalSlot};

/// Why the provider stopped generating before the coordinator was finished.
///
/// Host-owned rather than the substrate's stop reason, because the loop must
/// stay total over a foreign enum that may grow variants. The named cases
/// are the truncation reasons a provider actually reports mid-run; anything
/// else is carried verbatim rather than folded into a case that would
/// misdescribe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptionReason {
    /// The model reached its output token ceiling.
    TokenLimit,
    /// A configured stop sequence cut the turn short.
    StopSequence,
    /// The provider's content filter blocked further output.
    ContentFilter,
    /// A stop reason this build does not model, carried as the substrate
    /// reported it.
    Unclassified(String),
}

impl std::fmt::Display for InterruptionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenLimit => f.write_str("output token limit reached"),
            Self::StopSequence => f.write_str("stop sequence reached"),
            Self::ContentFilter => f.write_str("blocked by the provider content filter"),
            Self::Unclassified(reason) => f.write_str(reason),
        }
    }
}

/// What a coordinator run produced.
///
/// The substrate reports why its loop stopped; that alone does not say what
/// the user gets. This enum is the join of the stop reason with the answer
/// slot, and it separates the cases the caller must treat differently: an
/// answer the coordinator wrote, an answer the host wrote because the turn
/// budget ran out, a run that ended cleanly with nothing written, and a run
/// the provider cut short.
///
/// Substrate failures are absent by design. A provider stream failure and a
/// mid-stream cancellation come back as
/// [`CoordinatorRunError`](super::CoordinatorRunError), never as an outcome,
/// so no variant here describes a state the pin cannot deliver.
#[derive(Debug, Clone)]
pub enum CoordinatorOutcome {
    /// The coordinator wrote the answer.
    Responded { action: FinalResponse, turns: u32 },
    /// The turn budget stopped the loop before the coordinator wrote an
    /// answer, so the host wrote one from the run's freshest evidence.
    BudgetExhausted { fallback: FinalResponse, turns: u32 },
    /// The model ended its turn without writing an answer. The last thing it
    /// said is all there is.
    StoppedWithoutResponse { last_text: String, turns: u32 },
    /// The provider stopped generating before the coordinator was finished.
    Interrupted {
        reason: InterruptionReason,
        last_text: String,
        turns: u32,
    },
}

impl CoordinatorOutcome {
    /// Read a finished substrate run as a coordinator result.
    ///
    /// The join is total over both inputs. A filled slot absorbs every stop
    /// reason and yields [`Responded`](Self::Responded): the coordinator
    /// committed to an answer on a permitted round, and that answer stands
    /// whether the loop then ended on its own, ran out of turns, or was cut
    /// short. With an empty slot the stop reason decides:
    ///
    /// | Stop reason | Empty slot |
    /// |---|---|
    /// | `EndTurn` | `StoppedWithoutResponse` |
    /// | `MaxToolDepthReached` | `BudgetExhausted` with the host fallback |
    /// | `MaxTokens` | `Interrupted(TokenLimit)` |
    /// | `StopSequence` | `Interrupted(StopSequence)` |
    /// | `ContentFilter` | `Interrupted(ContentFilter)` |
    /// | anything else | `Interrupted(Unclassified)` carrying its text |
    ///
    /// The budget cannot be the round that writes the answer: the substrate
    /// refuses the exhausting response's tool calls without executing them,
    /// so slot-wins means an answer from an earlier round outranks the depth
    /// stop, never an answer written on the stop itself.
    pub fn interpret(
        outcome: AgentOutcome,
        answer: &TerminalSlot<FinalResponse>,
        runs: &RunStore,
    ) -> Self {
        let turns = outcome.iterations;
        if let Some(action) = answer.recorded() {
            return Self::Responded { action, turns };
        }

        let last_text = outcome.final_response.text();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "LoopStopReason is #[non_exhaustive]; the wildcard is the total case"
        )]
        match outcome.stop_reason {
            LoopStopReason::EndTurn => Self::StoppedWithoutResponse { last_text, turns },
            LoopStopReason::MaxToolDepthReached => Self::BudgetExhausted {
                fallback: FinalResponse::host_fallback(runs),
                turns,
            },
            LoopStopReason::MaxTokens => Self::Interrupted {
                reason: InterruptionReason::TokenLimit,
                last_text,
                turns,
            },
            LoopStopReason::StopSequence => Self::Interrupted {
                reason: InterruptionReason::StopSequence,
                last_text,
                turns,
            },
            LoopStopReason::ContentFilter => Self::Interrupted {
                reason: InterruptionReason::ContentFilter,
                last_text,
                turns,
            },
            other => Self::Interrupted {
                reason: InterruptionReason::Unclassified(other.to_string()),
                last_text,
                turns,
            },
        }
    }

    /// How many tool-calling turns the run took.
    pub fn turns(&self) -> u32 {
        match self {
            Self::Responded { turns, .. }
            | Self::BudgetExhausted { turns, .. }
            | Self::StoppedWithoutResponse { turns, .. }
            | Self::Interrupted { turns, .. } => *turns,
        }
    }
}
