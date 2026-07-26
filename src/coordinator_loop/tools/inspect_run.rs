//! Pull-on-demand reads of the run's own records.

use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::ToolError;
use async_trait::async_trait;
use serde::Deserialize;

use super::super::plan_id::PlanId;
use super::super::run_store::RunStore;
use super::native_definition;

/// Which record to read back.
///
/// Both cases name a record the run actually holds. There is no free-form
/// query: a selector that cannot be resolved to a stored record would be a
/// request the loop can only answer with an apology.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum RunSelector {
    /// A plan this run created; omit the handle for the most recent one.
    Plan {
        #[serde(default)]
        plan_id: Option<PlanId>,
    },
    /// The most recent execution observation.
    LatestExecution,
}

/// What the coordinator wants to read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InspectRunArgs {
    pub selector: RunSelector,
}

/// Reads a stored plan or execution back into the conversation.
///
/// This is what lets plan observations stay compact: task bodies live in the
/// run rather than in the conversation, and the coordinator pulls them back
/// only when it needs them.
pub struct InspectRunTool {
    definition: ToolDefinition,
    runs: RunStore,
}

impl InspectRunTool {
    /// Mount the inspection tool over a run's records.
    pub fn new(runs: RunStore) -> Self {
        Self {
            definition: native_definition(
                "inspect_run",
                "Read back one of this run's own records: a plan you created, or the most \
                 recent execution. Use it when you need the task text or the full evidence \
                 that an earlier observation summarised.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "selector": {
                            "type": "object",
                            "description": "Which record to read.",
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "record": { "const": "plan" },
                                        "plan_id": {
                                            "type": "string",
                                            "description": "Omit for the most recent plan."
                                        }
                                    },
                                    "required": ["record"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "record": { "const": "latest_execution" }
                                    },
                                    "required": ["record"]
                                }
                            ]
                        }
                    },
                    "required": ["selector"]
                }),
            ),
            runs,
        }
    }
}

#[async_trait]
impl Tool for InspectRunTool {
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
