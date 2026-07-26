//! The coordinator as one continuous tool loop.
//!
//! Planning, execution and run inspection are ordinary tools that return an
//! observation and leave the conversation running; the run ends when the
//! model stops calling tools or when its turn budget fires. There is no
//! host-owned outer loop, no control enum the model must emit to continue,
//! and no state replayed into a prompt — the conversation history is the
//! state.
//!
//! The design this narrows is recorded in `DESIGN.md` beside this file: what
//! each public type forbids, which seams the next card replaces, and which
//! parts of the reference design are deliberately absent.
//!
//! # Implementation status
//!
//! S71 landed this module as a type skeleton. The bodies are implemented in
//! the phase that follows the design review, so every function here is
//! `todo!()` and the fields they read are not read yet.

#![expect(
    dead_code,
    reason = "type skeleton: the bodies that read these fields land in S71 Phase 2"
)]

mod budget;
mod driver;
mod error;
mod executor;
mod observation;
mod outcome;
mod plan_id;
mod run_store;
mod terminal;
mod tools;

pub use budget::LoopBudget;
pub use driver::{CoordinatorLoop, CoordinatorLoopConfig, WorkerSections};
pub use error::{AlreadyRecorded, CoordinatorLoopError, CoordinatorRunError};
pub use executor::{PlanExecutor, StubExecutor};
pub use observation::{
    ExecutionObservation, OutcomeCounts, PlanObservation, TaskObservation,
};
pub use outcome::CoordinatorOutcome;
pub use plan_id::PlanId;
pub use run_store::RunStore;
pub use terminal::{FinalResponse, TerminalSlot, WorkerSubmission};
pub use tools::{
    CreatePlanArgs, CreatePlanTool, ExecuteArgs, ExecuteTool, InspectRunArgs, InspectRunTool,
    RespondArgs, RespondTool, RunSelector, SubmitResultArgs, SubmitResultTool,
};
