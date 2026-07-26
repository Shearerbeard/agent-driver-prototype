//! Execution persistence for orchestration observability.
//!
//! Run manifest schema, session history loading, and the artifact runtime
//! facade. The artifact runtime lives in the `artifacts` submodule and is
//! re-exported here so existing import paths remain unchanged.

use serde::{Deserialize, Serialize};

// ============================================================================
// Tool trace types
// ============================================================================

/// Condensed tool call entry for the manifest tool trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTraceEntry {
    pub tool: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    pub duration_ms: u64,
    pub outcome: ToolOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_filename: Option<String>,
}

/// Outcome of a single tool call in the trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Success { output_bytes: u64 },
    Error { message: String },
}

// ============================================================================
// Spilled-artifact pointer and inventory ref
// ============================================================================

use crate::context::ContextError;

/// Parse the trailing `[Full result (N chars) saved to artifact: FILE]` footer.
fn parse_trailing_footer(text: &str) -> Option<TrailingFooter> {
    const PREFIX: &str = "[Full result (";
    const INFIX: &str = " chars) saved to artifact: ";
    let start = text.rfind(PREFIX)?;
    let after_prefix = &text[start + PREFIX.len()..];
    let (digits, rest) = after_prefix.split_once(INFIX)?;
    let full_chars: usize = digits.parse().ok()?;
    let filename = rest.trim_end().strip_suffix(']')?;
    let artifact = SpilledArtifact::new(filename, full_chars).ok()?;
    Some(TrailingFooter { start, artifact })
}

/// The single classification point between inline and spilled evidence.
struct TrailingFooter {
    start: usize,
    artifact: SpilledArtifact,
}

/// Pointer to a worker result spilled to an artifact file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpilledArtifact {
    filename: String,
    full_chars: usize,
}

impl SpilledArtifact {
    /// Parse a spilled-result pointer from its artifact filename and the
    /// full result length in characters.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError::EmptyArtifactFilename`] when `filename` is
    /// empty or only whitespace.
    pub fn new(filename: &str, full_chars: usize) -> Result<Self, ContextError> {
        if filename.trim().is_empty() {
            return Err(ContextError::EmptyArtifactFilename);
        }
        Ok(Self {
            filename: filename.to_owned(),
            full_chars,
        })
    }

    /// Parse the trailing spill footer out of worker-reported text.
    pub fn parse_trailing(text: &str) -> Option<Self> {
        parse_trailing_footer(text).map(|footer| footer.artifact)
    }

    /// Parse the trailing spill footer and return the byte offset where it starts.
    ///
    /// The offset is the index of the `[` in the footer string, used by callers
    /// that need to recover the text that appeared before the footer.
    pub fn parse_trailing_with_offset(text: &str) -> Option<(usize, Self)> {
        parse_trailing_footer(text).map(|footer| (footer.start, footer.artifact))
    }

    /// The artifact filename, readable via `read_artifact`.
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Render the pointer together with a stand-in prefix.
    pub fn render_with_prefix(&self, prefix: &str) -> String {
        format!("{prefix}\n\n{self}")
    }
}

impl std::fmt::Display for SpilledArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[Full result ({} chars) saved to artifact: {}]",
            self.full_chars, self.filename
        )
    }
}

/// One artifact inventory line for a completed task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    filename: String,
    bytes: u64,
}

impl ArtifactRef {
    /// Parse an artifact inventory reference.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError::EmptyArtifactFilename`] when `filename` is
    /// empty or only whitespace.
    pub fn new(filename: &str, bytes: u64) -> Result<Self, ContextError> {
        if filename.trim().is_empty() {
            return Err(ContextError::EmptyArtifactFilename);
        }
        Ok(Self {
            filename: filename.to_owned(),
            bytes,
        })
    }

    /// The artifact filename, readable via `read_artifact`.
    pub fn filename(&self) -> &str {
        &self.filename
    }
}

impl std::fmt::Display for ArtifactRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Artifact: {} ({} bytes)]", self.filename, self.bytes)
    }
}
