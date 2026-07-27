//! The DAG executor: runs a plan's tasks through real worker inner loops.
//!
//! Replaces [`StubExecutor`](crate::coordinator_loop::StubExecutor) with an
//! executor that selects ready tasks from the plan's DAG, dispatches each to
//! a worker `AgentLoop` behind four tools (`keystrokes`, `capture-pane`,
//! `submit_result`, `read_artifact`), and propagates failure to descendants.
//! The `execute` result is a structured review packet: per-task status,
//! bounded summary, artifact handles, and failure category, with full bodies
//! spilled to addressed artifacts.
//!
//! Phase 1 declares the types; the execution body lands in Phase 2.

mod executor;
mod tools;
mod worker;

pub use executor::DagExecutor;
pub use tools::{CapturePaneArgs, CapturePaneTool, KeystrokesArgs, KeystrokesTool, ReadArtifactArgs, ReadArtifactTool};
pub use worker::{WorkerLoop, WorkerLoopConfig, WorkerOutcome};
