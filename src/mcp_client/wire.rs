//! Wire-shape types for the classic-SSE MCP protocol.
//!
//! Every type models what the F3 transcript captured: the `initialize`
//! result, the `tools/list` tool entries, and the `tools/call` content
//! payload. No rmcp type appears here — the JSON-RPC envelope is handled
//! inside [`super::client::SidecarClient`], and only plain JSON reaches the
//! public boundary.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};

use super::client::SidecarError;

/// A tool name the sidecar exposes.
///
/// The sidecar advertises exactly two tools — `keystrokes` and
/// `capture-pane` — but the type is not restricted to those two so a
/// future sidecar that adds a third needs no type change here.
///
/// Forbidden invalid state: an empty tool name sent to the sidecar in a
/// `tools/call` request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SidecarToolName(String);

impl SidecarToolName {
    /// Parse a tool name.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::EmptyToolName`] when `name` is empty or
    /// whitespace-only.
    pub fn new(name: &str) -> Result<Self, SidecarError> {
        if name.trim().is_empty() {
            return Err(SidecarError::EmptyToolName);
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SidecarToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Arguments for a `tools/call` request.
///
/// The MCP protocol requires `arguments` to be a JSON object; a bare value
/// or array is a protocol error. The newtype makes that rule structural.
///
/// Forbidden invalid state: a non-object `arguments` value reaching the
/// POST body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarToolArgs(Map<String, JsonValue>);

impl SidecarToolArgs {
    /// Parse a JSON value into tool-call arguments.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::ArgumentsNotObject`] when `value` is not a
    /// JSON object.
    pub fn from_value(value: JsonValue) -> Result<Self, SidecarError> {
        #[allow(
            clippy::wildcard_enum_match_arm,
            reason = "JsonValue is an external enum; the wildcard is the error case"
        )]
        match value {
            JsonValue::Object(map) => Ok(Self(map)),
            _ => Err(SidecarError::ArgumentsNotObject),
        }
    }

    /// Build arguments from an already-typed map.
    pub fn from_map(map: Map<String, JsonValue>) -> Self {
        Self(map)
    }

    /// The argument object as it appears on the wire.
    pub fn inner(&self) -> &Map<String, JsonValue> {
        &self.0
    }
}

/// Text content a `tools/call` returns.
///
/// The sidecar wraps its result in `{"content": [{"type": "text", "text":
/// …}]}`; this type is the unwrapped text. It may be empty — the sidecar
/// can return an empty pane — so no non-empty constraint is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarContent(String);

impl SidecarContent {
    /// Wrap the text the sidecar returned.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The content text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One entry from a `tools/list` response.
///
/// Forbidden invalid state: a tool entry with no name; the constructor
/// rejects that, and downstream code handles only valid names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarTool {
    name: SidecarToolName,
    description: String,
    input_schema: JsonValue,
}

impl SidecarTool {
    /// Parse a `tools/list` entry.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::EmptyToolName`] when the entry's `name` is
    /// empty.
    pub fn new(name: SidecarToolName, description: String, input_schema: JsonValue) -> Self {
        Self {
            name,
            description,
            input_schema,
        }
    }

    /// The tool's name.
    pub fn name(&self) -> &SidecarToolName {
        &self.name
    }

    /// The tool's description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The tool's input JSON schema.
    pub fn input_schema(&self) -> &JsonValue {
        &self.input_schema
    }
}

/// Server info from an `initialize` response.
///
/// Forbidden invalid state: none — this is an output type that reports what
/// the server sent; the caller trusts the boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarServerInfo {
    pub protocol_version: String,
    pub server_name: String,
    pub server_version: String,
}
