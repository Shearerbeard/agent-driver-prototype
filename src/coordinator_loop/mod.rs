//! The coordinator as one continuous tool loop.
//!
//! Planning, execution and run inspection are ordinary tools that return an
//! observation and leave the conversation running; the run ends when the
//! model stops calling tools or when its turn budget fires. There is no
//! host-owned outer loop, no control enum the model must emit to continue,
//! and no state replayed into a prompt: the conversation history is the
//! state.
//!
//! The design this narrows is recorded in `DESIGN.md` beside this file: what
//! each public type forbids, which seams the next card replaces, and which
//! parts of the reference design are deliberately absent.

mod budget;
mod driver;
mod error;
mod executor;
mod observation;
mod outcome;
mod plan_id;
mod roster;
mod run_store;
mod terminal;
mod tools;

pub use budget::LoopBudget;
pub use driver::{CoordinatorLoop, CoordinatorLoopConfig, WorkerSections};
pub use error::{AlreadyRecorded, CoordinatorLoopError, CoordinatorRunError};
pub use executor::{PlanExecutor, StubExecutor};
pub use observation::{
    ExecutionObservation, OutcomeCounts, PlanObservation, TaskObservation, TaskObservations,
};
pub use outcome::{CoordinatorOutcome, InterruptionReason};
pub use plan_id::PlanId;
pub use roster::{WorkerRoster, WorkerSpec, WorkerTool};
pub use run_store::{Attempt, AttemptZero, RunStore, TaskRecord};
pub use terminal::{FinalResponse, TerminalSlot, WorkerSubmission};
pub use tools::{
    CreatePlanArgs, CreatePlanTool, ExecuteArgs, ExecuteTool, InspectRunArgs, InspectRunTool,
    RespondArgs, RespondTool, RunSelector, SubmitResultArgs, SubmitResultTool,
};
