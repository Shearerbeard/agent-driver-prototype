//! Unified bounding module.
//!
//! One source of truth for every byte/character truncate, summarize, spill,
//! display-limit, and history-limit decision in the orchestrator.  The module
//! exposes strongly-typed limits.  This is a pure-consolidation refactor: it
//! models the semantics that production already accepts today, it does not
//! tighten them.
//!
//! This is the S3 bounding module: function bodies are implemented.  The
//! call-site wiring to production code happens in the implementation phase.

use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

// ============================================================================
// Private implementation-detail widths
// ============================================================================

/// A non-zero character width.
///
/// Private implementation detail.  Domain-specific public types below wrap
/// this so that char-bounded widths are not interchangeable with byte widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct CharWidth(NonZeroUsize);

impl CharWidth {
    // Unused S3 API surface.
    #[allow(dead_code)]
    fn new(chars: usize) -> Option<Self> {
        NonZeroUsize::new(chars).map(Self)
    }

    fn get(&self) -> usize {
        self.0.get()
    }
}

// ============================================================================
// Core config
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncateMarker {
    None,
    EllipsisChar,
    Dots,
}

fn truncate_chars(text: &str, max: usize, marker: TruncateMarker) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((cut, _)) => {
            let truncated = &text[..cut];
            match marker {
                TruncateMarker::None => truncated.to_string(),
                TruncateMarker::EllipsisChar => format!("{truncated}…"),
                TruncateMarker::Dots => format!("{truncated}..."),
            }
        }
    }
}

// ============================================================================
// Character caps
// ============================================================================

/// Character cap for failure-history task handles.
///
/// Business rule: a failed task's identity handle is its first line capped
/// at a fixed width, plus a truncation marker when cut.
///
/// Forbidden invalid state: a zero cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureHandleWidth(CharWidth);

impl FailureHandleWidth {
    /// Default cap matching the accepted baseline binary.
    pub const DEFAULT: Self = Self(match NonZeroUsize::new(120) {
        Some(n) => CharWidth(n),
        None => panic!("fixed failure-handle cap must be non-zero"),
    });

    /// Truncate to the cap, returning the text without marker and whether
    /// anything was cut.  The caller owns the marker decision.
    pub fn truncate_with_flag(&self, text: &str) -> (String, bool) {
        match text.char_indices().nth(self.0.get()) {
            None => (text.to_string(), false),
            Some((cut, _)) => (text[..cut].to_string(), true),
        }
    }

    // Unused S3 API surface.
    #[allow(dead_code)]
    fn truncate(&self, text: &str) -> String {
        truncate_chars(text, self.0.get(), TruncateMarker::None)
    }
}

/// Character cap for failed-task error previews.
///
/// Business rule: failure entries show a bounded error preview with an
/// explicit `[truncated]` marker, never an unbounded error body.
///
/// Forbidden invalid state: a zero cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPreviewWidth(CharWidth);

impl ErrorPreviewWidth {
    pub const DEFAULT: Self = Self(match NonZeroUsize::new(2000) {
        Some(n) => CharWidth(n),
        None => panic!("fixed error-preview cap must be non-zero"),
    });

    /// Truncate to the cap, returning the text without marker and whether
    /// anything was cut.  The caller owns the marker decision.
    pub fn truncate_with_flag(&self, text: &str) -> (String, bool) {
        match text.char_indices().nth(self.0.get()) {
            None => (text.to_string(), false),
            Some((cut, _)) => (text[..cut].to_string(), true),
        }
    }

    pub fn truncate(&self, text: &str) -> String {
        truncate_chars(text, self.0.get(), TruncateMarker::None)
    }
}

/// Character cap for tool-reasoning previews in continuation prompts.
///
/// Business rule: the `_aura_reasoning` text string forwarded into
/// continuation prompts is truncated to a bounded character width with an
/// ellipsis marker.  This cap applies to the reasoning text, not to a
/// reasoning-token budget.
///
/// Forbidden invalid state: a zero cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolReasoningWidth(CharWidth);

impl ToolReasoningWidth {
    pub const DEFAULT: Self = Self(match NonZeroUsize::new(100) {
        Some(n) => CharWidth(n),
        None => panic!("fixed tool-reasoning cap must be non-zero"),
    });

    pub fn truncate(&self, text: &str) -> String {
        truncate_chars(text, self.0.get(), TruncateMarker::EllipsisChar)
    }
}

// ============================================================================
// Tool List Limit
// ============================================================================

/// The coordinator planning prompt truncates long per-worker tool lists to a
/// bounded count before appending `(+N more)`; `max_tools_per_worker = 0` is
/// `HideAll`: Summary rendering shows `(+N more)` for a nonempty list, while
/// Full rendering omits the tool section.
///
/// Business rule: the coordinator planning prompt truncates long per-worker
/// tool lists to a bounded count.  `max_tools_per_worker = 0` is the raw
/// display limit; the renderer decides whether that renders the degenerate
/// `(+N more)` list or omits the tool section entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolListLimit {
    /// Render no tools; the display limit is zero.
    ///
    /// Summary rendering shows `(+N more)` for a nonempty list, while Full
    /// rendering omits the tool section when the limit is zero.
    HideAll,
    /// Render at most this many tools before the suffix.
    Limited(NonZeroUsize),
}

impl ToolListLimit {
    pub fn new(count: usize) -> Self {
        match NonZeroUsize::new(count) {
            Some(n) => Self::Limited(n),
            None => Self::HideAll,
        }
    }

    pub fn get(&self) -> usize {
        match self {
            Self::HideAll => 0,
            Self::Limited(n) => n.get(),
        }
    }
}
