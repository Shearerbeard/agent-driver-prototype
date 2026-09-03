//! The legacy MCP HTTP+SSE transport (the 2024-11-05 protocol).
//!
//! The wire protocol this pipe speaks:
//!
//! 1. `GET` the SSE endpoint with `Accept: text/event-stream`
//! 2. The server opens the stream with an `event: endpoint` frame naming
//!    the message URL, resolved against the SSE base per RFC 3986
//! 3. Client JSON-RPC messages are `POST`ed to that URL
//! 4. Server messages arrive as `event: message` frames on the stream
//!
//! [`connect`](Self::connect) consumes the discovery frame before the
//! transport is handed to `serve_client`, so the handshake's first
//! `receive` sees an actual message, not the endpoint event. Discovery
//! is bounded: a server that never delivers the endpoint frame fails
//! inside the connect timeout instead of parking the caller.

use std::future::Future;
use std::pin::Pin;

use futures::{Stream, StreamExt};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::Transport;
use sse_stream::{Error as SseError, Sse, SseStream};
use tokio::time::timeout;
use tracing::{debug, warn};

use super::client::CONNECT_TIMEOUT;

/// SSE event type carrying a JSON-RPC message from server to client.
const SSE_EVENT_MESSAGE: &str = "message";
/// SSE event type carrying endpoint discovery, the first frame on the
/// stream.
const SSE_EVENT_ENDPOINT: &str = "endpoint";

/// The boxed SSE frame stream the transport parks between `receive` calls.
type SseFrameStream = Pin<Box<dyn Stream<Item = Result<Sse, SseError>> + Send>>;

/// Why an SSE transport cannot be established or driven.
///
/// Self-contained by design: the transport is a leaf that must not lean on
/// the client's error surface, only feed it.
#[derive(Debug, thiserror::Error)]
pub(super) enum SseTransportError {
    /// An HTTP request or response failed.
    #[error("SSE HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The SSE base URL or the endpoint frame's URL did not parse.
    #[error("SSE URL parse failed: {0}")]
    UrlParse(#[from] url::ParseError),
    /// The SSE response carried no Content-Type at all.
    #[error("SSE response has no Content-Type header")]
    MissingContentType,
    /// The SSE response's Content-Type was not `text/event-stream`.
    #[error("unexpected Content-Type for SSE response: {0}")]
    UnexpectedContentType(String),
    /// The byte stream failed SSE framing while draining to the endpoint
    /// event.
    #[error("SSE stream error: {0}")]
    SseStream(#[from] SseError),
    /// The stream ended before the `event: endpoint` discovery frame.
    #[error("SSE stream closed before the endpoint event")]
    MissingEndpointEvent,
    /// The opening `GET` or the endpoint-frame wait did not complete
    /// inside the connect bound.
    #[error("SSE endpoint discovery timed out after {0:?}")]
    ConnectTimeout(std::time::Duration),
}

/// MCP client transport over the legacy SSE protocol.
///
/// After [`connect`](Self::connect) the transport holds a live SSE stream
/// and the resolved message endpoint; `close` drops the stream, which is
/// the session teardown signal `receive` reports as `None`.
pub(super) struct SseTransport {
    http_client: reqwest::Client,
    message_endpoint: url::Url,
    stream: Option<SseFrameStream>,
}

impl Transport<RoleClient> for SseTransport {
    type Error = SseTransportError;

    /// POST one JSON-RPC message to the resolved endpoint.
    ///
    /// The future must be `Send + 'static` because rmcp may drive sends
    /// concurrently with `receive`; the client and endpoint are cloned
    /// into the future so no borrow of the transport crosses the await.
    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let client = self.http_client.clone();
        let endpoint = self.message_endpoint.clone();
        async move {
            let response = client
                .post(endpoint.as_str())
                .json(&item)
                .send()
                .await
                .map_err(SseTransportError::Http)?;
            response
                .error_for_status()
                .map_err(SseTransportError::Http)?;
            Ok(())
        }
    }

    /// Read the next JSON-RPC message off the SSE stream.
    ///
    /// Only `event: message` frames carry protocol traffic; other frames
    /// (keep-alive comments, retry hints) are skipped. A frame error or
    /// the server closing the stream ends the session: `None` is the
    /// clean-teardown signal rmcp expects.
    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        let stream = self.stream.as_mut()?;
        loop {
            let sse = match stream.next().await {
                Some(Ok(sse)) => sse,
                Some(Err(error)) => {
                    warn!(%error, "SSE stream error; closing transport");
                    return None;
                }
                None => {
                    debug!("SSE stream closed by server");
                    return None;
                }
            };
            if let (Some(SSE_EVENT_MESSAGE), Some(data)) = (sse.event.as_deref(), sse.data) {
                match serde_json::from_str(&data) {
                    Ok(message) => return Some(message),
                    Err(error) => {
                        warn!(%error, "discarding undecodable SSE message frame")
                    }
                }
            }
        }
    }

    /// Drop the SSE stream. Idempotent: an already-closed transport is a
    /// no-op.
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.stream.take();
        Ok(())
    }
}

impl SseTransport {
    /// Open the SSE connection and resolve the message endpoint.
    ///
    /// The headers arrive already validated, typed for reqwest's
    /// `default_headers`: they ride every request of the session — the
    /// opening `GET` and every message `POST` alike.
    ///
    /// The `event: endpoint` discovery frame is consumed here, before the
    /// transport is handed to `serve_client`, so the handshake's first
    /// response wait starts on a stream already positioned past it. The
    /// opening `GET` and the discovery drain are bounded by
    /// `CONNECT_TIMEOUT`.
    ///
    /// # Errors
    ///
    /// Returns the connect-failure variants for a refused, non-SSE, or
    /// endpoint-less response, and
    /// [`SseTransportError::ConnectTimeout`] when discovery does not
    /// complete inside the bound.
    pub(super) async fn connect(url: &str, headers: HeaderMap) -> Result<Self, SseTransportError> {
        let sse_endpoint = url::Url::parse(url)?;

        let http_client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(SseTransportError::Http)?;

        let (stream, message_endpoint) = timeout(CONNECT_TIMEOUT, async {
            let response = http_client
                .get(url)
                .header(ACCEPT, "text/event-stream")
                .send()
                .await
                .map_err(SseTransportError::Http)?;
            let response = response
                .error_for_status()
                .map_err(SseTransportError::Http)?;

            let content_types: Vec<&str> = response
                .headers()
                .get_all(CONTENT_TYPE)
                .into_iter()
                .filter_map(|value| value.to_str().ok())
                .collect();
            if !content_types
                .iter()
                .any(|content_type| content_type.starts_with("text/event-stream"))
            {
                return Err(match content_types.first() {
                    Some(content_type) => {
                        SseTransportError::UnexpectedContentType((*content_type).to_owned())
                    }
                    None => SseTransportError::MissingContentType,
                });
            }

            let mut stream = SseStream::from_bytes_stream(response.bytes_stream()).boxed();

            let message_endpoint = loop {
                let sse = stream
                    .next()
                    .await
                    .transpose()
                    .map_err(SseTransportError::SseStream)?
                    .ok_or(SseTransportError::MissingEndpointEvent)?;
                if let (Some(SSE_EVENT_ENDPOINT), Some(endpoint)) = (sse.event.as_deref(), sse.data)
                {
                    break resolve_message_endpoint(sse_endpoint, endpoint)?;
                }
            };

            Ok((stream, message_endpoint))
        })
        .await
        .map_err(|_| SseTransportError::ConnectTimeout(CONNECT_TIMEOUT))??;

        Ok(Self {
            http_client,
            message_endpoint,
            stream: Some(stream),
        })
    }
}

/// Resolve the endpoint frame's URL against the SSE base per RFC 3986.
///
/// The protocol allows a bare query (`?sessionId=…`), a relative or
/// absolute path, or a full URL; `Url::join` implements all four.
fn resolve_message_endpoint(
    base: url::Url,
    endpoint: String,
) -> Result<url::Url, SseTransportError> {
    base.join(&endpoint).map_err(SseTransportError::UrlParse)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- endpoint resolution -----------------------------------------------

    #[test]
    fn endpoint_resolution_supports_the_bare_query_form() {
        let base = url::Url::parse("https://localhost/sse").unwrap();
        let result = resolve_message_endpoint(base, "?sessionId=x".to_owned()).unwrap();
        assert_eq!(result.as_str(), "https://localhost/sse?sessionId=x");
    }

    #[test]
    fn endpoint_resolution_supports_relative_paths() {
        let base = url::Url::parse("https://localhost/sse").unwrap();
        let result = resolve_message_endpoint(base, "mypath?sessionId=x".to_owned()).unwrap();
        assert_eq!(result.as_str(), "https://localhost/mypath?sessionId=x");
    }

    #[test]
    fn endpoint_resolution_supports_absolute_paths() {
        let base = url::Url::parse("https://localhost/sse").unwrap();
        let result = resolve_message_endpoint(base, "/xxx?sessionId=x".to_owned()).unwrap();
        assert_eq!(result.as_str(), "https://localhost/xxx?sessionId=x");
    }

    #[test]
    fn endpoint_resolution_supports_full_urls() {
        let base = url::Url::parse("https://localhost/sse").unwrap();
        let result =
            resolve_message_endpoint(base, "http://example.com/xxx?sessionId=x".to_owned())
                .unwrap();
        assert_eq!(result.as_str(), "http://example.com/xxx?sessionId=x");
    }

    #[test]
    fn endpoint_resolution_resolves_against_the_sse_directory() {
        let base = url::Url::parse("https://example.com/api/mcp/sse").unwrap();
        let result = resolve_message_endpoint(base, "messages?sessionId=x".to_owned()).unwrap();
        assert_eq!(
            result.as_str(),
            "https://example.com/api/mcp/messages?sessionId=x"
        );
    }

    #[test]
    fn endpoint_resolution_appends_a_bare_query_to_the_full_sse_url() {
        let base = url::Url::parse("https://example.com/api/mcp/sse").unwrap();
        let result = resolve_message_endpoint(base, "?sessionId=x".to_owned()).unwrap();
        assert_eq!(
            result.as_str(),
            "https://example.com/api/mcp/sse?sessionId=x"
        );
    }

    // --- connect bound ------------------------------------------------------

    /// A listener that completes the TCP handshake and then never answers
    /// cannot park discovery forever: the connect is bounded, and expiry
    /// reports as [`SseTransportError::ConnectTimeout`]. Paused time
    /// makes the 30-second bound elapse at once.
    #[tokio::test(start_paused = true)]
    async fn a_silent_server_fails_endpoint_discovery_inside_the_bound() {
        let blackhole = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the blackhole listener binds");
        let port = blackhole
            .local_addr()
            .expect("the blackhole listener has an address")
            .port();
        tokio::spawn(async move {
            // Held, not served: the sockets stay open and say nothing.
            let mut held = Vec::new();
            while let Ok((connection, _)) = blackhole.accept().await {
                held.push(connection);
            }
        });

        let error =
            SseTransport::connect(&format!("http://127.0.0.1:{port}/sse"), HeaderMap::new())
                .await
                .err()
                .expect("a silent server must fail, not hang");

        assert!(
            matches!(error, SseTransportError::ConnectTimeout(_)),
            "discovery expiry maps to ConnectTimeout, got: {error}"
        );
    }
}
