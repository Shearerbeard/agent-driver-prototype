//! Static tool-definition constructors returning `crate::message::ToolDefinition`
//! with the EXACT name, description, and parameters schema from each aura
//! tool's `definition()` body. The spike does NOT port rig tool structs;
//! these constructors transcribe the static `serde_json::json!` bodies
//! verbatim. The `load_skill` description is dynamic (lists available skills)
//! and takes the skill list as a parameter.
//!
//! The remaining constructors are all WORKER-side registrations. The
//! coordinator envelope attaches the shared factory's definitions
//! (`coordinator_loop::coordinator_tool_definitions`) projected onto this
//! mirror type, so the retired router/recon/history constructors were
//! removed with that migration (S114).

use crate::config::SkillConfig;
use crate::message::ToolDefinition;

// ============================================================================
// Worker tools
// ============================================================================

/// `read_artifact` tool definition (verbatim from `ReadArtifactTool::definition`).
/// Worker-side registration: workers mount `read_artifact` over the run's
/// artifacts (`src/dag_executor/mount.rs`); the coordinator does not.
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
