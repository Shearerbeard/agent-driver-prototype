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
            return Err(ShimError::InvalidRequest("config path is empty".to_owned()));
        }
        Ok(Self {
            port,
            sidecar_url,
            config_path,
        })
    }

    /// Parse `std::env::args` into typed CLI args.
    ///
    /// Expected: `sse_shim --port <N> --sidecar-url <URL> --config <PATH>`.
    /// Both `--flag value` and `--flag=value` forms are accepted; the program
    /// name (argv[0]) is skipped. Unknown flags and missing values are hard
    /// errors. The sidecar URL is validated via [`SidecarUrl::new`].
    ///
    /// # Errors
    ///
    /// Returns [`ShimError::InvalidRequest`] when required args are missing,
    /// a value is malformed, or an unknown argument is present.
    pub fn parse() -> Result<Self, ShimError> {
        let mut args = std::env::args().skip(1);
        let mut port: Option<u16> = None;
        let mut sidecar_url: Option<String> = None;
        let mut config_path: Option<PathBuf> = None;

        while let Some(arg) = args.next() {
            // Split `--flag=value` so the value is not mistaken for the next
            // flag. `split_once` splits on the first `=`, so a URL value with
            // `=` in its query string is preserved intact.
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) => (f, Some(v.to_owned())),
                None => (arg.as_str(), None),
            };
            match flag {
                "--port" => {
                    let value = take_value(flag, inline, &mut args)?;
                    let parsed: u16 = value.parse().map_err(|_| {
                        ShimError::InvalidRequest(format!(
                            "--port must be a port number, got {value}"
                        ))
                    })?;
                    port = Some(parsed);
                }
                "--sidecar-url" => {
                    sidecar_url = Some(take_value(flag, inline, &mut args)?);
                }
                "--config" => {
                    config_path = Some(PathBuf::from(take_value(flag, inline, &mut args)?));
                }
                other => {
                    return Err(ShimError::InvalidRequest(format!(
                        "unknown CLI argument: {other}"
                    )));
                }
            }
        }

        let port =
            port.ok_or_else(|| ShimError::InvalidRequest("missing required --port".to_owned()))?;
        let sidecar_raw = sidecar_url.ok_or_else(|| {
            ShimError::InvalidRequest("missing required --sidecar-url".to_owned())
        })?;
        let config_path = config_path
            .ok_or_else(|| ShimError::InvalidRequest("missing required --config".to_owned()))?;
        let sidecar_url =
            SidecarUrl::new(&sidecar_raw).map_err(|e| ShimError::InvalidRequest(e.to_string()))?;

        Self::from_parts(ShimPort::new(port), sidecar_url, config_path)
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

/// Pull a flag's value from an inline `--flag=value` or the next argv slot.
///
/// # Errors
///
/// Returns [`ShimError::InvalidRequest`] when no value follows the flag.
fn take_value(
    flag: &str,
    inline: Option<String>,
    args: &mut impl Iterator<Item = String>,
) -> Result<String, ShimError> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(ShimError::InvalidRequest(format!("empty value for {flag}")));
        }
        return Ok(value);
    }
    match args.next() {
        Some(value) if !value.is_empty() => Ok(value),
        Some(_) => Err(ShimError::InvalidRequest(format!("empty value for {flag}"))),
        None => Err(ShimError::InvalidRequest(format!(
            "missing value for {flag}"
        ))),
    }
}
