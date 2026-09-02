//! The sidecar client: rmcp streamable-HTTP transport behind a plain-JSON
//! boundary.
//!
//! `SidecarClient` is the only type that touches the MCP transport. rmcp's
//! types (`RunningService`, `Peer`, `Tool`, `CallToolResult`) stay private
//! to this module; the public methods take and return plain JSON-shaped
//! data, so no rmcp type ever crosses the seam and `producers.rs` and the
//! worker path are insulated from the SDK.
//!
//! [`SidecarClient::connect`] performs the full MCP handshake — rmcp's
//! `serve_client` sends `initialize`, awaits the result, and delivers the
//! `notifications/initialized` notification — so a client returned by
//! `connect` is ready for `tools/list` and `tools/call` with no further
//! ceremony. [`SidecarClient::initialize`] reports the handshake result
//! and keeps its place in the public surface so existing callers are
//! unchanged.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderName, HeaderValue};
use tokio::time::timeout;

use super::wire::{
    SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName,
};

/// Bound on how long a single `tools/list` or `tools/call` round trip may
/// wait for the server's response.
///
/// The bound matches the hand-rolled client it replaces: a server that
/// accepts the request but never answers fails loud instead of parking the
/// caller forever. Abandoning the wait does not cancel the request
/// downstream; rmcp's cancellation propagation is a separate adoption
/// concern, deliberately out of scope here.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// The endpoint of an MCP server.
///
/// Forbidden invalid state: an empty or non-HTTP URL reaching the connect
/// step. The constructor checks for a non-empty string with an `http://` or
/// `https://` prefix; rmcp parses the rest when the transport opens.
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

impl fmt::Display for SidecarUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a sidecar operation failed.
///
/// Variants carry enough context to diagnose the failure without repeating
/// the full request body. rmcp sub-errors (handshake, transport, JSON-RPC)
/// land as string descriptions on [`SidecarError::Connect`] (handshake) or
/// [`SidecarError::Protocol`] (post-handshake requests); the variant set is
/// closed so callers can match exhaustively.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SidecarError {
    /// The URL was empty or not an HTTP(S) URL.
    #[error("invalid sidecar URL: {0}")]
    InvalidUrl(String),
    /// The handshake could not be completed: transport, HTTP, or
    /// initialize-round-trip failure.
    #[error("sidecar connection failed: {0}")]
    Connect(String),
    /// A post-handshake request was malformed, rejected, or timed out.
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
    /// A configured static header had an invalid name or value.
    #[error("invalid header: {0}")]
    InvalidHeader(String),
}

/// The client identity rmcp presents in the `initialize` handshake.
fn client_info() -> rmcp::model::ClientInfo {
    rmcp::model::InitializeRequestParams::new(
        rmcp::model::ClientCapabilities::default(),
        rmcp::model::Implementation::new("agent-driver-prototype", env!("CARGO_PKG_VERSION")),
    )
}

/// Convert plain string pairs into the typed header map rmcp's transport
/// config takes.
///
/// `http` 1.x unifies across this crate's reqwest 0.12 and rmcp's reqwest
/// 0.13, so `reqwest::header::{HeaderName, HeaderValue}` are the exact
/// types rmcp expects. Header names that rmcp reserves for its own session
/// protocol pass this check and are rejected by the transport at request
/// time with `ReservedHeaderConflict`.
///
/// # Errors
///
/// Returns [`SidecarError::InvalidHeader`] for a name that is not a valid
/// HTTP header token or a value that is not a valid HTTP header value.
fn header_map(
    headers: &HashMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>, SidecarError> {
    let mut map = HashMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name = HeaderName::try_from(name.as_str())
            .map_err(|e| SidecarError::InvalidHeader(format!("header name {name:?}: {e}")))?;
        let value = HeaderValue::try_from(value.as_str())
            .map_err(|e| SidecarError::InvalidHeader(format!("header {name:?} value: {e}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

/// Extract the server identity from the handshake rmcp already recorded.
///
/// # Errors
///
/// Returns [`SidecarError::Protocol`] when the server reported no
/// implementation identity in its initialize result.
fn server_info(peer_info: &rmcp::model::ServerPeerInfo) -> Result<SidecarServerInfo, SidecarError> {
    let Some(implementation) = peer_info.server_info.as_ref() else {
        return Err(SidecarError::Protocol(
            "initialize response missing serverInfo".to_owned(),
        ));
    };
    Ok(SidecarServerInfo {
        protocol_version: peer_info.protocol_version.as_str().to_owned(),
        server_name: implementation.name.clone(),
        server_version: implementation.version.clone(),
    })
}

/// Join the text blocks of a `tools/call` result, matching the shape the
/// hand-rolled client returned: text blocks joined with `\n`, non-text
/// blocks skipped.
fn text_content(result: &rmcp::model::CallToolResult) -> String {
    let mut text = String::new();
    for block in &result.content {
        if let rmcp::model::ContentBlock::Text(text_block) = block {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&text_block.text);
        }
    }
    text
}

/// The live session state shared by every clone of a [`SidecarClient`].
///
/// Dropping the last clone drops the `RunningService`, whose guard cancels
/// the session — the same teardown shape the hand-rolled client had when
/// the SSE stream dropped.
struct Shared {
    url: SidecarUrl,
    service: Option<rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientInfo>>,
    handshake: Option<SidecarServerInfo>,
}

/// The JSON-boundary client for an MCP server over rmcp streamable HTTP.
///
/// Cloning shares the connection state, so worker tools that need the
/// server each take a cheap clone. The rmcp service lives behind the `Arc`;
/// no rmcp type appears in any public signature.
#[derive(Clone)]
pub struct SidecarClient {
    shared: Arc<Shared>,
}

impl fmt::Debug for SidecarClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SidecarClient")
            .field("url", &self.shared.url)
            .field("connected", &self.shared.service.is_some())
            .finish_non_exhaustive()
    }
}

impl SidecarClient {
    /// Connect to an MCP server over streamable HTTP and complete the MCP
    /// handshake.
    ///
    /// The returned client is ready for [`Self::list_tools`] and
    /// [`Self::call_tool`] immediately: rmcp sends `initialize`, reads the
    /// result, and delivers `notifications/initialized` before this
    /// returns, so a server that would reject early requests never sees
    /// one.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::InvalidUrl`] when the URL does not parse and
    /// [`SidecarError::Connect`] when the transport cannot be established
    /// or the handshake fails.
    pub async fn connect(url: SidecarUrl) -> Result<Self, SidecarError> {
        Self::connect_streamable(url, HashMap::new()).await
    }

    /// Connect over streamable HTTP with static headers sent on every
    /// request.
    ///
    /// The headers are the `[mcp.servers.*.headers]` block of the
    /// orchestration TOML — the auth headers the mezmo server expects on
    /// every POST and GET. They ride rmcp's per-request custom-header
    /// channel, which applies them to every request of the session.
    ///
    /// # Errors
    ///
    /// As [`Self::connect`], plus [`SidecarError::InvalidHeader`] when a
    /// header name or value is not HTTP-valid.
    pub async fn connect_streamable(
        url: SidecarUrl,
        headers: HashMap<String, String>,
    ) -> Result<Self, SidecarError> {
        let config =
            rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
                url.as_str(),
            )
            .custom_headers(header_map(&headers)?);
        let transport = rmcp::transport::StreamableHttpClientTransport::from_config(config);
        let service = rmcp::serve_client(client_info(), transport)
            .await
            .map_err(|e| SidecarError::Connect(format!("streamable HTTP handshake failed: {e}")))?;
        let handshake = server_info(service.peer_info().as_deref().ok_or_else(|| {
            SidecarError::Connect("handshake recorded no server info".to_owned())
        })?)?;
        Ok(Self {
            shared: Arc::new(Shared {
                url,
                service: Some(service),
                handshake: Some(handshake),
            }),
        })
    }

    /// A non-functional client for test construction.
    ///
    /// The client carries a dummy URL and no transport. Any method that
    /// forwards through it fails with [`SidecarError::Connect`]. Use this
    /// only when the sidecar tools (`keystrokes`, `capture-pane`) are not
    /// exercised, e.g. in tests with `MockProvider`-backed workers that
    /// only call `submit_result`.
    pub fn disconnected() -> Self {
        let url = SidecarUrl::new("http://localhost:0/mcp").expect("valid dummy URL");
        Self {
            shared: Arc::new(Shared {
                url,
                service: None,
                handshake: None,
            }),
        }
    }

    /// Report the server info from the handshake [`Self::connect`]
    /// completed.
    ///
    /// The handshake itself runs inside `connect`; this returns its result
    /// so callers that log or check the server identity keep their place.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Connect`] on the disconnected test client.
    pub fn initialize(&self) -> Result<SidecarServerInfo, SidecarError> {
        self.shared.handshake.clone().ok_or_else(|| {
            SidecarError::Connect("client is the disconnected test placeholder".to_owned())
        })
    }

    /// Send `tools/list` and read the tool definitions.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] when the request fails, times
    /// out, or the response is malformed, and [`SidecarError::EmptyToolName`]
    /// when a tool entry has an empty name.
    pub async fn list_tools(&self) -> Result<Vec<SidecarTool>, SidecarError> {
        let service = self.live_service()?;
        let tools = timeout(RESPONSE_TIMEOUT, service.peer().list_all_tools())
            .await
            .map_err(|_| {
                SidecarError::Protocol(format!("tools/list timed out after {RESPONSE_TIMEOUT:?}"))
            })?
            .map_err(|e| SidecarError::Protocol(format!("tools/list failed: {e}")))?;
        let mut out = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = SidecarToolName::new(tool.name.as_ref())?;
            let description = tool.description.as_deref().unwrap_or("").to_owned();
            let input_schema = tool.schema_as_json_value();
            out.push(SidecarTool::new(name, description, input_schema));
        }
        Ok(out)
    }

    /// Send `tools/call` and read the text content.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::ToolCall`] when the server reports
    /// `isError: true` or returns no content, and [`SidecarError::Protocol`]
    /// when the request fails, times out, or the response is a shape this
    /// client does not drive (input-required rounds, async tasks).
    pub async fn call_tool(
        &self,
        name: &SidecarToolName,
        args: &SidecarToolArgs,
    ) -> Result<SidecarContent, SidecarError> {
        let service = self.live_service()?;
        // The params type owns a 'static Cow, so the borrowed tool name is
        // copied once here.
        let params = rmcp::model::CallToolRequestParams::new(name.as_str().to_owned())
            .with_arguments(args.inner().clone());
        let response = timeout(RESPONSE_TIMEOUT, service.peer().call_tool_once(params))
            .await
            .map_err(|_| {
                SidecarError::Protocol(format!(
                    "tools/call {name} timed out after {RESPONSE_TIMEOUT:?}"
                ))
            })?
            .map_err(|e| SidecarError::Protocol(format!("tools/call {name} failed: {e}")))?;
        let result = match response {
            rmcp::model::CallToolResponse::Complete(result) => result,
            rmcp::model::CallToolResponse::InputRequired(_) => {
                return Err(SidecarError::Protocol(format!(
                    "tools/call {name} returned input_required; this client does not drive MRTR rounds"
                )));
            }
            rmcp::model::CallToolResponse::Task(_) => {
                return Err(SidecarError::Protocol(format!(
                    "tools/call {name} returned a task; this client does not poll tasks"
                )));
            }
            // `CallToolResponse` is non_exhaustive upstream: a variant added
            // after this match is an unsupported shape, not a silent drop.
            unexpected => {
                return Err(SidecarError::Protocol(format!(
                    "tools/call {name} returned an unsupported response shape: {unexpected:?}"
                )));
            }
        };
        let text = text_content(&result);
        if result.is_error == Some(true) {
            let msg = if text.is_empty() {
                format!("tools/call {name} returned isError with no text")
            } else {
                text
            };
            return Err(SidecarError::ToolCall(msg));
        }
        if result.content.is_empty() {
            return Err(SidecarError::ToolCall(format!(
                "tools/call {name} returned no content"
            )));
        }
        Ok(SidecarContent::new(text))
    }

    /// The live service handle, or the disconnected-client error.
    fn live_service(
        &self,
    ) -> Result<
        &rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientInfo>,
        SidecarError,
    > {
        self.shared.service.as_ref().ok_or_else(|| {
            SidecarError::Connect("client is the disconnected test placeholder".to_owned())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    // --- SidecarUrl -------------------------------------------------------

    #[test]
    fn sidecar_url_rejects_empty_and_whitespace() {
        assert!(SidecarUrl::new("").is_err());
        assert!(SidecarUrl::new("   ").is_err());
        assert!(SidecarUrl::new("\t\n").is_err());
    }

    #[test]
    fn sidecar_url_rejects_non_http_scheme() {
        assert!(SidecarUrl::new("ftp://localhost/mcp").is_err());
        assert!(SidecarUrl::new("localhost:8000/mcp").is_err());
        assert!(SidecarUrl::new("ws://localhost/mcp").is_err());
    }

    #[test]
    fn sidecar_url_accepts_http_and_https_and_trims() {
        let http = SidecarUrl::new("http://localhost:8000/mcp").unwrap();
        assert_eq!(http.as_str(), "http://localhost:8000/mcp");
        let https = SidecarUrl::new("https://example.com/api/mcp").unwrap();
        assert_eq!(https.as_str(), "https://example.com/api/mcp");
        let trimmed = SidecarUrl::new("  http://localhost:8000/mcp  ").unwrap();
        assert_eq!(trimmed.as_str(), "http://localhost:8000/mcp");
    }

    // --- Static header conversion ------------------------------------------

    #[test]
    fn header_map_accepts_the_mezmo_auth_block_shape() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_owned(), "Bearer s3cret".to_owned());
        headers.insert("x-mock-scenario".to_owned(), "db-pool".to_owned());

        let map = header_map(&headers).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get(&HeaderName::from_static("authorization")),
            Some(&HeaderValue::from_static("Bearer s3cret"))
        );
    }

    #[test]
    fn header_map_rejects_invalid_name_and_value() {
        let mut headers = HashMap::new();
        headers.insert("not a header name".to_owned(), "v".to_owned());
        assert!(matches!(
            header_map(&headers),
            Err(SidecarError::InvalidHeader(_))
        ));

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_owned(), "bad\nvalue".to_owned());
        assert!(matches!(
            header_map(&headers),
            Err(SidecarError::InvalidHeader(_))
        ));
    }

    #[test]
    fn header_map_empty_is_empty() {
        assert!(header_map(&HashMap::new()).unwrap().is_empty());
    }

    // --- Server info from the handshake ------------------------------------

    #[test]
    fn server_info_maps_peer_identity() {
        let peer = rmcp::model::ServerPeerInfo::new(
            rmcp::model::ProtocolVersion::default(),
            rmcp::model::ServerCapabilities::default(),
        )
        .with_server_info(rmcp::model::Implementation::new("t-bench", "1.6.0"));

        let info = server_info(&peer).unwrap();
        assert_eq!(info.server_name, "t-bench");
        assert_eq!(info.server_version, "1.6.0");
        assert_eq!(
            info.protocol_version,
            rmcp::model::ProtocolVersion::default().as_str()
        );
    }

    #[test]
    fn server_info_rejects_missing_identity() {
        let peer = rmcp::model::ServerPeerInfo::new(
            rmcp::model::ProtocolVersion::default(),
            rmcp::model::ServerCapabilities::default(),
        );

        assert_eq!(
            server_info(&peer).unwrap_err(),
            SidecarError::Protocol("initialize response missing serverInfo".to_owned())
        );
    }

    // --- tools/call text extraction ---------------------------------------

    #[test]
    fn text_content_joins_text_blocks_and_skips_the_rest() {
        let result = rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text("line one"),
            rmcp::model::ContentBlock::text("line two"),
        ]);
        assert_eq!(text_content(&result), "line one\nline two");

        let image = rmcp::model::ContentBlock::image("aGk=", "image/png");
        let result = rmcp::model::CallToolResult::success(vec![image]);
        assert_eq!(text_content(&result), "");
    }

    // --- The disconnected placeholder --------------------------------------

    #[test]
    fn disconnected_client_fails_loud_on_every_path() {
        let client = SidecarClient::disconnected();

        assert!(matches!(client.initialize(), Err(SidecarError::Connect(_))));

        let name = SidecarToolName::new("keystrokes").unwrap();
        let args = SidecarToolArgs::from_value(json!({})).unwrap();
        let list = client.list_tools();
        let call = client.call_tool(&name, &args);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime builds");
        rt.block_on(async {
            assert!(matches!(list.await, Err(SidecarError::Connect(_))));
            assert!(matches!(call.await, Err(SidecarError::Connect(_))));
        });
    }

    #[test]
    fn clones_share_state() {
        let client = SidecarClient::disconnected();
        let clone = client.clone();
        // Both clones report the same placeholder URL through Debug.
        assert!(format!("{client:?}").contains("localhost:0"));
        assert!(format!("{clone:?}").contains("localhost:0"));
    }
}
