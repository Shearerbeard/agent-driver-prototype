//! Runtime orchestration helpers.
//!
//! The pure, serializable orchestration config types live in
//! `aura_config::orchestration`. This module re-exports them and holds the
//! runtime-only helpers that depend on aura's prompt templates or the
//! `env_flags` escape-hatch toggle: coordinator/worker preamble building and
//! vector-store context strings.

use crate::config::VectorStoreConfig;

// ============================================================================
// Vector Store Context Helpers
// ============================================================================

/// Build a formatted context string describing available vector stores.
///
/// This is injected into the agent's system prompt so it knows about its RAG
/// capabilities upfront, rather than discovering them via tool inspection.
///
/// # Example output
///
/// ```text
/// ## Available Knowledge Bases
///
/// You have access to the following knowledge bases for retrieval:
///
/// - **mezmo_docs**: Mezmo documentation and knowledge base articles...
///   Tool: `vector_search_mezmo_docs`
/// ```
pub fn build_vector_store_context(stores: &[VectorStoreConfig]) -> String {
    if stores.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n## Available Knowledge Bases\n\n");
    context.push_str("You have access to the following knowledge bases for retrieval:\n\n");

    for store in stores {
        let description = store
            .context_prefix
            .as_deref()
            .unwrap_or("No description provided");
        context.push_str(&format!(
            "- **{}**: {}\n  Tool: `vector_search_{}`\n\n",
            store.name, description, store.name
        ));
    }

    context
}

// ============================================================================
// Preamble Builders
// ============================================================================

/// Build the coordinator's system prompt by composing the orchestrator
/// framework template with the user's domain-specific system prompt.
///
/// Layering: orchestration instructions → user system prompt → (worker
/// details injected into user message by the planning prompt).
///
/// The `agent_system_prompt` parameter is `[agent].system_prompt` from config.
pub fn build_coordinator_preamble(
    agent_system_prompt: &str,
    include_recon_tools: bool,
    include_history_tools: bool,
) -> String {
    let artifact_tools = if include_history_tools {
        "two **artifact/history tools** (`read_artifact`, `list_prior_runs`)"
    } else {
        "one **artifact tool** (`read_artifact`)"
    };

    let tools_section = if include_recon_tools {
        format!(
            "You have three **routing tools** (`respond_directly`, `create_plan`, `request_clarification`), \
             two **reconnaissance tools** (`list_tools`, `inspect_tool_params`), and {artifact_tools}. \
             Call exactly one routing tool per query."
        )
    } else {
        format!(
            "You have three **routing tools** (`respond_directly`, `create_plan`, `request_clarification`) \
             and {artifact_tools}. Call exactly one routing tool per query."
        )
    };

    let recon_guidance = if include_recon_tools {
        "## Reconnaissance Guidance\n\n\
         Tool names and worker capabilities are already listed in the planning context below. \
         You do NOT need to call `list_tools` or `inspect_tool_params` to discover what's available \
         — that information is already provided to you.\n\n\
         Only call `inspect_tool_params` when you need the **exact parameter schema** for a tool \
         (e.g., to decide between two similar tools based on their parameters). In most cases, \
         the tool name and worker description are sufficient for planning.\n\n\
         **Budget awareness**: Each planning attempt has a limited number of tool calls. \
         Prioritize calling a routing tool (`respond_directly`, `create_plan`, or \
         `request_clarification`) over reconnaissance. Do not spend multiple turns inspecting tools.\n\n\
         **Worker names vs tool names**: The worker names listed below (e.g., \"arithmetic\", \
         \"statistics\") are role assignments for task routing — they are NOT callable tools. \
         Only the tools listed under each worker (e.g., \"add\", \"mean\", \"sin\") are MCP tools \
         that workers can execute."
    } else {
        "**Worker names vs tool names**: The worker names listed below (e.g., \"arithmetic\", \
         \"statistics\") are role assignments for task routing — they are NOT callable tools. \
         Only the tools listed under each worker (e.g., \"add\", \"mean\", \"sin\") are MCP tools \
         that workers can execute."
    };

    let mut preamble =
        super::templates::render_coordinator_preamble(&super::templates::CoordinatorPreambleVars {
            orchestration_system_prompt: agent_system_prompt,
            tools_section: &tools_section,
            recon_guidance,
        });

    // AURA_ESCAPE_HATCH=false strips the "Resolve tool gaps" directive for A/B testing.
    // Inlined from `aura::env_flags::bool_env("AURA_ESCAPE_HATCH", true)`; the
    // canonical truthy/falsy vocabulary is mirrored exactly (unrecognized
    // values fall back to the default, here `true`).
    let escape_hatch_on = match std::env::var("AURA_ESCAPE_HATCH") {
        Ok(v) if v.is_empty() => true,
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" | "on" => true,
            "0" | "false" | "f" | "no" | "n" | "off" => false,
            _ => true,
        },
        Err(_) => true,
    };
    if !escape_hatch_on {
        preamble = preamble.replace(
            "6. **Resolve tool gaps pragmatically**: If a user requests an operation with no matching tool, create a plan using the available tools and note the gap in `planning_summary`. Do NOT deliberate at length about missing capabilities — route what you can, report what you cannot.\n",
            "",
        );
    }

    preamble
}

/// Build the complete worker preamble by injecting the custom system prompt
/// into the worker template.
///
/// The template contains `%%WORKER_SYSTEM_PROMPT%%` which is replaced with
/// the user's custom prompt, or a default message if none is provided.
pub fn build_worker_preamble(config: &crate::config::OrchestrationConfig) -> String {
    let custom_prompt = config
        .worker_system_prompt
        .as_deref()
        .unwrap_or("(No custom instructions provided)");

    super::templates::render_worker_preamble(&super::templates::WorkerPreambleVars {
        worker_system_prompt: custom_prompt,
    })
}
