//! Pull-on-demand reads of the run's own records.

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;

use super::super::plan_id::PlanId;
use super::super::run_store::RunStore;
use super::{native_definition, observation_result};

/// Which record to read back.
///
/// Each case names a record the run actually holds, and "the latest plan" is
/// its own case rather than an absent handle: an optional field would encode
/// a second selector inside the first, which is the shape `ExecuteArgs`
/// rejects for the same reason. The `Task` case addresses a per-task
/// execution record by task id and attempt together, the key the S72 run
/// journal uses.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum RunSelector {
    /// A specific plan this run created.
    Plan { plan_id: PlanId },
    /// The most recently created plan.
    LatestPlan,
    /// The most recent execution observation.
    LatestExecution,
    /// A per-task execution record, keyed by task id and attempt.
    Task { task_id: usize, attempt: usize },
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
                "Read back one of this run's own records: a plan you created, the most recent \
                 plan, or the most recent execution. Use it when you need the task text or the \
                 full evidence that an earlier observation summarised.",
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
                                            "description": "The plan_id returned by \
                                                            `create_plan`."
                                        }
                                    },
                                    "required": ["record", "plan_id"]
                                },
                                {
                                    "type": "object",
                                    "properties": { "record": { "const": "latest_plan" } },
                                    "required": ["record"]
                                },
                                {
                                    "type": "object",
                                    "properties": { "record": { "const": "latest_execution" } },
                                    "required": ["record"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "record": { "const": "task" },
                                        "task_id": {
                                            "type": "integer",
                                            "description": "The task id to read."
                                        },
                                        "attempt": {
                                            "type": "integer",
                                            "description": "The attempt number (1-indexed)."
                                        }
                                    },
                                    "required": ["record", "task_id", "attempt"]
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
        input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: InspectRunArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "inspect_run arguments did not parse: {error}"
                )));
            }
        };

        Ok(match args.selector {
            RunSelector::Plan { plan_id } => match self.runs.plan(&plan_id) {
                Some(plan) => observation_result(&plan),
                None => {
                    ToolResult::error(format!("no plan with id {plan_id} was created in this run"))
                }
            },
            RunSelector::LatestPlan => match self.runs.latest_plan() {
                Some((_, plan)) => observation_result(&plan),
                None => ToolResult::error("this run has not created a plan yet".to_owned()),
            },
            RunSelector::LatestExecution => match self.runs.latest_execution() {
                Some(execution) => observation_result(&execution),
                None => ToolResult::error("this run has not executed a plan yet".to_owned()),
            },
            RunSelector::Task { task_id, attempt } => {
                let _ = (task_id, attempt);
                todo!(
                    "Phase 2: read the task record keyed by (task_id, attempt) \
                     from the run store"
                )
            }
        })
    }
}
