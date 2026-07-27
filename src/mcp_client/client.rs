//! The sidecar client: connection lifecycle and the three JSON-RPC calls.
//!
//! `SidecarClient` is the only type that touches the SSE transport. The
//! rmcp `Transport<RoleClient>` trait and its version-pinned types stay
//! private to this module; the public methods take and return plain JSON.

use std::sync::Arc;

use super::wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};

/// The SSE endpoint of a TerminalBench sidecar.
///
/// Forbidden invalid state: an empty or non-HTTP URL reaching the connect
/// step. The constructor checks for a non-empty string with an `http://` or
/// `https://` prefix; full URL parsing lands in Phase 2 with the `url` crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarUrl(String);

impl SidecarUrl {
    /// Parse a sidecar URL.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::InvalidUrl`] when `url` is empty,
    /// whitespace-only, or does not start with `http://` or `https://`.
    pub fn new(url: &str) -> Result<Self, SidecarError> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(SidecarError::InvalidUrl("url is empty".to_owned()));
        }
        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
            return Err(SidecarError::InvalidUrl(
                "url must start with http:// or https://".to_owned(),
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The URL as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SidecarUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a sidecar operation failed.
///
/// Variants carry enough context to diagnose the failure without repeating
/// the full request body. The SSE-transport-specific sub-errors (HTTP status,
/// content-type mismatch, stream decode) land as string descriptions in
/// Phase 2; the variant set is final now so callers can match exhaustively.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SidecarError {
    /// The URL was empty or not an HTTP(S) URL.
    #[error("invalid sidecar URL: {0}")]
    InvalidUrl(String),
    /// The SSE connection could not be established.
    #[error("sidecar connection failed: {0}")]
    Connect(String),
    /// The server sent no `endpoint` event before the stream closed.
    #[error("SSE stream closed before the endpoint event arrived")]
    MissingEndpointEvent,
    /// A JSON-RPC response was malformed or carried an error result.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// A tool call returned `isError: true` or no content.
    #[error("tool call failed: {0}")]
    ToolCall(String),
    /// A tool name was empty or whitespace-only.
    #[error("tool name is empty")]
    EmptyToolName,
    /// `tools/call` arguments were not a JSON object.
    #[error("tool arguments must be a JSON object")]
    ArgumentsNotObject,
}

/// The JSON-boundary client for a classic-SSE MCP sidecar.
///
/// Cloning shares the connection state, so worker tools that need the
/// sidecar each take a cheap clone. The transport internals (reqwest client,
/// SSE stream handle, message endpoint URL) land in Phase 2 behind the
/// `Arc`; the skeleton holds only the URL so the type is constructable.
#[derive(Debug, Clone)]
pub struct SidecarClient {
    url: SidecarUrl,
    _shared: Arc<()>,
}

impl SidecarClient {
    /// Connect to a sidecar and complete the `initialize` handshake.
    ///
    /// Returns a client ready for `list_tools` and `call_tool`. The
    /// skeleton stores the URL; the SSE connect + initialize body lands in
    /// Phase 2.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Connect`] when the SSE stream cannot be
    /// opened or the `endpoint` event is missing.
    pub async fn connect(url: SidecarUrl) -> Result<Self, SidecarError> {
        Ok(Self {
            url,
            _shared: Arc::new(()),
        })
    }

    /// The endpoint this client was constructed for.
    pub fn url(&self) -> &SidecarUrl {
        &self.url
    }

    /// Send `initialize` and read the server info.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] when the response is malformed.
    pub async fn initialize(&self) -> Result<SidecarServerInfo, SidecarError> {
        todo!("Phase 2: send initialize JSON-RPC, parse protocolVersion + serverInfo")
    }

    /// Send `tools/list` and read the tool definitions.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] when the response is malformed.
    pub async fn list_tools(&self) -> Result<Vec<SidecarTool>, SidecarError> {
        todo!("Phase 2: send tools/list JSON-RPC, parse the tools array")
    }

    /// Send `tools/call` and read the text content.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::ToolCall`] when the sidecar reports
    /// `isError: true`, and [`SidecarError::Protocol`] when the response is
    /// malformed.
    pub async fn call_tool(
        &self,
        name: &SidecarToolName,
        args: &SidecarToolArgs,
    ) -> Result<SidecarContent, SidecarError> {
        let _ = (name, args);
        todo!("Phase 2: send tools/call JSON-RPC, parse content text")
    }
}
