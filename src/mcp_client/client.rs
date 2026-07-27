//! The sidecar client: connection lifecycle and the three JSON-RPC calls.
//!
//! `SidecarClient` is the only type that touches the SSE transport. The
//! rmcp `Transport<RoleClient>` trait and its version-pinned types stay
//! private to this module; the public methods take and return plain JSON.
//!
//! Transport shape (ported from aura's `mcp_sse.rs`, classic SSE, 2024-11-05
//! spec): `GET /sse` opens a long-lived event stream; the first event is
//! `event: endpoint` carrying the relative messages URL; JSON-RPC requests
//! are `POST`ed to that URL and return `202 Accepted`; responses arrive as
//! `event: message` frames on the stream, interleaved with `: ping` comment
//! lines. The MCP handshake requires a `notifications/initialized`
//! notification (no id, no response) after the `initialize` result before
//! any further request; [`SidecarClient::initialize`] sends it automatically
//! so callers cannot forget it. No rmcp type is used — the JSON-RPC envelope
//! is built and parsed with `serde_json` so the rmcp 0.12-vs-1.7 version gap
//! never crosses this seam.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value as JsonValue;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::timeout;

use super::wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};

/// Bound on how long a single JSON-RPC response read may block the stream.
///
/// The SSE stream itself stays open for the client's lifetime; this guards
/// only the wait for the next response frame after a POST.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound on how long `connect` may wait for the sidecar's opening
/// `event: endpoint` frame.
///
/// The SSE GET response is already open by the time this runs; the bound
/// guards only the wait for the first dispatchable frame. A wire-shape
/// regression that the parser cannot split (a terminator it does not
/// recognize) would otherwise leave the read pending forever on an
/// open-but-silent stream — this timeout makes it fail loudly as a
/// [`SidecarError::Connect`] instead of hanging.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// The SSE endpoint of a TerminalBench sidecar.
///
/// Forbidden invalid state: an empty or non-HTTP URL reaching the connect
/// step. The constructor checks for a non-empty string with an `http://` or
/// `https://` prefix. Full RFC 3986 parsing happens in
/// [`SidecarClient::connect`] via the `url` crate, which also resolves the
/// relative `endpoint` event URL against this base.
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
/// the full request body. SSE-transport sub-errors (HTTP status, content-type
/// mismatch, stream decode, timeout, response-id mismatch) land as string
/// descriptions on [`SidecarError::Connect`] (connection establishment) or
/// [`SidecarError::Protocol`] (post-connect protocol); the variant set is
/// closed so callers can match exhaustively.
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

// ===========================================================================
// SSE frame parsing (pure, transport-independent)
// ===========================================================================

/// One decoded SSE item: either a dispatched event frame or a comment line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SseItem {
    /// An event frame with an optional `event:` type and the joined `data:`
    /// payload. `event == None` means no `event:` field was present, which the
    /// SSE spec dispatches as the default `message` event.
    Frame {
        event: Option<String>,
        data: String,
    },
    /// A `: ...` comment line (keep-alive ping). Carries no event or data.
    Comment(String),
}

/// Split an SSE field line into `(field, value)`, stripping the one optional
/// leading space after the colon per the WHATWG SSE spec.
fn split_sse_field(line: &str) -> (&str, &str) {
    match line.find(':') {
        Some(idx) => {
            let field = &line[..idx];
            let mut value = &line[idx + 1..];
            if let Some(stripped) = value.strip_prefix(' ') {
                value = stripped;
            }
            (field, value)
        }
        // A line with no colon and not starting with `:` is ignored by the
        // spec; report it as an empty field with no value.
        None => (line, ""),
    }
}

/// Parse one complete SSE block (the text between two blank-line terminators)
/// into an item, or `None` for an empty block.
fn parse_sse_block(block: &str) -> Option<SseItem> {
    let mut event: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    for line in block.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(':') {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            comments.push(rest.to_owned());
            continue;
        }
        let (field, value) = split_sse_field(line);
        match field {
            "event" => event = Some(value.to_owned()),
            "data" => data_lines.push(value.to_owned()),
            // `id`, `retry`, and unknown fields are ignored by this client.
            _ => {}
        }
    }
    if !data_lines.is_empty() || event.is_some() {
        Some(SseItem::Frame {
            event,
            data: data_lines.join("\n"),
        })
    } else if !comments.is_empty() {
        Some(SseItem::Comment(comments.join("\n")))
    } else {
        None
    }
}

/// Length of the SSE line break starting at `bytes[i]`, or `None` when
/// `bytes[i]` is not a line break.
///
/// Per the WHATWG SSE spec a line break is `\r\n`, `\n`, or a lone `\r`;
/// `\r\n` is checked first so it counts as one break, not two.
fn line_break_len(bytes: &[u8], i: usize) -> Option<usize> {
    match bytes[i] {
        b'\n' => Some(1),
        b'\r' => Some(if i + 1 < bytes.len() && bytes[i + 1] == b'\n' { 2 } else { 1 }),
        _ => None,
    }
}

/// Find the next SSE blank-line terminator in `buf` and return
/// `(start, len)`: `start` is the byte index where the terminator begins
/// (the line break ending the last field line) and `len` is the full
/// terminator length (both consecutive line breaks).
///
/// Returns `None` when no complete blank line is present yet, including
/// the partial case where the buffer ends immediately after a single line
/// break — the second break may arrive in a later chunk, so the partial
/// block must stay buffered. Real sidecars send LF (`\n\n`) or CRLF
/// (`\r\n\r\n`); mixed endings (`\r\n\n`, `\n\r\n`) and CR-only are
/// accepted too.
fn find_frame_terminator(buf: &str) -> Option<(usize, usize)> {
    let bytes = buf.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let Some(lb1) = line_break_len(bytes, i) else {
            i += 1;
            continue;
        };
        let after_first = i + lb1;
        if after_first >= bytes.len() {
            // Buffer ends right after the first break: a second break could
            // still arrive in a later chunk, so do not emit a frame yet.
            return None;
        }
        if let Some(lb2) = line_break_len(bytes, after_first) {
            return Some((i, lb1 + lb2));
        }
        // Ordinary line ending inside a frame; resume scanning after it.
        i = after_first;
    }
    None
}

/// Incremental SSE frame parser: feed raw UTF-8 chunks, drain complete frames.
///
/// Incomplete frames (no trailing blank line yet) stay buffered until the next
/// feed, so a frame split across TCP reads is reassembled correctly.
struct SseParser {
    buf: String,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buf: String::new(),
        }
    }

    fn feed(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
    }

    /// Take all complete frames currently in the buffer, leaving any
    /// partial trailing block buffered for the next call.
    ///
    /// Frame terminators are detected line-ending-agnostically per the
    /// WHATWG SSE spec: a blank line is two consecutive line breaks, each
    /// of which may be `\n`, `\r\n`, or `\r`. The live TerminalBench
    /// sidecar sends CRLF (`\r\n\r\n`); accepting LF, CRLF, and mixed
    /// endings keeps a CRLF-only stream from starving the parser and
    /// hanging `connect` on an endpoint frame it can never split.
    fn take_frames(&mut self) -> Vec<SseItem> {
        let mut out = Vec::new();
        while let Some((term_start, term_len)) = find_frame_terminator(&self.buf) {
            // Drain the block content plus its trailing blank-line
            // terminator so the next iteration starts at a block boundary.
            let block: String = self.buf.drain(..term_start + term_len).collect();
            if let Some(item) = parse_sse_block(&block) {
                out.push(item);
            }
        }
        out
    }
}

// ===========================================================================
// SSE reader (async byte stream -> items)
// ===========================================================================

/// Pulls SSE items out of a reqwest response byte stream, buffering partial
/// frames across chunk boundaries.
struct SseReader {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    parser: SseParser,
    pending_items: VecDeque<SseItem>,
}

impl SseReader {
    /// Return the next decoded item, or `None` when the stream has ended.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] on a stream read error or a
    /// non-UTF-8 chunk (the sidecar speaks text SSE).
    async fn next_item(&mut self) -> Result<Option<SseItem>, SidecarError> {
        loop {
            if let Some(item) = self.pending_items.pop_front() {
                return Ok(Some(item));
            }
            match self.stream.next().await {
                None => return Ok(None),
                Some(Err(e)) => {
                    return Err(SidecarError::Protocol(format!(
                        "SSE stream read error: {e}"
                    )));
                }
                Some(Ok(chunk)) => {
                    let text = std::str::from_utf8(&chunk).map_err(|e| {
                        SidecarError::Protocol(format!("SSE stream non-UTF-8 chunk: {e}"))
                    })?;
                    self.parser.feed(text);
                    self.pending_items.extend(self.parser.take_frames());
                }
            }
        }
    }
}

/// Read SSE items until the first `event: endpoint` frame and return its
/// data: the relative messages URL the sidecar advertises.
///
/// Comments and non-endpoint frames are skipped; a closed stream yields
/// [`SidecarError::MissingEndpointEvent`]. This helper has no internal
/// bound by design — [`SidecarClient::connect`] wraps it in
/// `timeout(CONNECT_TIMEOUT, …)` so a stream that stays open without ever
/// delivering the endpoint frame fails loudly instead of hanging.
async fn read_endpoint_event(reader: &mut SseReader) -> Result<String, SidecarError> {
    loop {
        match reader.next_item().await? {
            None => return Err(SidecarError::MissingEndpointEvent),
            Some(SseItem::Comment(_)) => continue,
            Some(SseItem::Frame { event, data }) if event.as_deref() == Some("endpoint") => {
                return Ok(data);
            }
            Some(_) => continue,
        }
    }
}

/// Render an item back into the on-the-wire SSE block shape (including the
/// trailing blank line) for the probe transcript.
///
/// Line endings normalize to LF: `parse_sse_block` already stripped any CR
/// via `str::lines`, so the original wire endings (LF or CRLF) are not
/// recoverable here. The transcript is the normalized frame shape, which is
/// what the F3 fixture and its round-trip tests pin.
fn format_sse_item(item: &SseItem) -> String {
    match item {
        SseItem::Comment(c) => format!(": {c}\n\n"),
        SseItem::Frame {
            event: Some(e),
            data,
        } => format!("event: {e}\ndata: {data}\n\n"),
        SseItem::Frame { event: None, data } => format!("data: {data}\n\n"),
    }
}

/// Build a JSON-RPC notification envelope: `jsonrpc: "2.0"` plus `method`,
/// with no `id` and no `params`.
///
/// Per JSON-RPC 2.0 a notification carries no `id` and elicits no response.
/// The MCP 2024-11-05 handshake requires the client to send a
/// `notifications/initialized` notification after the `initialize` result
/// before any further request; the sidecar accepts it with HTTP 202 and
/// emits no SSE frame.
fn notification_envelope(method: &str) -> JsonValue {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
    })
}

// ===========================================================================
// Shared connection state
// ===========================================================================

/// The stream state held under the tokio mutex: the live SSE reader plus a
/// buffer of response frames whose ids did not match the in-flight request
/// (out-of-order tolerance).
struct SseState {
    reader: SseReader,
    pending: Vec<(u64, JsonValue)>,
}

struct Shared {
    http: reqwest::Client,
    message_endpoint: url::Url,
    next_id: AtomicU64,
    stream: TokioMutex<SseState>,
    last_request: StdMutex<Option<String>>,
    frames: StdMutex<Vec<SseItem>>,
}

/// The JSON-boundary client for a classic-SSE MCP sidecar.
///
/// Cloning shares the connection state, so worker tools that need the sidecar
/// each take a cheap clone. The transport internals (reqwest client, SSE
/// stream handle, message endpoint URL) live behind the `Arc`.
#[derive(Clone)]
pub struct SidecarClient {
    url: SidecarUrl,
    shared: Arc<Shared>,
}

impl fmt::Debug for SidecarClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SidecarClient")
            .field("url", &self.url)
            .field("message_endpoint", &self.shared.message_endpoint)
            .finish_non_exhaustive()
    }
}

impl SidecarClient {
    /// Connect to a sidecar: open the SSE stream, read the `endpoint` event,
    /// and resolve the messages URL.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::InvalidUrl`] when the base URL does not parse,
    /// [`SidecarError::Connect`] when the SSE GET fails or the content-type is
    /// not `text/event-stream`, and [`SidecarError::MissingEndpointEvent`] when
    /// the stream closes before the `endpoint` event arrives.
    pub async fn connect(url: SidecarUrl) -> Result<Self, SidecarError> {
        let base = url::Url::parse(url.as_str())
            .map_err(|e| SidecarError::InvalidUrl(e.to_string()))?;

        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| SidecarError::Connect(format!("HTTP client build failed: {e}")))?;

        let response = http
            .get(url.as_str())
            .header(ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| SidecarError::Connect(format!("SSE GET failed: {e}")))?;
        let response = response
            .error_for_status()
            .map_err(|e| SidecarError::Connect(format!("SSE GET bad status: {e}")))?;

        let content_type_ok = response
            .headers()
            .get_all(CONTENT_TYPE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|ct| ct.starts_with("text/event-stream"));
        if !content_type_ok {
            let ct = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("(none)")
                .to_owned();
            return Err(SidecarError::Connect(format!(
                "unexpected content-type: {ct}"
            )));
        }

        let mut reader = SseReader {
            stream: Box::pin(response.bytes_stream()),
            parser: SseParser::new(),
            pending_items: VecDeque::new(),
        };

        let endpoint_data = match timeout(CONNECT_TIMEOUT, read_endpoint_event(&mut reader)).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(SidecarError::Connect(format!(
                    "timed out after {CONNECT_TIMEOUT:?} waiting for the endpoint event"
                )));
            }
        };

        let message_endpoint = base
            .join(&endpoint_data)
            .map_err(|e| SidecarError::Protocol(format!("endpoint URL resolve failed: {e}")))?;

        let frames = vec![SseItem::Frame {
            event: Some("endpoint".to_owned()),
            data: endpoint_data,
        }];
        let shared = Arc::new(Shared {
            http,
            message_endpoint,
            next_id: AtomicU64::new(1),
            stream: TokioMutex::new(SseState {
                reader,
                pending: Vec::new(),
            }),
            last_request: StdMutex::new(None),
            frames: StdMutex::new(frames),
        });
        Ok(Self { url, shared })
    }

    /// A non-functional client for test construction.
    ///
    /// The client carries a dummy URL and an empty SSE stream. Any tool
    /// that forwards through it will fail. Use this only when the sidecar
    /// tools (`keystrokes`, `capture-pane`) are not exercised, e.g. in
    /// tests with `MockProvider`-backed workers that only call
    /// `submit_result`.
    pub fn disconnected() -> Self {
        let url = SidecarUrl::new("http://localhost:0/sse").expect("valid dummy URL");
        let http = reqwest::Client::builder()
            .build()
            .expect("HTTP client builds without a network");
        let message_endpoint =
            url::Url::parse("http://localhost:0/messages").expect("valid dummy URL");
        let shared = Arc::new(Shared {
            http,
            message_endpoint,
            next_id: AtomicU64::new(1),
            stream: TokioMutex::new(SseState {
                reader: SseReader {
                    stream: Box::pin(futures::stream::empty()),
                    parser: SseParser::new(),
                    pending_items: VecDeque::new(),
                },
                pending: Vec::new(),
            }),
            last_request: StdMutex::new(None),
            frames: StdMutex::new(Vec::new()),
        });
        Self { url, shared }
    }

    /// The endpoint this client was constructed for.
    pub fn url(&self) -> &SidecarUrl {
        &self.url
    }

    /// The resolved messages URL that JSON-RPC POSTs go to.
    pub fn message_endpoint(&self) -> String {
        self.shared.message_endpoint.as_str().to_owned()
    }

    /// The verbatim JSON-RPC request body of the most recent POST, for
    /// transcript capture. Overwritten on every call.
    pub fn last_request(&self) -> Option<String> {
        self.shared.last_request.lock().unwrap().clone()
    }

    /// Drain and return all buffered SSE frames (formatted as on-the-wire
    /// blocks) since the last drain, for transcript capture.
    pub fn drain_transcript(&self) -> Vec<String> {
        self.shared
            .frames
            .lock()
            .unwrap()
            .drain(..)
            .map(|i| format_sse_item(&i))
            .collect()
    }

    /// Send `initialize`, read the server info, and complete the MCP
    /// handshake by sending the `notifications/initialized` notification.
    ///
    /// The 2024-11-05 spec requires the client to send the notification
    /// after the `initialize` result and before any further request; a
    /// sidecar that receives `tools/list` first kills the session with
    /// `Received request before initialization was complete`. Sending it
    /// here means callers cannot forget it — the client is usable the
    /// moment this returns.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] when the response is malformed or
    /// the notification POST fails.
    pub async fn initialize(&self) -> Result<SidecarServerInfo, SidecarError> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "agent-driver-prototype",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });
        let result = self.request("initialize", Some(params)).await?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SidecarError::Protocol("initialize response missing protocolVersion".to_owned())
            })?
            .to_owned();
        let server_info = result.get("serverInfo").ok_or_else(|| {
            SidecarError::Protocol("initialize response missing serverInfo".to_owned())
        })?;
        let server_name = server_info
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SidecarError::Protocol("serverInfo missing name".to_owned()))?
            .to_owned();
        let server_version = server_info
            .get("version")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SidecarError::Protocol("serverInfo missing version".to_owned()))?
            .to_owned();
        // MCP handshake: the notification goes after the initialize result
        // and before the client is usable. The sidecar accepts it with 202
        // and emits no SSE response frame, so do not wait on the stream.
        self.notify("notifications/initialized").await?;
        Ok(SidecarServerInfo {
            protocol_version,
            server_name,
            server_version,
        })
    }

    /// Send `tools/list` and read the tool definitions.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Protocol`] when the response is malformed and
    /// [`SidecarError::EmptyToolName`] when a tool entry has an empty name.
    pub async fn list_tools(&self) -> Result<Vec<SidecarTool>, SidecarError> {
        let result = self.request("tools/list", None).await?;
        let tools = result
            .get("tools")
            .ok_or_else(|| SidecarError::Protocol("tools/list response missing tools".to_owned()))?;
        let tools_arr = tools.as_array().ok_or_else(|| {
            SidecarError::Protocol("tools/list tools field not an array".to_owned())
        })?;
        let mut out = Vec::with_capacity(tools_arr.len());
        for entry in tools_arr {
            let name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SidecarError::Protocol("tool entry missing name".to_owned()))?;
            let name = SidecarToolName::new(name)?;
            let description = entry
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let input_schema = entry
                .get("inputSchema")
                .cloned()
                .unwrap_or(JsonValue::Null);
            out.push(SidecarTool::new(name, description, input_schema));
        }
        Ok(out)
    }

    /// Send `tools/call` and read the text content.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::ToolCall`] when the sidecar reports
    /// `isError: true` or returns no content, and [`SidecarError::Protocol`]
    /// when the response is malformed.
    pub async fn call_tool(
        &self,
        name: &SidecarToolName,
        args: &SidecarToolArgs,
    ) -> Result<SidecarContent, SidecarError> {
        let params = serde_json::json!({
            "name": name.as_str(),
            "arguments": args.inner(),
        });
        let result = self.request("tools/call", Some(params)).await?;
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let content = result.get("content").ok_or_else(|| {
            SidecarError::Protocol("tools/call response missing content".to_owned())
        })?;
        let content_arr = content.as_array().ok_or_else(|| {
            SidecarError::Protocol("tools/call content not an array".to_owned())
        })?;
        let mut text = String::new();
        for item in content_arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("text")
                && let Some(t) = item.get("text").and_then(|v| v.as_str())
            {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
        if is_error {
            let msg = if text.is_empty() {
                "tools/call returned isError with no text".to_owned()
            } else {
                text
            };
            return Err(SidecarError::ToolCall(msg));
        }
        if content_arr.is_empty() {
            return Err(SidecarError::ToolCall(
                "tools/call returned no content".to_owned(),
            ));
        }
        Ok(SidecarContent::new(text))
    }

    /// Send a JSON-RPC request, POST it, and read the matching response.
    /// Returns the `result` object after checking for a JSON-RPC `error`.
    async fn request(
        &self,
        method: &str,
        params: Option<JsonValue>,
    ) -> Result<JsonValue, SidecarError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::SeqCst);
        let mut envelope = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            envelope["params"] = p;
        }
        let body = serde_json::to_string(&envelope).map_err(|e| {
            SidecarError::Protocol(format!("request serialize failed: {e}"))
        })?;
        *self.shared.last_request.lock().unwrap() = Some(body.clone());

        let post = self
            .shared
            .http
            .post(self.shared.message_endpoint.as_str())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| SidecarError::Protocol(format!("POST {method} failed: {e}")))?;
        post.error_for_status()
            .map_err(|e| SidecarError::Protocol(format!("POST {method} bad status: {e}")))?;

        let value = self.read_matching_response(id).await?;
        if let Some(err) = value.get("error") {
            return Err(SidecarError::Protocol(format!(
                "JSON-RPC error for {method}: {err}"
            )));
        }
        value.get("result").cloned().ok_or_else(|| {
            SidecarError::Protocol(format!("JSON-RPC response to {method} missing result"))
        })
    }

    /// Send a JSON-RPC notification (no id, no response) to the message
    /// endpoint.
    ///
    /// Per JSON-RPC 2.0 a notification carries no `id` and elicits no
    /// response; the sidecar accepts it with HTTP 202 and emits no SSE
    /// frame, so this does not wait on the stream. It does not touch
    /// `last_request`, which tracks id-bearing requests for transcript
    /// capture — a notification is not a request.
    async fn notify(&self, method: &str) -> Result<(), SidecarError> {
        let envelope = notification_envelope(method);
        let body = serde_json::to_string(&envelope)
            .map_err(|e| SidecarError::Protocol(format!("notification serialize failed: {e}")))?;
        let post = self
            .shared
            .http
            .post(self.shared.message_endpoint.as_str())
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| SidecarError::Protocol(format!("POST notification {method} failed: {e}")))?;
        post.error_for_status().map_err(|e| {
            SidecarError::Protocol(format!("POST notification {method} bad status: {e}"))
        })?;
        Ok(())
    }

    /// Read frames from the SSE stream until the response with `id` arrives.
    /// Out-of-order responses are buffered; comments and non-message frames
    /// are logged to the transcript but skipped.
    async fn read_matching_response(&self, id: u64) -> Result<JsonValue, SidecarError> {
        let mut state = self.shared.stream.lock().await;
        if let Some(pos) = state.pending.iter().position(|(pid, _)| *pid == id) {
            let (_, value) = state.pending.swap_remove(pos);
            return Ok(value);
        }
        loop {
            let item = match timeout(RESPONSE_TIMEOUT, state.reader.next_item()).await {
                Ok(Ok(Some(item))) => item,
                Ok(Ok(None)) => {
                    return Err(SidecarError::Protocol(format!(
                        "SSE stream closed while waiting for response to id {id}"
                    )));
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(SidecarError::Protocol(format!(
                        "timed out after {RESPONSE_TIMEOUT:?} waiting for response to id {id}"
                    )));
                }
            };
            match item {
                SseItem::Comment(c) => {
                    self.push_frame(SseItem::Comment(c));
                }
                SseItem::Frame { event, data } => {
                    let is_message =
                        event.as_deref() == Some("message") || event.is_none();
                    if !is_message {
                        self.push_frame(SseItem::Frame { event, data });
                        continue;
                    }
                    let value: JsonValue = serde_json::from_str(&data).map_err(|e| {
                        SidecarError::Protocol(format!("JSON-RPC response parse failed: {e}"))
                    })?;
                    let rid = value.get("id").and_then(|x| x.as_u64()).ok_or_else(|| {
                        SidecarError::Protocol(
                            "JSON-RPC response missing numeric id".to_owned(),
                        )
                    })?;
                    self.push_frame(SseItem::Frame {
                        event: event.clone(),
                        data: data.clone(),
                    });
                    if rid == id {
                        return Ok(value);
                    }
                    state.pending.push((rid, value));
                }
            }
        }
    }

    /// Append a raw item to the transcript buffer (probe diagnostic channel).
    fn push_frame(&self, item: SseItem) {
        self.shared.frames.lock().unwrap().push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SidecarUrl -------------------------------------------------------

    #[test]
    fn sidecar_url_rejects_empty_and_whitespace() {
        assert!(SidecarUrl::new("").is_err());
        assert!(SidecarUrl::new("   ").is_err());
        assert!(SidecarUrl::new("\t\n").is_err());
    }

    #[test]
    fn sidecar_url_rejects_non_http_scheme() {
        assert!(SidecarUrl::new("ftp://localhost/sse").is_err());
        assert!(SidecarUrl::new("localhost:8000/sse").is_err());
        assert!(SidecarUrl::new("ws://localhost/sse").is_err());
    }

    #[test]
    fn sidecar_url_accepts_http_and_https_and_trims() {
        let http = SidecarUrl::new("http://localhost:8000/sse").unwrap();
        assert_eq!(http.as_str(), "http://localhost:8000/sse");
        let https = SidecarUrl::new("https://example.com/api/mcp/sse").unwrap();
        assert_eq!(https.as_str(), "https://example.com/api/mcp/sse");
        let trimmed = SidecarUrl::new("  http://localhost:8000/sse  ").unwrap();
        assert_eq!(trimmed.as_str(), "http://localhost:8000/sse");
    }

    // --- SSE field splitting ----------------------------------------------

    #[test]
    fn split_sse_field_strips_one_leading_space() {
        assert_eq!(split_sse_field("data: hello"), ("data", "hello"));
        assert_eq!(split_sse_field("data:hello"), ("data", "hello"));
        assert_eq!(split_sse_field("data:  two spaces"), ("data", " two spaces"));
        assert_eq!(split_sse_field("data:"), ("data", ""));
        assert_eq!(split_sse_field(": ping - x"), ("", "ping - x"));
    }

    // --- SSE block parsing ------------------------------------------------

    #[test]
    fn parse_endpoint_block_yields_frame() {
        let block = "event: endpoint\ndata: /messages/?session_id=abc\n\n";
        let item = parse_sse_block(block).unwrap();
        match item {
            SseItem::Frame {
                event,
                data,
            } => {
                assert_eq!(event.as_deref(), Some("endpoint"));
                assert_eq!(data, "/messages/?session_id=abc");
            }
            SseItem::Comment(_) => panic!("expected frame"),
        }
    }

    #[test]
    fn parse_comment_block_yields_comment() {
        let block = ": ping - 2026-07-26 03:10:14.816383+00:00\n\n";
        let item = parse_sse_block(block).unwrap();
        assert_eq!(item, SseItem::Comment("ping - 2026-07-26 03:10:14.816383+00:00".to_owned()));
    }

    #[test]
    fn parse_multi_data_line_block_joins_with_newline() {
        let block = "event: message\ndata: line one\ndata: line two\n\n";
        let item = parse_sse_block(block).unwrap();
        match item {
            SseItem::Frame { event, data } => {
                assert_eq!(event.as_deref(), Some("message"));
                assert_eq!(data, "line one\nline two");
            }
            SseItem::Comment(_) => panic!("expected frame"),
        }
    }

    #[test]
    fn parse_block_without_event_defaults_to_no_event_field() {
        // SSE spec: a frame with data and no `event:` dispatches as the
        // default `message` event; the parser records `event: None` and the
        // reader treats `None` as `message`.
        let block = "data: {\"hi\":1}\n\n";
        let item = parse_sse_block(block).unwrap();
        match item {
            SseItem::Frame { event, data } => {
                assert!(event.is_none());
                assert_eq!(data, "{\"hi\":1}");
            }
            SseItem::Comment(_) => panic!("expected frame"),
        }
    }

    // --- Incremental framed parsing ---------------------------------------

    #[test]
    fn parser_buffers_partial_frame_until_terminator() {
        let mut parser = SseParser::new();
        parser.feed("event: message\ndata: not-yet-complete");
        assert!(parser.take_frames().is_empty(), "no terminator yet");
        parser.feed("\n\nevent: endpoint\ndata: /m/?s=1\n\n");
        let items = parser.take_frames();
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], SseItem::Frame { event, .. } if event.as_deref() == Some("message")));
        assert!(matches!(&items[1], SseItem::Frame { event, .. } if event.as_deref() == Some("endpoint")));
    }

    // --- The frozen F3 transcript -----------------------------------------
    //
    // The board owner diffs the live probe transcript against this exact
    // capture. Feeding it through the parser proves the frame boundary logic
    // matches the real sidecar wire shape byte-for-byte at the frame level.

    const F3_TRANSCRIPT: &str = r##"event: endpoint
data: /messages/?session_id=cde45def1da348018c0e2dcb74d3f8ca

event: message
data: {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"experimental":{},"tools":{"listChanged":false}},"serverInfo":{"name":"t-bench","version":"1.6.0"}}}

event: message
data: {"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"keystrokes","description":"Send keystrokes to a tmux session.","inputSchema":{"properties":{"keystrokes":{"description":"Keystrokes to execute in the terminal. Use tmux-style escape sequences for special characters (e.g. C-c for ctrl-c).","title":"Keystrokes","type":"string"},"wait_time_sec":{"default":0.0,"description":"The number of expected seconds to wait for the command to complete.","title":"Wait Time Sec","type":"number"},"append_enter":{"default":false,"description":"Whether to append a newline character to the end of the keystrokes. (This is necessary to execute bash commands.)","title":"Append Enter","type":"boolean"}},"required":["keystrokes"],"title":"Command","type":"object"}},{"name":"capture-pane","description":"Capture the pane of a tmux session.","inputSchema":{"properties":{"wait_before_capture_sec":{"default":0.0,"description":"The number of seconds to wait before capturing the pane. This is useful if you just executed a command and want to wait a bit to capture the output.","title":"Wait Before Capture Sec","type":"number"}},"title":"CapturePaneSchema","type":"object"}}]}}

: ping - 2026-07-26 03:10:14.816383+00:00

event: message
data: {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"111e30bd0529:/# echo S54_PROBE_OK_$((40+2))\nS54_PROBE_OK_42\n111e30bd0529:/#\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n"}],"isError":false}}

event: message
data: {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"111e30bd0529:/# echo S54_PROBE_OK_$((40+2))\nS54_PROBE_OK_42\n111e30bd0529:/#\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n"}],"isError":false}}

: ping - 2026-07-26 03:10:29.823135+00:00

: ping - 2026-07-26 03:10:44.830723+00:00

: ping - 2026-07-26 03:10:59.833692+00:00

"##;

    #[test]
    fn parser_frames_the_f3_transcript_in_one_feed() {
        let mut parser = SseParser::new();
        parser.feed(F3_TRANSCRIPT);
        let items = parser.take_frames();
        assert_eq!(items.len(), 9, "endpoint + 4 messages + 4 pings");

        assert!(matches!(&items[0], SseItem::Frame { event, data }
            if event.as_deref() == Some("endpoint")
            && data.starts_with("/messages/?session_id=")));

        let message_ids = [1u64, 2, 3, 4];
        let mut msg_idx = 0;
        for item in &items {
            if let SseItem::Frame { event, data } = item
                && event.as_deref() == Some("message")
            {
                let v: JsonValue = serde_json::from_str(data).unwrap();
                let id = v["id"].as_u64().unwrap();
                assert_eq!(id, message_ids[msg_idx], "messages must arrive in id order");
                msg_idx += 1;
            }
        }
        assert_eq!(msg_idx, 4);

        let ping_count = items
            .iter()
            .filter(|i| matches!(i, SseItem::Comment(c) if c.starts_with("ping -")))
            .count();
        assert_eq!(ping_count, 4);
        // No partial frame should remain buffered.
        assert!(parser.take_frames().is_empty());
    }

    #[test]
    fn parser_frames_the_f3_transcript_across_arbitrary_chunk_boundary() {
        let mid = F3_TRANSCRIPT.len() / 2 + 7;
        let mut parser = SseParser::new();
        parser.feed(&F3_TRANSCRIPT[..mid]);
        let first = parser.take_frames();
        parser.feed(&F3_TRANSCRIPT[mid..]);
        let second = parser.take_frames();
        let items: Vec<SseItem> = first.into_iter().chain(second).collect();
        assert_eq!(items.len(), 9, "splitting the feed must not lose or duplicate frames");
        assert!(parser.take_frames().is_empty());
    }

    #[test]
    fn format_sse_item_round_trips_f3_endpoint_block() {
        let item = SseItem::Frame {
            event: Some("endpoint".to_owned()),
            data: "/messages/?session_id=cde45def1da348018c0e2dcb74d3f8ca".to_owned(),
        };
        assert_eq!(
            format_sse_item(&item),
            "event: endpoint\ndata: /messages/?session_id=cde45def1da348018c0e2dcb74d3f8ca\n\n"
        );
    }

    #[test]
    fn format_sse_item_round_trips_comment_block() {
        let item = SseItem::Comment("ping - x".to_owned());
        assert_eq!(format_sse_item(&item), ": ping - x\n\n");
    }

    // --- CRLF wire endings (the live TerminalBench sidecar shape) ---------
    //
    // The F3 transcript file carries CRLF line terminators. A terminator
    // scan that only looks for `\n\n` never sees a frame boundary in a
    // `\r\n\r\n` stream, so `take_frames` returns nothing forever and
    // `connect` hangs on the endpoint event. These tests pin the
    // line-ending-agnostic splitter so that regression cannot return.

    #[test]
    fn parser_handles_crlf_framed_stream_like_f3_transcript() {
        // The first three F3 items — endpoint frame, message frame,
        // `: ping` comment — framed with CRLF, exactly the wire shape the
        // live sidecar emits.
        let crlf = concat!(
            "event: endpoint\r\n",
            "data: /messages/?session_id=cde45def1da348018c0e2dcb74d3f8ca\r\n",
            "\r\n",
            "event: message\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"serverInfo\":{\"name\":\"t-bench\",\"version\":\"1.6.0\"}}}\r\n",
            "\r\n",
            ": ping - 2026-07-26 03:10:14.816383+00:00\r\n",
            "\r\n",
        );

        let mut parser = SseParser::new();
        parser.feed(crlf);
        let items = parser.take_frames();
        assert_eq!(items.len(), 3, "endpoint frame + message frame + ping comment");

        assert!(matches!(&items[0], SseItem::Frame { event, data }
            if event.as_deref() == Some("endpoint")
            && data == "/messages/?session_id=cde45def1da348018c0e2dcb74d3f8ca"));

        match &items[1] {
            SseItem::Frame { event, data } => {
                assert_eq!(event.as_deref(), Some("message"));
                let v: JsonValue = serde_json::from_str(data).unwrap();
                assert_eq!(v["id"].as_u64(), Some(1));
                assert_eq!(v["result"]["protocolVersion"].as_str(), Some("2024-11-05"));
                assert_eq!(v["result"]["serverInfo"]["name"].as_str(), Some("t-bench"));
            }
            SseItem::Comment(_) => panic!("expected message frame, got comment"),
        }

        assert_eq!(
            items[2],
            SseItem::Comment("ping - 2026-07-26 03:10:14.816383+00:00".to_owned())
        );

        // No partial frame should remain buffered.
        assert!(parser.take_frames().is_empty());
    }

    #[test]
    fn parser_handles_crlf_framed_stream_across_chunk_boundary() {
        // A CRLF terminator split across two feeds (the `\r\n\r\n` broken
        // between the field line's `\r\n` and the blank line's `\r\n`) must
        // reassemble into whole frames, not lose or duplicate them.
        let crlf = concat!(
            "event: endpoint\r\n",
            "data: /messages/?session_id=abc\r\n",
            "\r\n",
            "event: message\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":1}\r\n",
            "\r\n",
        );
        // Split inside the first frame's terminator: after the field line's
        // `\r\n`, before the blank line's `\r\n`. ASCII-only, so any byte
        // split is a UTF-8 boundary.
        let split = crlf.find("\r\n\r\n").map(|p| p + 2).unwrap();

        let mut parser = SseParser::new();
        parser.feed(&crlf[..split]);
        let first = parser.take_frames();
        parser.feed(&crlf[split..]);
        let second = parser.take_frames();
        let items: Vec<SseItem> = first.into_iter().chain(second).collect();

        assert_eq!(
            items.len(),
            2,
            "splitting the CRLF feed must not lose or duplicate frames"
        );
        assert!(matches!(&items[0], SseItem::Frame { event, .. } if event.as_deref() == Some("endpoint")));
        assert!(matches!(&items[1], SseItem::Frame { event, .. } if event.as_deref() == Some("message")));
        assert!(parser.take_frames().is_empty());
    }

    #[tokio::test]
    async fn read_endpoint_event_hangs_without_caller_timeout() {
        // Reproduce the CRLF-hang symptom at the reader level: the GET
        // succeeded and the stream is open, but the sidecar emitted only a
        // keep-alive comment and then went quiet — no endpoint frame ever
        // arrives. `read_endpoint_event` has no internal bound by design
        // (the connect path supplies `CONNECT_TIMEOUT`); unbounded it would
        // hang forever. A short caller timeout must fire so the regression
        // fails loud instead of hanging.
        let mock = futures::stream::iter(vec![
            Ok::<Bytes, reqwest::Error>(Bytes::from_static(b": ping\r\n\r\n")),
        ])
        .chain(futures::stream::pending::<Result<Bytes, reqwest::Error>>());

        let mut reader = SseReader {
            stream: Box::pin(mock),
            parser: SseParser::new(),
            pending_items: VecDeque::new(),
        };

        let waited = timeout(Duration::from_millis(200), read_endpoint_event(&mut reader)).await;
        assert!(
            waited.is_err(),
            "endpoint wait must be bounded by the caller's timeout, not hang silently"
        );
    }

    // --- MCP handshake notification --------------------------------------
    //
    // The 2024-11-05 spec requires a `notifications/initialized`
    // notification after the initialize result before any further request.
    // A JSON-RPC notification has no `id` and elicits no response; the
    // sidecar kills the session (`Received request before initialization
    // was complete`) if `tools/list` arrives first. This test pins the
    // envelope shape so a regression that adds an id or params is caught.

    #[test]
    fn initialized_notification_envelope_has_no_id_and_no_params() {
        let env = notification_envelope("notifications/initialized");
        assert_eq!(env["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(env["method"].as_str(), Some("notifications/initialized"));
        // A notification carries no id and elicits no response.
        assert!(
            env.get("id").is_none(),
            "notification must not carry an id"
        );
        assert!(
            env.get("params").is_none(),
            "initialized notification carries no params"
        );
        // Exactly the two required fields.
        let obj = env.as_object().unwrap();
        assert_eq!(obj.len(), 2, "envelope must be exactly jsonrpc + method");
    }
}
