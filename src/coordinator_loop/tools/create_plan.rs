//! Planning as an ordinary tool: decompose the request, keep the loop going.

use agent_driver_rs::ToolError;
use agent_driver_rs::tool::{Tool, ToolContext, ToolDefinition, ToolInput, ToolResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use crate::types::{Plan, StepInput, flatten_steps};

use super::super::driver::WorkerSections;
use super::super::error::CoordinatorLoopError;
use super::super::observation::PlanObservation;
use super::super::roster::WorkerRoster;
use super::super::run_store::RunStore;
use super::{native_definition, observation_result};

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

impl CreatePlanArgs {
    /// Turn the proposed steps into a dependency-ordered plan.
    ///
    /// Two rules decide it. Flattening is the structural check: a tree that
    /// cannot become a task list is not a plan. The roster is the semantic
    /// one: a task assigned to a worker that does not exist would be
    /// dispatched nowhere.
    ///
    /// # Errors
    ///
    /// Returns [`CoordinatorLoopError::UnexecutableSteps`] when the step
    /// tree does not flatten, and [`CoordinatorLoopError::UnknownWorker`]
    /// when a task names a worker the run has not configured.
    pub fn to_plan(&self, roster: &WorkerRoster) -> Result<Plan, CoordinatorLoopError> {
        let tasks = flatten_steps(&self.steps).map_err(CoordinatorLoopError::UnexecutableSteps)?;
        for task in &tasks {
            if let Some(worker) = task.worker.as_deref() {
                roster.check(worker)?;
            }
        }
        let mut plan = Plan::new(self.goal.trim());
        plan.steps = Some(self.steps.clone());
        for task in tasks {
            plan.add_task(task);
        }
        Ok(plan)
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
    roster: WorkerRoster,
}

impl CreatePlanTool {
    /// Mount the planning tool over a run's records and its worker roster.
    ///
    /// The roster reaches both the schema, which offers the configured names
    /// and nothing else, and the parse, which rejects a name the schema did
    /// not offer.
    pub fn new(runs: RunStore, sections: &WorkerSections) -> Self {
        let roster = sections.roster().clone();
        Self {
            definition: native_definition(
                "create_plan",
                "Decompose the request into an ordered task list. Creating a plan does not run \
                 it and does not answer the user: it returns a plan_id you pass to `execute`. \
                 You may create a plan, look at it, revise it, or execute it - this call leaves \
                 you in control either way.",
                plan_schema(&roster, sections.worker_field()),
            ),
            runs,
            roster,
        }
    }
}

/// The planning schema, with the worker property present only when workers
/// are configured to receive assignments.
fn plan_schema(roster: &WorkerRoster, worker_field: &str) -> JsonValue {
    let mut leaf_properties = json!({
        "type": { "const": "task" },
        "task": {
            "type": "string",
            "description": "What the worker must do, stated so it can be done without \
                            seeing the rest of the plan."
        }
    });
    if !roster.is_empty() {
        let names: Vec<&str> = roster
            .names()
            .iter()
            .map(crate::context::WorkerRole::as_str)
            .collect();
        if let Some(properties) = leaf_properties.as_object_mut() {
            properties.insert(
                "worker".to_owned(),
                json!({
                    "type": "string",
                    "enum": names,
                    "description": "Worker to assign; omit to leave the task unassigned."
                }),
            );
        }
    }

    let steps_description = format!(
        "Ordered steps. Steps run in sequence; use a parallel group for work that has no \
         ordering between its branches. A task step has the shape {{ \"type\": \"task\", \
         \"task\": \"...\"{worker_field} }}."
    );

    json!({
        "type": "object",
        "properties": {
            "goal": {
                "type": "string",
                "description": "What the whole plan is meant to achieve."
            },
            "steps": {
                "type": "array",
                "description": steps_description,
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
                        "properties": leaf_properties,
                        "required": ["type", "task"]
                    },
                    {
                        "type": "object",
                        "properties": {
                            "type": { "const": "parallel" },
                            "items": { "type": "array", "items": { "$ref": "#/$defs/step" } }
                        },
                        "required": ["type", "items"]
                    },
                    {
                        "type": "object",
                        "properties": {
                            "type": { "const": "chain" },
                            "steps": { "type": "array", "items": { "$ref": "#/$defs/step" } }
                        },
                        "required": ["type", "steps"]
                    }
                ]
            }
        }
    })
}

#[async_trait]
impl Tool for CreatePlanTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }

    async fn execute(
        &self,
        input: &ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: CreatePlanArgs = match input.parse() {
            Ok(args) => args,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "create_plan arguments did not parse: {error}"
                )));
            }
        };

        let plan = match args.to_plan(&self.roster) {
            Ok(plan) => plan,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };

        let plan_id = self.runs.record_plan(&args, plan.clone());
        match PlanObservation::from_plan(plan_id, &plan) {
            Ok(observation) => Ok(observation_result(&observation)),
            Err(error) => Ok(ToolResult::error(error.to_string())),
        }
    }
}
