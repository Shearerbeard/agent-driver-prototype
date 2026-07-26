//! Planning as an ordinary tool: decompose the request, keep the loop going.

use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use agent_driver_rs::ToolError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{Plan, StepInput};

use super::super::error::CoordinatorLoopError;
use super::super::run_store::RunStore;
use super::native_definition;

/// The plan the model proposed, exactly as it arrived.
///
/// This is the wire shape and nothing more: the fields are unvalidated
/// because validation is what turns them into a [`Plan`]. Nothing downstream
/// accepts these arguments directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreatePlanArgs {
    pub goal: String,
    pub steps: Vec<StepInput>,
    pub planning_rationale: String,
}

/// Flattening the step tree is the validation step: a tree that cannot
/// become a dependency-ordered task list is not a plan.
impl TryFrom<CreatePlanArgs> for Plan {
    type Error = CoordinatorLoopError;

    fn try_from(_args: CreatePlanArgs) -> Result<Self, Self::Error> {
        todo!("S71 Phase 2")
    }
}

/// Records a plan and hands back its handle.
///
/// Nonterminal by construction: it stores the plan and returns an
/// observation, so the coordinator can revise, inspect, or execute next
/// without the loop having gone anywhere.
pub struct CreatePlanTool {
    definition: ToolDefinition,
    runs: RunStore,
}

impl CreatePlanTool {
    /// Mount the planning tool over a run's records.
    pub fn new(runs: RunStore) -> Self {
        Self {
            definition: native_definition(
                "create_plan",
                "Decompose the request into an ordered task list. Creating a plan does not run \
                 it and does not answer the user: it returns a plan_id you pass to `execute`. \
                 You may create a plan, look at it, revise it, or execute it — this call leaves \
                 you in control either way.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "goal": {
                            "type": "string",
                            "description": "What the whole plan is meant to achieve."
                        },
                        "steps": {
                            "type": "array",
                            "description": "Ordered steps. Steps run in sequence; use a \
                                            parallel group for work that has no ordering \
                                            between its branches.",
                            "items": { "$ref": "#/$defs/step" }
                        },
                        "planning_rationale": {
                            "type": "string",
                            "description": "Why this decomposition, in one or two sentences."
                        }
                    },
                    "required": ["goal", "steps", "planning_rationale"],
                    "$defs": {
                        "step": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": { "const": "task" },
                                        "task": {
                                            "type": "string",
                                            "description": "What the worker must do, stated \
                                                            so it can be done without seeing \
                                                            the rest of the plan."
                                        },
                                        "worker": {
                                            "type": "string",
                                            "description": "Worker to assign; omit to leave \
                                                            the task unassigned."
                                        }
                                    },
                                    "required": ["type", "task"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": { "const": "parallel" },
                                        "items": {
                                            "type": "array",
                                            "items": { "$ref": "#/$defs/step" }
                                        }
                                    },
                                    "required": ["type", "items"]
                                },
                                {
                                    "type": "object",
                                    "properties": {
                                        "type": { "const": "chain" },
                                        "steps": {
                                            "type": "array",
                                            "items": { "$ref": "#/$defs/step" }
                                        }
                                    },
                                    "required": ["type", "steps"]
                                }
                            ]
                        }
                    }
                }),
            ),
            runs,
        }
    }
}

#[async_trait]
impl Tool for CreatePlanTool {
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
