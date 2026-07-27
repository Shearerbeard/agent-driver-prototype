//! JSON-boundary client for the TerminalBench classic-SSE sidecar.
//!
//! The sidecar speaks the pre-2025-11-05 MCP SSE protocol (GET `/sse` for the
//! event stream, POST `/messages/?session_id=…` for requests). The rmcp
//! 0.12-vs-1.7 type-version difference that prevents `agent-driver-rs` from
//! speaking classic SSE is confined behind this module: the public surface is
//! plain JSON types — tool name plus args in, text content out — so no rmcp
//! type ever crosses the seam.
//!
//! Phase 1 declares the types; the SSE transport body lands in Phase 2.

mod client;
mod wire;

pub use client::{SidecarClient, SidecarError, SidecarUrl};
pub use wire::{SidecarContent, SidecarServerInfo, SidecarTool, SidecarToolArgs, SidecarToolName};
