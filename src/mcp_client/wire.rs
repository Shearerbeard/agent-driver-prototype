//! Wire-shape types for the MCP tool protocol.
//!
//! Every type models what the wire carries: the `initialize` result, the
//! `tools/list` tool entries, and the `tools/call` content payload. No rmcp
//! type appears here — the transport is handled inside
//! [`super::client::SidecarClient`], and only plain JSON reaches the
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
    /// Infallible because [`SidecarToolName`] already rejected the empty-name
    /// case at its own constructor; nothing this constructor does can fail.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- SidecarToolName --------------------------------------------------

    #[test]
    fn tool_name_accepts_non_empty_and_round_trips() {
        let name = SidecarToolName::new("keystrokes").unwrap();
        assert_eq!(name.as_str(), "keystrokes");
        assert_eq!(name.to_string(), "keystrokes");
    }

    #[test]
    fn tool_name_rejects_empty_or_whitespace() {
        assert_eq!(
            SidecarToolName::new("").unwrap_err(),
            SidecarError::EmptyToolName
        );
        assert_eq!(
            SidecarToolName::new("   ").unwrap_err(),
            SidecarError::EmptyToolName
        );
        assert_eq!(
            SidecarToolName::new("\t\n").unwrap_err(),
            SidecarError::EmptyToolName
        );
    }

    #[test]
    fn tool_name_is_hashable_for_roster_keys() {
        let a = SidecarToolName::new("capture-pane").unwrap();
        let b = SidecarToolName::new("capture-pane").unwrap();
        let mut set = std::collections::HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    // --- SidecarToolArgs --------------------------------------------------

    #[test]
    fn tool_args_from_value_accepts_object() {
        let args =
            SidecarToolArgs::from_value(json!({"keystrokes": "ls", "append_enter": true})).unwrap();
        assert_eq!(args.inner().len(), 2);
        assert_eq!(args.inner()["keystrokes"].as_str().unwrap(), "ls");
        assert!(args.inner()["append_enter"].as_bool().unwrap());
    }

    #[test]
    fn tool_args_from_value_rejects_non_object() {
        assert_eq!(
            SidecarToolArgs::from_value(json!(["a", "b"])).unwrap_err(),
            SidecarError::ArgumentsNotObject
        );
        assert_eq!(
            SidecarToolArgs::from_value(json!("keystrokes")).unwrap_err(),
            SidecarError::ArgumentsNotObject
        );
        assert_eq!(
            SidecarToolArgs::from_value(json!(42)).unwrap_err(),
            SidecarError::ArgumentsNotObject
        );
        assert_eq!(
            SidecarToolArgs::from_value(json!(null)).unwrap_err(),
            SidecarError::ArgumentsNotObject
        );
        assert_eq!(
            SidecarToolArgs::from_value(json!(true)).unwrap_err(),
            SidecarError::ArgumentsNotObject
        );
    }

    #[test]
    fn tool_args_from_map_preserves_entries() {
        let mut map = Map::new();
        map.insert("k".to_owned(), json!(1));
        let args = SidecarToolArgs::from_map(map);
        assert_eq!(args.inner()["k"].as_u64().unwrap(), 1);
    }

    // --- SidecarContent ---------------------------------------------------

    #[test]
    fn content_holds_text_including_empty() {
        assert_eq!(SidecarContent::new("hello".to_owned()).as_str(), "hello");
        // An empty pane is valid sidecar output, so empty text is allowed.
        assert_eq!(SidecarContent::new(String::new()).as_str(), "");
    }

    // --- SidecarTool ------------------------------------------------------

    #[test]
    fn tool_accessors_round_trip_constructor_inputs() {
        let name = SidecarToolName::new("capture-pane").unwrap();
        let schema = json!({"type": "object"});
        let tool = SidecarTool::new(name, "Capture the pane.".to_owned(), schema.clone());
        assert_eq!(tool.name().as_str(), "capture-pane");
        assert_eq!(tool.description(), "Capture the pane.");
        assert_eq!(tool.input_schema(), &schema);
    }

    // --- SidecarServerInfo ------------------------------------------------

    #[test]
    fn server_info_serde_round_trip_preserves_fields() {
        let info = SidecarServerInfo {
            protocol_version: "2024-11-05".to_owned(),
            server_name: "t-bench".to_owned(),
            server_version: "1.6.0".to_owned(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: SidecarServerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
    }

    #[test]
    fn server_info_parses_the_f3_initialize_result() {
        let result = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"experimental": {}, "tools": {"listChanged": false}},
            "serverInfo": {"name": "t-bench", "version": "1.6.0"}
        });
        // The client extracts these three fields by key; the type itself
        // models only the protocolVersion + serverInfo pair the boundary
        // exposes. Re-derive from the raw result to mirror `initialize`.
        let info = SidecarServerInfo {
            protocol_version: result["protocolVersion"].as_str().unwrap().to_owned(),
            server_name: result["serverInfo"]["name"].as_str().unwrap().to_owned(),
            server_version: result["serverInfo"]["version"].as_str().unwrap().to_owned(),
        };
        assert_eq!(info.protocol_version, "2024-11-05");
        assert_eq!(info.server_name, "t-bench");
        assert_eq!(info.server_version, "1.6.0");
    }
}
