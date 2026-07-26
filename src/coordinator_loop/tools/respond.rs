//! Writing the run's answer, without ending the loop by mechanism.

use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::ToolError;
use async_trait::async_trait;
use serde::Deserialize;

use super::super::error::CoordinatorLoopError;
use super::super::terminal::{FinalResponse, TerminalSlot};
use super::native_definition;

/// The answer the model wrote, as it arrived.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RespondArgs {
    pub response: String,
    #[serde(default)]
    pub response_summary: Option<String>,
}

impl TryFrom<RespondArgs> for FinalResponse {
    type Error = CoordinatorLoopError;

    fn try_from(_args: RespondArgs) -> Result<Self, Self::Error> {
        todo!("S71 Phase 2")
    }
}

/// Records the run's answer and acknowledges it.
///
/// The substrate has no terminal-tool concept, so this tool cannot end the
/// loop and does not pretend to: it writes into a first-write-wins slot and
/// returns an acknowledgement, and the run ends when the model stops calling
/// tools or the turn budget fires. A second call is refused, so the answer
/// the run committed to is the answer the user gets.
pub struct RespondTool {
    definition: ToolDefinition,
    answer: TerminalSlot<FinalResponse>,
}

impl RespondTool {
    /// Mount the answer tool over a run's answer slot.
    pub fn new(answer: TerminalSlot<FinalResponse>) -> Self {
        Self {
            definition: native_definition(
                "respond",
                "Write the final answer for the user. The user never sees task results, so the \
                 answer must state the concrete findings itself rather than refer to work that \
                 was done. Write it once: the first answer is the one delivered, and a second \
                 call is refused. Once you have written it, stop calling tools.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "response": {
                            "type": "string",
                            "description": "The complete answer, with the findings inlined."
                        },
                        "response_summary": {
                            "type": "string",
                            "description": "Optional one-line gloss of the answer."
                        }
                    },
                    "required": ["response"]
                }),
            ),
            answer,
        }
    }
}

#[async_trait]
impl Tool for RespondTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        _input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        todo!("S71 Phase 2")
    }
}
