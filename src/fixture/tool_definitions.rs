//! Static tool-definition constructors returning `crate::message::ToolDefinition`
//! with the EXACT name, description, and parameters schema from each aura
//! tool's `definition()` body. The spike does NOT port rig tool structs;
//! these constructors transcribe the static `serde_json::json!` bodies
//! verbatim. The `load_skill` description is dynamic (lists available skills)
//! and takes the skill list as a parameter.
//!
//! Then `coordinator_tool_definitions` / `worker_tool_definitions` assemble
//! these in the same production registration order with the same
//! scenario-conditional inclusion logic.

use crate::config::SkillConfig;
use crate::message::ToolDefinition;

// ============================================================================
// Recon tools
// ============================================================================

/// `list_tools` tool definition (verbatim from `ListToolsTool::definition`).
pub fn list_tools_definition() -> ToolDefinition {
    ToolDefinition {
        name: "list_tools".to_string(),
        description: "List all available MCP tool names. Only use this if tool names \
             were not already provided in the planning context above."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

/// `inspect_tool_params` tool definition (verbatim from
/// `InspectToolParamsTool::definition`).
pub fn inspect_tool_params_definition() -> ToolDefinition {
    ToolDefinition {
        name: "inspect_tool_params".to_string(),
        description: "Get the parameter schema for a specific tool. Only use when you need \
             exact parameter details and the tool name alone is insufficient for planning."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "The name of the tool to inspect"
                }
            },
            "required": ["tool_name"]
        }),
    }
}

// ============================================================================
// Routing tools
// ============================================================================

/// `respond_directly` tool definition (verbatim from
/// `RespondDirectlyTool::definition`).
pub fn respond_directly_definition() -> ToolDefinition {
    ToolDefinition {
        name: "respond_directly".to_string(),
        description: "Answer the user's query directly. At initial routing, use \
            for simple factual questions that do not require tool execution. \
            At end-of-iteration, use to synthesize the completed task results \
            into the final answer — inline all concrete findings, names, and \
            values because the user does not see task results directly."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "response": {
                    "type": "string",
                    "description": "The complete response to send to the user. This is the ONLY text the user sees — include all concrete data points, names, and values from task results."
                },
                "routing_rationale": {
                    "type": "string",
                    "description": "Brief explanation of why this query can be answered directly"
                },
                "summary": {
                    "type": "string",
                    "description": "1-2 sentence summary of your response for session history. Provide when the response is longer than a few sentences."
                }
            },
            "required": ["response", "routing_rationale"]
        }),
    }
}

/// `create_plan` tool definition (verbatim from `CreatePlanTool::definition`).
pub fn create_plan_definition() -> ToolDefinition {
    let step_schema = serde_json::json!({
        "type": "object",
        "required": ["type"],
        "properties": {
            "type": {
                "type": "string",
                "enum": ["task", "parallel", "chain"],
                "description": "Step kind: 'task' for a single task, 'parallel' for concurrent steps, 'chain' for a sequential sub-chain inside a parallel group"
            },
            "task": {
                "type": "string",
                "description": "What this task accomplishes. Fully resolve all references — workers do NOT see conversation history. Required when type=task."
            },
            "worker": {
                "type": "string",
                "description": "Name of the specialized worker to assign this task to. Required when type=task."
            },
            "items": {
                "type": "array",
                "description": "Steps to run concurrently. Required when type=parallel.",
                "items": { "type": "object" }
            },
            "steps": {
                "type": "array",
                "description": "Sequential steps in a sub-chain. Required when type=chain.",
                "items": { "type": "object" }
            }
        }
    });

    ToolDefinition {
        name: "create_plan".to_string(),
        description: "Decompose the user's query into an ordered sequence of steps for \
            orchestrated execution. Steps run sequentially by default. Use \
            {\"parallel\": [...]} when tasks are independent. Use this for queries \
            requiring tool execution, data gathering, system inspection, or multi-step \
            analysis."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The overall goal this plan addresses"
                },
                "steps": {
                    "type": "array",
                    "description": "Ordered steps to execute. Each step runs after the previous one completes. Use {\"parallel\": [...]} to run independent steps concurrently.",
                    "items": step_schema
                },
                "routing_rationale": {
                    "type": "string",
                    "description": "Brief explanation of why this query requires orchestration"
                },
                "planning_summary": {
                    "type": "string",
                    "minLength": 1,
                    "description": "REQUIRED. Summarize the plan in natural language: what steps will run, in what order, and what the expected outcome is."
                }
            },
            "required": ["goal", "steps", "routing_rationale", "planning_summary"]
        }),
    }
}

/// `request_clarification` tool definition (verbatim from
/// `RequestClarificationTool::definition`).
pub fn request_clarification_definition() -> ToolDefinition {
    ToolDefinition {
        name: "request_clarification".to_string(),
        description: "Request clarification from the user when the query is genuinely \
            ambiguous. Use sparingly — prefer create_plan when a reasonable interpretation \
            exists."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The clarification question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional suggested choices for the user"
                },
                "routing_rationale": {
                    "type": "string",
                    "description": "Brief explanation of why clarification is needed"
                }
            },
            "required": ["question", "routing_rationale"]
        }),
    }
}

// ============================================================================
// Artifact / history tools
// ============================================================================

/// `read_artifact` tool definition (verbatim from `ReadArtifactTool::definition`).
pub fn read_artifact_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read_artifact".to_string(),
        description: "Read the content of a result artifact. By default reads from \
            the current run. Supply an optional run_id to read artifacts from a prior run \
            in this session (see session history for available run_id values). A large \
            artifact is returned as a scratchpad pointer to explore in place (with head, \
            grep, slice, etc.) rather than inlined."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The artifact filename (e.g. 'task-0-sre-iter-1-result.txt')"
                },
                "run_id": {
                    "type": "string",
                    "description": "Run ID for cross-run artifact access. Omit to read from the current run."
                }
            },
            "required": ["filename"]
        }),
    }
}

/// `list_prior_runs` tool definition (verbatim from
/// `ListPriorRunsTool::definition`).
pub fn list_prior_runs_definition() -> ToolDefinition {
    ToolDefinition {
        name: "list_prior_runs".to_string(),
        description: "List all prior runs in the current session. Returns run metadata \
             including run_id, goal, outcome, and artifact counts. Use run_id values \
             with read_artifact for cross-run artifact access."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }
}

// ============================================================================
// Worker tools
// ============================================================================

/// `submit_result` tool definition (verbatim from
/// `SubmitResultTool::definition`).
pub fn submit_result_definition() -> ToolDefinition {
    ToolDefinition {
        name: "submit_result".to_string(),
        description: "Submit your structured result. Call this once when you have \
            your final answer. Provide a concise summary for the coordinator, \
            your complete findings, and your confidence level."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Concise summary of findings (1-3 sentences). This becomes the preview shown to the coordinator and stored in session history."
                },
                "result": {
                    "type": "string",
                    "description": "Complete findings and analysis."
                },
                "confidence": {
                    "type": "string",
                    "enum": ["high", "medium", "low"],
                    "description": "Confidence in the result. 'low' if key data was unavailable or ambiguous."
                }
            },
            "required": ["summary", "result", "confidence"]
        }),
    }
}

// ============================================================================
// Skill tools
// ============================================================================

/// `load_skill` tool definition. The description is dynamic — it lists the
/// available skills. Ported from `LoadSkillTool::definition` +
/// `LoadSkillTool::build_description`.
pub fn load_skill_definition(skills: &[SkillConfig]) -> ToolDefinition {
    let mut desc =
        String::from("Load detailed instructions for a specific skill. Available skills:\n");
    for skill in skills {
        desc.push_str(&format!("- {}: {}\n", skill.name, skill.description));
    }
    ToolDefinition {
        name: "load_skill".to_string(),
        description: desc,
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the skill to load"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
    }
}

/// `read_skill_file` tool definition (verbatim from
/// `ReadSkillFileTool::definition`).
pub fn read_skill_file_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read_skill_file".to_string(),
        description: "Read a resource file from a named skill. Use one of the relative \
                      paths listed under 'Skill resources' in the output of `load_skill`."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Name of the skill that owns the resource"
                },
                "path": {
                    "type": "string",
                    "description": "Relative path to the resource file, e.g. 'references/REFERENCE.md'"
                }
            },
            "required": ["skill", "path"],
            "additionalProperties": false
        }),
    }
}
