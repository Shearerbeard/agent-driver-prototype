//! Filename-addressed artifact storage with inline-size-bound spill.
//!
//! Ported from aura's `orchestration/persistence/artifacts/{storage,spill}.rs`
//! (923 LOC), keeping only the rig-free subset: filename-addressed writes and
//! reads, cross-run path-traversal guards, and the inline-size bound that
//! spills full bodies to addressed files. The scratchpad pointer path from
//! the aura source is out of scope.
//!
//! Phase 1 declares the types; the filesystem bodies land in Phase 2.

mod spill;
mod storage;

pub use spill::{InlineThreshold, SpilledBody};
pub use storage::{ArtifactError, ArtifactFilename, ArtifactStore};
