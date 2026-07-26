//! Execution as an ordinary tool: run a plan, observe it, keep the loop going.

use std::sync::Arc;

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;

use super::super::executor::PlanExecutor;
use super::super::plan_id::PlanId;
use super::super::run_store::RunStore;
use super::{native_definition, observation_result};

/// Which plan to run.
///
/// The handle is required rather than defaulted, because "the latest plan"
/// is ambiguous the moment the coordinator revises one, and running the
/// wrong plan is not recoverable from an observation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExecuteArgs {
    pub plan_id: PlanId,
}

/// Runs a stored plan and reports what its tasks produced.
///
/// Nonterminal: the observation goes back into the same conversation, so the
/// coordinator reads the evidence and decides what to do next without a
/// break in the loop.
pub struct ExecuteTool {
    definition: ToolDefinition,
    runs: RunStore,
    executor: Arc<dyn PlanExecutor>,
}

impl ExecuteTool {
    /// Mount the execution tool over a run's records and an executor.
    pub fn new(runs: RunStore, executor: Arc<dyn PlanExecutor>) -> Self {
        Self {
            definition: native_definition(
                "execute",
                "Run the tasks of a plan you created and observe what they produced. Returns \
                 per-task evidence and an outcome tally; it does not answer the user. You stay \
                 in control after it returns: read the evidence, execute another plan, or write \
                 the answer with `respond`.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "plan_id": {
                            "type": "string",
                            "description": "The plan_id returned by `create_plan`."
                        }
                    },
                    "required": ["plan_id"]
                }),
            ),
            runs,
            executor,
        }
    }
}

#[async_trait]
impl Tool for ExecuteTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(&self, input: &ToolInput, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ExecuteArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "execute arguments did not parse: {error}"
                )));
            }
        };

        let Some(plan) = self.runs.plan(&args.plan_id) else {
            return Ok(ToolResult::error(format!(
                "no plan with id {} was created in this run; call create_plan first",
                args.plan_id
            )));
        };

        let observation = self.executor.execute(&plan, ctx).await;
        self.runs.record_execution(observation.clone());
        Ok(observation_result(&observation))
    }
}
