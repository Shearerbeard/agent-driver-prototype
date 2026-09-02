//! JSON-boundary client for MCP servers, rmcp streamable HTTP underneath.
//!
//! rmcp carries the JSON-RPC envelope, the initialize handshake, and the
//! streamable-HTTP session protocol; this module confines it behind a
//! plain-JSON surface — tool name plus args in, text content out — so no
//! rmcp type ever crosses the seam and the worker path is insulated from
//! the SDK. The legacy classic-SSE transport this module used to implement
//! by hand (GET `/sse`, `event: endpoint`, session POSTs) was removed from
//! rmcp in 0.11.0 and has no replacement here; streamable HTTP is the one
//! client transport.
//!
//! The wire-shape types live in [`wire`]; the client in [`client`].

mod client;
mod wire;

pub use client::{SidecarClient, SidecarError, SidecarUrl};
pub use wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};
