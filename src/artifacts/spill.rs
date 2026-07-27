//! The inline-size bound and the spill pointer it produces.

use std::num::NonZeroUsize;

use super::storage::{ArtifactError, ArtifactFilename};

/// The character threshold below which a result stays inline and at or
/// above which it is spilled to an artifact file.
///
/// Forbidden invalid state: a zero threshold, which would spill every
/// result — including an empty one — and produce a spill pointer with no
/// inline body for the coordinator to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineThreshold(NonZeroUsize);

impl InlineThreshold {
    /// The default inline bound matching the aura baseline's
    /// `result_summary_length`.
    pub const DEFAULT: Self = Self(match NonZeroUsize::new(4000) {
        Some(n) => n,
        None => panic!("the default inline threshold must be non-zero"),
    });

    /// Parse an inline-size threshold.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError`] when `chars` is zero.
    pub fn new(chars: usize) -> Result<Self, ArtifactError> {
        NonZeroUsize::new(chars)
            .map(Self)
            .ok_or(ArtifactError::Disabled)
    }

    /// The character count at which spill triggers.
    pub fn get(&self) -> usize {
        self.0.get()
    }

    /// Whether `text` fits inline under this threshold.
    pub fn allows_inline(&self, text: &str) -> bool {
        text.chars().count() < self.get()
    }
}

/// A result body that was spilled to an artifact file.
///
/// The pointer carries the filename and the full body's character count, so
/// the coordinator's observation can show a bounded stand-in plus the
/// pointer without the full body crossing into the conversation.
///
/// Forbidden invalid state: a spill pointer with an empty filename; the
/// constructor delegates to [`ArtifactFilename`] which rejects that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpilledBody {
    filename: ArtifactFilename,
    full_chars: usize,
}

impl SpilledBody {
    /// Parse a spill pointer from a validated filename and the full result
    /// length.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::EmptyFilename`] when `filename` is empty.
    pub fn new(filename: ArtifactFilename, full_chars: usize) -> Self {
        Self {
            filename,
            full_chars,
        }
    }

    /// The artifact filename, readable via `read_artifact`.
    pub fn filename(&self) -> &ArtifactFilename {
        &self.filename
    }

    /// The full result length in characters.
    pub fn full_chars(&self) -> usize {
        self.full_chars
    }
}

impl std::fmt::Display for SpilledBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[Full result ({} chars) saved to artifact: {}]",
            self.full_chars, self.filename
        )
    }
}
