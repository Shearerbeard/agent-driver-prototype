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

/// The "Resolve tool gaps" core-behavior directive, which
/// `AURA_ESCAPE_HATCH=false` strips from the coordinator preamble.
///
/// The literal must stay byte-identical to the directive line in
/// `orchestrator_preamble.md`; `escape_hatch_literal_matches_the_template`
/// pins that sync.
const TOOL_GAPS_DIRECTIVE: &str = "6. **Resolve tool gaps pragmatically**: If a user requests an operation with no matching tool, create a plan using the available tools and note the gap in `planning_rationale`. Do NOT deliberate at length about missing capabilities — plan what you can, report what you cannot.\n";

/// Build the coordinator's system prompt by composing the orchestrator
/// framework template with the user's domain-specific system prompt.
///
/// Layering: orchestration instructions → user system prompt → (worker
/// details injected into user message by the planning prompt).
///
/// The `agent_system_prompt` parameter is `[agent].system_prompt` from config.
///
/// The `include_recon_tools` and `include_history_tools` flags are retired
/// with the bounded router's tool surface: the registered coordinator
/// surface is `create_plan`, `execute`, `inspect_run` and `respond`
/// regardless of either flag, so neither affects the rendered preamble.
/// They remain in the signature until the bounded router retires; S103 is
/// the next event that re-opens this template.
pub fn build_coordinator_preamble(
    agent_system_prompt: &str,
    include_recon_tools: bool,
    include_history_tools: bool,
) -> String {
    let _ = (include_recon_tools, include_history_tools);

    let tools_section = "\
You have four tools to drive this run. Call them as needed:

1. `create_plan` — Decompose the request into an ordered task list of tasks assigned to workers.
2. `execute` — Run the tasks of a plan you created; it returns per-task evidence, not an answer.
3. `inspect_run` — Read back one of this run's own records when you need the full evidence.
4. `respond` — Write the final answer for the user. The first response is the one recorded.

Typical loop: `create_plan` → `execute` → `respond`, with `inspect_run` whenever an observation's summary is not enough.";

    let preamble =
        super::templates::render_coordinator_preamble(&super::templates::CoordinatorPreambleVars {
            orchestration_system_prompt: agent_system_prompt,
            tools_section,
        });

    // AURA_ESCAPE_HATCH=false strips the "Resolve tool gaps" directive for
    // A/B testing. The env var is read exactly once, here at the call site;
    // `apply_escape_hatch` is pure in the toggle. Inlined from
    // `aura::env_flags::bool_env("AURA_ESCAPE_HATCH", true)`; the canonical
    // truthy/falsy vocabulary is mirrored exactly (unrecognized values fall
    // back to the default, here `true`).
    let escape_hatch_on = match std::env::var("AURA_ESCAPE_HATCH") {
        Ok(v) if v.is_empty() => true,
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" | "on" => true,
            "0" | "false" | "f" | "no" | "n" | "off" => false,
            _ => true,
        },
        Err(_) => true,
    };
    apply_escape_hatch(preamble, escape_hatch_on)
}

/// Apply the `AURA_ESCAPE_HATCH` toggle to a rendered coordinator preamble.
///
/// Pure in the toggle: the environment is read once at
/// [`build_coordinator_preamble`]'s call site and passed in, so tests cover
/// both states through this helper without process-env mutation (the corpus
/// harness asserts the variable is unset under concurrent test execution).
fn apply_escape_hatch(preamble: String, escape_hatch_on: bool) -> String {
    if escape_hatch_on {
        return preamble;
    }
    preamble.replace(TOOL_GAPS_DIRECTIVE, "")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered_preamble() -> String {
        crate::templates::render_coordinator_preamble(&crate::templates::CoordinatorPreambleVars {
            orchestration_system_prompt: "Fixture playbook for escape-hatch checks.",
            tools_section: "You have four tools.",
        })
    }

    #[test]
    fn escape_hatch_on_keeps_the_tool_gaps_directive() {
        let preamble = apply_escape_hatch(rendered_preamble(), true);
        assert!(preamble.contains(TOOL_GAPS_DIRECTIVE));
    }

    #[test]
    fn escape_hatch_off_strips_the_tool_gaps_directive() {
        let preamble = apply_escape_hatch(rendered_preamble(), false);
        assert!(!preamble.contains("Resolve tool gaps"));
        assert!(preamble.contains("## Core Behavior"));
    }

    #[test]
    fn escape_hatch_literal_matches_the_template() {
        assert!(crate::templates::ORCHESTRATOR_PREAMBLE_TEMPLATE.contains(TOOL_GAPS_DIRECTIVE));
    }
}
