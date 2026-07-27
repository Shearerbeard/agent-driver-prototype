//! Filename-addressed artifact storage with cross-run guards.
//!
//! Every artifact is a single file under the run's `artifacts/` directory,
//! addressed by a safe filename. Cross-run reads resolve through the session
//! directory and are guarded against path traversal by
//! [`ArtifactFilename`]'s constructor and [`RunId`]'s constructor.

use std::path::PathBuf;
use std::sync::Arc;

/// Validate that `raw` is a single safe path component.
///
/// Rejects empty/whitespace, path separators (`/`, `\\`), the exact
/// components `.` and `..`, and control characters. Used by both
/// [`ArtifactFilename`] and [`RunId`] so they share one rule family.
fn validate_path_component(raw: &str) -> Result<String, ArtifactError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ArtifactError::EmptyFilename);
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(ArtifactError::UnsafeFilename);
    }
    if trimmed == "." || trimmed == ".." {
        return Err(ArtifactError::UnsafeFilename);
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(ArtifactError::UnsafeFilename);
    }
    Ok(trimmed.to_owned())
}

/// A single path component safe to use as an artifact filename.
///
/// Forbidden invalid state: a filename containing `/`, `\\`, `.`/`..`, or
/// control characters reaching the filesystem. The constructor rejects
/// those, so downstream code can join the filename onto any base directory
/// without re-checking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactFilename(String);

impl ArtifactFilename {
    /// Parse a filename into a safe path component.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::EmptyFilename`] when `filename` is empty or
    /// whitespace-only, and [`ArtifactError::UnsafeFilename`] when it
    /// contains `/`, `\\`, `.`/`..`, or control characters.
    pub fn new(filename: &str) -> Result<Self, ArtifactError> {
        validate_path_component(filename).map(Self)
    }

    /// The filename as a path component.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ArtifactFilename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A run identifier that is a single safe path component.
///
/// Same rule family as [`ArtifactFilename`]: the constructor rejects empty,
/// whitespace, path separators, `.`/`..`, and control characters, so a
/// `RunId` can be joined onto a session directory without re-checking.
///
/// Forbidden invalid state: a run id that is not a safe path component
/// reaching the filesystem via a cross-run read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(String);

impl RunId {
    /// Parse a run id into a safe path component.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::EmptyFilename`] when `run_id` is empty or
    /// whitespace-only, and [`ArtifactError::UnsafeFilename`] when it
    /// contains `/`, `\\`, `.`/`..`, or control characters.
    pub fn new(run_id: &str) -> Result<Self, ArtifactError> {
        validate_path_component(run_id).map(Self)
    }

    /// The run id as a path component.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why an artifact operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    /// The filename was empty or whitespace-only.
    #[error("artifact filename is empty")]
    EmptyFilename,
    /// The filename contained path separators, `.`/`..`, or control
    /// characters.
    #[error("artifact filename is not a safe path component")]
    UnsafeFilename,
    /// The store is disabled and cannot serve the request.
    #[error("artifact store is disabled")]
    Disabled,
    /// A filesystem operation failed.
    #[error("artifact I/O error: {0}")]
    Io(String),
}

/// Filename-addressed artifact storage with cross-run guards.
///
/// Cloning shares the base path, so worker tools that need `read_artifact`
/// each take a cheap clone. The skeleton holds the base path; the
/// filesystem I/O bodies land in Phase 2.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    #[allow(dead_code)]
    base_path: PathBuf,
    _shared: Arc<()>,
}

impl ArtifactStore {
    /// Create a store rooted at `base_path`.
    ///
    /// The directory is not created here; Phase 2's write methods will
    /// `create_dir_all` on first use.
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            base_path,
            _shared: Arc::new(()),
        }
    }

    /// A disabled store: writes are no-ops, reads return `Disabled`.
    pub fn disabled() -> Self {
        Self {
            base_path: PathBuf::new(),
            _shared: Arc::new(()),
        }
    }

    /// Write `content` to an artifact file and return its filename.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`] when the store is disabled, and
    /// [`ArtifactError::Io`] when the write fails.
    pub async fn write_artifact(
        &self,
        filename: &ArtifactFilename,
        content: &str,
    ) -> Result<ArtifactFilename, ArtifactError> {
        let _ = (filename, content);
        todo!("Phase 2: write content to base_path/artifacts/filename")
    }

    /// Read an artifact from the current run by filename.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`] when the store is disabled,
    /// [`ArtifactError::Io`] when the read fails.
    pub async fn read_artifact(&self, filename: &ArtifactFilename) -> Result<String, ArtifactError> {
        let _ = filename;
        todo!("Phase 2: read base_path/artifacts/filename")
    }

    /// Read an artifact from a different run in the same session.
    ///
    /// The `run_id` is a [`RunId`] — already validated as a safe path
    /// component — and the resolved path is checked against the session
    /// directory to prevent traversal.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`], [`ArtifactError::Io`].
    pub async fn read_artifact_cross_run(
        &self,
        filename: &ArtifactFilename,
        run_id: &RunId,
    ) -> Result<String, ArtifactError> {
        let _ = (filename, run_id);
        todo!("Phase 2: resolve cross-run path with traversal guard, read")
    }
}
