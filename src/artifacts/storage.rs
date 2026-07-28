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
/// each take a cheap clone. The base path is the run directory; artifacts
/// live under `base_path/artifacts/`. Cross-run reads resolve through the
/// session directory (the run directory's parent) and are guarded by
/// [`ArtifactFilename`]'s and [`RunId`]'s constructors, which reject path
/// separators and `..`.
#[derive(Debug, Clone)]
pub struct ArtifactStore {
    base_path: PathBuf,
    _shared: Arc<()>,
}

impl ArtifactStore {
    /// Create a store rooted at `base_path`.
    ///
    /// The directory is not created here; [`write_artifact`](Self::write_artifact)
    /// creates the `artifacts/` subtree on first use via `create_dir_all`.
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

    fn is_disabled(&self) -> bool {
        self.base_path.as_os_str().is_empty()
    }

    fn artifacts_dir(&self) -> PathBuf {
        self.base_path.join("artifacts")
    }

    /// Write `content` to an artifact file and return its filename.
    ///
    /// Creates the `artifacts/` directory on first use (resolves R2).
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`] when the store is disabled, and
    /// [`ArtifactError::Io`] when the directory creation or write fails.
    pub async fn write_artifact(
        &self,
        filename: &ArtifactFilename,
        content: &str,
    ) -> Result<ArtifactFilename, ArtifactError> {
        if self.is_disabled() {
            return Err(ArtifactError::Disabled);
        }
        let dir = self.artifacts_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        let path = dir.join(filename.as_str());
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ArtifactError::Io(e.to_string()))?;
        Ok(filename.clone())
    }

    /// Read an artifact from the current run by filename.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`] when the store is disabled,
    /// [`ArtifactError::Io`] when the read fails (including file-not-found,
    /// surfaced as the io error message).
    pub async fn read_artifact(
        &self,
        filename: &ArtifactFilename,
    ) -> Result<String, ArtifactError> {
        if self.is_disabled() {
            return Err(ArtifactError::Disabled);
        }
        let path = self.artifacts_dir().join(filename.as_str());
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ArtifactError::Io(e.to_string()))
    }

    /// Read an artifact from a different run in the same session.
    ///
    /// The `run_id` is a [`RunId`] — already validated as a safe path
    /// component — and the resolved path is checked against the session
    /// directory to prevent traversal.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::Disabled`] when the store is disabled,
    /// [`ArtifactError::Io`] when the read fails or the session directory
    /// cannot be resolved.
    pub async fn read_artifact_cross_run(
        &self,
        filename: &ArtifactFilename,
        run_id: &RunId,
    ) -> Result<String, ArtifactError> {
        if self.is_disabled() {
            return Err(ArtifactError::Disabled);
        }
        let session_dir = self.base_path.parent().ok_or_else(|| {
            ArtifactError::Io("cannot resolve session directory from base path".to_owned())
        })?;
        let cross_path = session_dir
            .join(run_id.as_str())
            .join("artifacts")
            .join(filename.as_str());
        if !cross_path.starts_with(session_dir) {
            return Err(ArtifactError::UnsafeFilename);
        }
        tokio::fs::read_to_string(&cross_path)
            .await
            .map_err(|e| ArtifactError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::InlineThreshold;

    #[tokio::test]
    async fn write_then_read_round_trips_content() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let store = ArtifactStore::new(dir.path().to_path_buf());
        let filename = ArtifactFilename::new("result.txt").expect("valid filename");

        store
            .write_artifact(&filename, "hello world")
            .await
            .expect("write");
        let read = store.read_artifact(&filename).await.expect("read");
        assert_eq!(read, "hello world");
    }

    #[tokio::test]
    async fn cross_run_read_finds_artifact_in_sibling_run() {
        let session = tempfile::TempDir::new().expect("temp dir");
        let run_a = session.path().join("run-a");
        let run_b = session.path().join("run-b");
        let store_a = ArtifactStore::new(run_a);
        let store_b = ArtifactStore::new(run_b);

        let filename = ArtifactFilename::new("shared.txt").expect("valid filename");
        store_a
            .write_artifact(&filename, "cross-run content")
            .await
            .expect("write in run-a");

        let run_id = RunId::new("run-a").expect("valid run id");
        let read = store_b
            .read_artifact_cross_run(&filename, &run_id)
            .await
            .expect("cross-run read");
        assert_eq!(read, "cross-run content");
    }

    #[tokio::test]
    async fn cross_run_read_missing_run_returns_io_error() {
        let session = tempfile::TempDir::new().expect("temp dir");
        let store = ArtifactStore::new(session.path().join("current"));
        let filename = ArtifactFilename::new("missing.txt").expect("valid filename");
        let run_id = RunId::new("nonexistent").expect("valid run id");
        let err = store
            .read_artifact_cross_run(&filename, &run_id)
            .await
            .expect_err("should fail");
        assert!(matches!(err, ArtifactError::Io(_)));
    }

    #[test]
    fn traversal_filenames_are_rejected_at_parse_time() {
        assert!(ArtifactFilename::new("../etc/passwd").is_err());
        assert!(ArtifactFilename::new("..").is_err());
        assert!(ArtifactFilename::new(".").is_err());
        assert!(ArtifactFilename::new("foo/bar").is_err());
        assert!(ArtifactFilename::new("foo\\bar").is_err());
        assert!(RunId::new("../escape").is_err());
        assert!(RunId::new("..").is_err());
    }

    #[tokio::test]
    async fn disabled_store_rejects_writes_and_reads() {
        let store = ArtifactStore::disabled();
        let filename = ArtifactFilename::new("x.txt").expect("valid filename");
        assert_eq!(
            store.write_artifact(&filename, "data").await,
            Err(ArtifactError::Disabled)
        );
        assert_eq!(
            store.read_artifact(&filename).await,
            Err(ArtifactError::Disabled)
        );
        let run_id = RunId::new("other").expect("valid run id");
        assert_eq!(
            store.read_artifact_cross_run(&filename, &run_id).await,
            Err(ArtifactError::Disabled)
        );
    }

    #[test]
    fn inline_threshold_boundary_allows_below_spills_at_or_above() {
        let threshold = InlineThreshold::new(10).expect("non-zero");
        assert!(threshold.allows_inline("123456789"));
        assert!(!threshold.allows_inline("1234567890"));
        assert!(!threshold.allows_inline("12345678901"));
    }

    #[test]
    fn inline_threshold_rejects_zero() {
        assert_eq!(InlineThreshold::new(0), Err(ArtifactError::Disabled));
    }
}
