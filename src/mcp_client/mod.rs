//! JSON-boundary client for MCP servers, rmcp underneath.
//!
//! rmcp owns the JSON-RPC envelope, the initialize handshake, and the
//! served client session; only the byte transports live here — rmcp's
//! streamable HTTP, plus the legacy (2024-11-05) HTTP+SSE protocol
//! behind a custom `Transport<RoleClient>` impl in [`sse_transport`].
//! The wire-shape types live in [`wire`]; the client in [`client`].

mod client;
mod sse_transport;
mod wire;

pub use client::{SidecarClient, SidecarError, SidecarUrl};
pub use wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};
