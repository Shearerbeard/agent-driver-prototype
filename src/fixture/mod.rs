//! The S2 golden-frame test harness: fixture scenario types, envelope
//! builders, and snapshot normalization — ported from
//! `crates/aura/src/orchestration/context_fixture/` onto this crate's
//! local mirror types.

mod envelope;
mod helpers;
mod normalize;
mod scenario;
mod tool_definitions;

pub(crate) use envelope::{coordinator_envelope, worker_envelope};
#[cfg(test)]
pub(crate) use normalize::assert_envelope_snapshot;
pub(crate) use normalize::{NormalizedSnapshot, normalize};
pub(crate) use scenario::{
    CompletedResultFixture, ContinuationThread, CoordinatorCall, CoordinatorScenario,
    CoordinatorToolConfig, FailedResultFixture, FixtureError, FrameGraph, HistoryTools,
    IterationFixture, PlanDecision, PlanningBudget, PreambleFixture, ReconTools, ScratchpadWiring,
    SessionHistoryFixture, SpilledStandIn, TaskOutcome, WorkerFrameFixture, WorkerPreambleAppends,
    WorkerPreambleFixture, WorkerRosterFixture, WorkerScenario,
};
