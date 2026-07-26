//! The worker side of the tool surface: a worker's attested result.

use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::ToolError;
use async_trait::async_trait;
use serde::Deserialize;

use crate::tools::submit_result::Confidence;

use super::super::error::CoordinatorLoopError;
use super::super::terminal::{TerminalSlot, WorkerSubmission};
use super::native_definition;

/// What a worker reported, as it arrived.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SubmitResultArgs {
    pub summary: String,
    pub result: String,
    pub confidence: Confidence,
}

impl TryFrom<SubmitResultArgs> for WorkerSubmission {
    type Error = CoordinatorLoopError;

    fn try_from(_args: SubmitResultArgs) -> Result<Self, Self::Error> {
        todo!("S71 Phase 2")
    }
}

/// Records a worker's attested result.
///
/// It belongs to worker sessions, not the coordinator's: a worker reports
/// evidence, and the coordinator reads that evidence back through an
/// execution observation. It is defined here because the loop's tool surface
/// is one surface; the session that mounts it is what makes it a worker
/// tool.
pub struct SubmitResultTool {
    definition: ToolDefinition,
    submission: TerminalSlot<WorkerSubmission>,
}

impl SubmitResultTool {
    /// Mount the worker result tool over a worker's submission slot.
    pub fn new(submission: TerminalSlot<WorkerSubmission>) -> Self {
        Self {
            definition: native_definition(
                "submit_result",
                "Report what you produced. The summary is what the coordinator reads first, so \
                 state the finding rather than the activity. Submit once: the first submission \
                 is the one recorded.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "One or two sentences stating the finding."
                        },
                        "result": {
                            "type": "string",
                            "description": "The full result the summary is about."
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["high", "medium", "low"],
                            "description": "How much the evidence supports the summary."
                        }
                    },
                    "required": ["summary", "result", "confidence"]
                }),
            ),
            submission,
        }
    }
}

#[async_trait]
impl Tool for SubmitResultTool {
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
