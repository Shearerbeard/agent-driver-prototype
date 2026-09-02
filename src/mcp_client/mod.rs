//! JSON-boundary client for MCP servers, rmcp underneath.
//!
//! rmcp carries the JSON-RPC envelope and the initialize handshake; this
//! module confines it behind a plain-JSON surface — tool name plus args
//! in, text content out — so no rmcp type ever crosses the seam and the
//! worker path is insulated from the SDK. Two client transports ride that
//! boundary: rmcp's streamable HTTP, and the legacy (2024-11-05) HTTP+SSE
//! protocol behind a custom `Transport<RoleClient>` impl in
//! [`sse_transport`] — the extension seam the `Transport` trait designs
//! for, with rmcp still owning every envelope concern.
//!
//! The wire-shape types live in [`wire`]; the client in [`client`].

mod client;
mod sse_transport;
mod wire;

pub use client::{SidecarClient, SidecarError, SidecarUrl};
pub use wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};
