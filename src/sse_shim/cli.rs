//! CLI argument types for the SSE shim binary.
//!
//! Parsed from `std::env::args` to match the existing `sidecar_probe`
//! convention — the spike repo does not depend on `clap`.

use std::path::PathBuf;

use crate::mcp_client::SidecarUrl;

use super::error::ShimError;

/// The TCP port the shim listens on.
///
/// `0` means "bind to an ephemeral port and report the bound port on stdout
/// as `SHIM_PORT=<n>`". Any `u16` is valid, so this is a newtype alias.
///
/// Forbidden invalid state: none at the type level — `u16` prevents values
/// outside `[0, 65535]`. The ephemeral sentinel is `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShimPort(u16);

impl ShimPort {
    /// Construct a port. Infallible: any `u16` is valid.
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self(port)
    }

    /// The raw port value.
    #[must_use]
    pub fn get(self) -> u16 {
        self.0
    }

    /// Whether this is the ephemeral sentinel (`0`).
    #[must_use]
    pub fn is_ephemeral(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for ShimPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Parsed CLI arguments for the shim binary.
///
/// Forbidden invalid state: a missing sidecar URL or config path reaching
/// the server startup. The constructor validates all three fields.
#[derive(Debug, Clone)]
pub struct ShimCliArgs {
    port: ShimPort,
    sidecar_url: SidecarUrl,
    config_path: PathBuf,
}

impl ShimCliArgs {
    /// Construct CLI args from their typed parts.
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when the config path is empty.
    pub fn from_parts(
        port: ShimPort,
        sidecar_url: SidecarUrl,
        config_path: PathBuf,
    ) -> Result<Self, ShimError> {
        if config_path.as_os_str().is_empty() {
            return Err(ShimError::InvalidRequest(
                "config path is empty".to_owned(),
            ));
        }
        Ok(Self {
            port,
            sidecar_url,
            config_path,
        })
    }

    /// Parse `std::env::args` into typed CLI args.
    ///
    /// Expected: `sse_shim --port <N> --sidecar-url <URL> --config <PATH>`
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when required args are missing
    /// or malformed.
    ///
    /// # Panics
    ///
    /// This method body is `todo!()` in the type skeleton.
    pub fn parse() -> Result<Self, ShimError> {
        todo!("parse --port, --sidecar-url, --config from std::env::args")
    }

    /// The listen port.
    pub fn port(&self) -> ShimPort {
        self.port
    }

    /// The sidecar SSE URL.
    pub fn sidecar_url(&self) -> &SidecarUrl {
        &self.sidecar_url
    }

    /// The config file path.
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }
}
