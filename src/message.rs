//! Local mirror of the two rig types the aura frame machinery touches,
//! plus the request envelope the golden corpus snapshots.
//!
//! These types exist so the ported producers and the golden harness stay
//! byte-identical to the aura corpus without depending on rig. S71 maps
//! them onto agent-driver-rs `Message`/tool types at the loop seam.

use serde::Serialize;

/// A single text turn. The corpus holds only single-part text turns.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User(String),
    Assistant(String),
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self::User(text.into())
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::Assistant(text.into())
    }
}

/// Mirror of `rig::completion::ToolDefinition`. Field declaration order is
/// part of the byte contract: serde serializes struct fields in order, and
/// the corpus's canonical JSON is `name`, `description`, `parameters`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Everything handed to the provider for one coordinator or worker call.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestEnvelope {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
}

impl RequestEnvelope {
    /// Canonical JSON for the tool definitions. `serde_json`'s
    /// `BTreeMap`-backed maps make this byte-deterministic.
    pub fn tools_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.tools).expect("tool definitions serialize")
    }
}
