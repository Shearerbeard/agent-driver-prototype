//! Orchestrator-side frame producer builders, ported as free functions from
//! `crates/aura/src/orchestration/orchestrator.rs`.
//!
//! Each function was an `Orchestrator` method in aura; here it is a free
//! function that takes the `&self` state it reads as parameters. The bodies
//! are byte-fidelity ports: only crate paths (`super::` → `crate::`) and the
//! documented MCP-less substitutions differ.

use std::collections::HashMap;

use crate::bounding::ToolListLimit;
use crate::config::{OrchestrationConfig, VectorStoreConfig};
use crate::context::{
    AncestorDistance, CoordinatorTurn, CorrelationLabel, DependencyRelation, EvidenceEntry,
    PriorWorkEntry, PriorWorkFrame, TaskId, TokenBudget, WorkerClaim, WorkerRole,
};
use crate::types::{Plan, PlanningResponse, TaskState};

// ============================================================================
// Planning wrapper (iter-1)
// ============================================================================

/// Build the iter-1 planning wrapper (fresh query, no prior iteration).
/// Enumerates the three routing tools with neutral bullets.
pub fn build_planning_wrapper(
    query: &str,
    worker_section: &str,
    worker_guidelines: &str,
) -> String {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    crate::templates::render_planning_prompt(&crate::templates::PlanningVars {
        timestamp: &timestamp,
        query,
        worker_section,
        worker_guidelines,
    })
}

// ============================================================================
// Continuation wrapper (post-execute decision point)
// ============================================================================

/// Build the post-execute continuation wrapper (end-of-iteration decision
/// point). Renders the continuation prompt from the iteration context and
/// deliberately does NOT re-enumerate the three routing tools — the
/// coordinator already has them in its preamble, and re-listing them here
/// would layer additional tool-choice bias into the user message.
pub fn build_continuation_wrapper(
    ctx: &crate::types::IterationContext,
    max_iterations: usize,
    show_tool_chain: bool,
    content_max_length: usize,
) -> String {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let base = ctx.build_continuation_prompt(max_iterations, show_tool_chain, content_max_length);
    crate::templates::render_continuation_wrapper(&crate::templates::ContinuationWrapperVars {
        timestamp: &timestamp,
        continuation_body: &base,
    })
}

// ============================================================================
// Compact decision turn
// ============================================================================

/// The assistant turn recorded after a routing decision: the compact
/// decision text — variant, rationale, and plan shape for `create_plan`;
/// the model's actual response or question text for the terminal
/// variants — never the pretty-printed `PlanningResponse` JSON, which
/// duplicated every task description into the conversation
/// (`docs/redesign/ARCHITECTURE.md` sections 2.2-2.3).
///
/// A decision the compact recorder rejects (empty rationale, empty
/// plan, empty response or question text) degrades to the model's own
/// streamed text, then to the bare variant name, rather than failing
/// the run. The variant-name tier is structurally task-body-free; the
/// model-text tier records narration that can mention tasks, matching
/// the old primary recording, so the at-most-once property still holds
/// because the continuation adds no second copy.
pub fn compact_decision_turn(decision: &PlanningResponse, model_text: &str) -> String {
    match CoordinatorTurn::try_from(decision) {
        Ok(turn) => String::from(turn.render()),
        Err(e) => {
            tracing::warn!("Routing decision has no compact turn ({e}); recording fallback text");
            if model_text.trim().is_empty() {
                decision.variant_name().to_owned()
            } else {
                model_text.to_owned()
            }
        }
    }
}

// ============================================================================
// Task context (prior-work frame)
// ============================================================================

/// Build the read-only prior-work context for a worker task.
///
/// Walks the completed ancestor closure of `task_id`, builds a
/// [`PriorWorkEntry`] for each completed ancestor, and assembles them
/// under the default token budget. Direct dependencies are the floor;
/// transitive ancestors fill remaining budget nearest-first.
///
/// Returns `None` when the task has no completed ancestors, so the
/// `%%CONTEXT%%` slot is left empty.
pub fn build_task_context(plan: &Plan, task_id: usize) -> Option<String> {
    let ancestors = completed_ancestors(plan, task_id);
    if ancestors.is_empty() {
        return None;
    }

    let entries: Vec<PriorWorkEntry> = ancestors
        .into_iter()
        .filter_map(|(ancestor_id, distance)| {
            let ancestor = plan.tasks.iter().find(|t| t.id == ancestor_id)?;
            let result = match &ancestor.state {
                TaskState::Complete { result } => result,
                _ => return None,
            };

            let label = CorrelationLabel {
                task: TaskId::new(ancestor_id),
                worker: ancestor
                    .worker
                    .as_deref()
                    .and_then(|w| WorkerRole::new(w).ok()),
            };

            let claim = ancestor
                .structured_output
                .as_ref()
                .and_then(|so| WorkerClaim::try_from(so).ok());

            let evidence = match EvidenceEntry::from_completed_result(result, claim) {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(
                        "failed to build evidence entry for task {}: {}",
                        ancestor_id,
                        e
                    );
                    return None;
                }
            };

            let relation = if distance == 1 {
                DependencyRelation::Direct
            } else {
                DependencyRelation::Transitive {
                    distance: AncestorDistance::new(distance)
                        .expect("distance >= 2 for transitive"),
                }
            };

            Some(PriorWorkEntry {
                label,
                relation,
                evidence,
            })
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    let frame = PriorWorkFrame::assemble(entries, TokenBudget::default()).ok()?;
    Some(String::from(frame.render()))
}

/// Find all completed ancestors of `task_id` with their shortest edge
/// distance. Distance 1 is a direct dependency; larger distances are
/// transitive. Results are returned in plan order (ascending task id).
pub fn completed_ancestors(plan: &Plan, task_id: usize) -> Vec<(usize, usize)> {
    use std::collections::{HashMap, VecDeque};

    let mut best_distance: HashMap<usize, usize> = HashMap::new();
    let mut queue = VecDeque::new();
    best_distance.insert(task_id, 0);
    queue.push_back(task_id);

    while let Some(current_id) = queue.pop_front() {
        let current_distance = best_distance[&current_id];
        if let Some(current_task) = plan.tasks.iter().find(|t| t.id == current_id) {
            for &dep_id in &current_task.dependencies {
                let next_distance = current_distance + 1;
                match best_distance.entry(dep_id) {
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if next_distance < *e.get() {
                            e.insert(next_distance);
                            queue.push_back(dep_id);
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(next_distance);
                        queue.push_back(dep_id);
                    }
                }
            }
        }
    }

    best_distance.remove(&task_id);

    let mut completed: Vec<(usize, usize)> = best_distance
        .into_iter()
        .filter(|(id, _)| {
            plan.tasks
                .iter()
                .any(|t| t.id == *id && matches!(t.state, TaskState::Complete { .. }))
        })
        .collect();
    completed.sort_by_key(|(id, _)| *id);
    completed
}

// ============================================================================
// Worker prompt sections
// ============================================================================

/// Build the worker-related sections of the planning prompt.
///
/// Based on `tools_in_planning` config:
/// - `None`: Just worker descriptions (original behavior)
/// - `Summary`: Worker descriptions + tool names
/// - `Full`: Worker descriptions + tool names + descriptions
pub fn build_worker_prompt_sections(
    config: &OrchestrationConfig,
    tool_list_limit: ToolListLimit,
    vector_stores: &[VectorStoreConfig],
    inventory: &ToolInventory,
) -> (String, String, String) {
    use crate::config::ToolVisibility;

    if config.has_workers() {
        let worker_names: Vec<&str> = config.available_worker_names();
        let names_json: Vec<String> = worker_names.iter().map(|n| format!("\"{}\"", n)).collect();

        let field = r#",
      "worker": "worker_name""#
            .to_string();

        let guidelines =
            crate::templates::render_worker_guidelines(&crate::templates::WorkerGuidelinesVars {
                valid_worker_names: &names_json.join(", "),
            });

        let section = match &config.tools_in_planning {
            ToolVisibility::None => build_workers_section_no_tools(config),
            ToolVisibility::Summary => {
                build_workers_section_with_tools(config, tool_list_limit, inventory)
            }
            ToolVisibility::Full => build_workers_section_with_full_tools(
                config,
                tool_list_limit,
                vector_stores,
                inventory,
            ),
        };

        (section, field, guidelines)
    } else {
        (String::new(), String::new(), String::new())
    }
}

/// Build worker section without tool information (ToolVisibility::None).
pub fn build_workers_section_no_tools(config: &OrchestrationConfig) -> String {
    let workers_list = config.format_workers_for_prompt();
    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "",
        roster_content: &workers_list,
        closing_line: "Each worker has specialized capabilities. Assign tasks to the most appropriate worker.",
    })
}

/// Build worker section with tool names (ToolVisibility::Summary).
pub fn build_workers_section_with_tools(
    config: &OrchestrationConfig,
    tool_list_limit: ToolListLimit,
    inventory: &ToolInventory,
) -> String {
    let worker_tools = resolve_worker_tools(config, inventory);
    let max_tools = tool_list_limit.get();
    let sections: Vec<String> = config
        .workers
        .iter()
        .map(|(name, config)| {
            let tools = worker_tools.get(name).cloned().unwrap_or_default();
            let tool_list = format_tool_list(&tools, max_tools);

            if tool_list.is_empty() {
                format!(
                    "## {}\n{}\nTools: (none configured — this worker cannot query external systems)",
                    name, config.description
                )
            } else {
                format!("## {}\n{}\nTools: {}", name, config.description, tool_list)
            }
        })
        .collect();

    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "NOTE: Worker names below are role assignments, not callable tool names. Only the tools listed under each worker are MCP tools that workers can execute.\n\n",
        roster_content: &sections.join("\n\n"),
        closing_line: "Assign tasks to the worker whose tools best match the required operations.",
    })
}

/// Build worker section with full tool info (ToolVisibility::Full).
pub fn build_workers_section_with_full_tools(
    config: &OrchestrationConfig,
    tool_list_limit: ToolListLimit,
    vector_stores: &[VectorStoreConfig],
    inventory: &ToolInventory,
) -> String {
    let worker_tools = resolve_worker_tools(config, inventory);
    let tool_descriptions = get_all_tool_descriptions(vector_stores);
    let max_tools = tool_list_limit.get();
    let sections: Vec<String> = config
        .workers
        .iter()
        .map(|(name, config)| {
            let tools = worker_tools.get(name).cloned().unwrap_or_default();

            let tool_details: Vec<String> = tools
                .iter()
                .take(max_tools)
                .map(|t| {
                    if let Some(desc) = tool_descriptions.get(t) {
                        format!("  - {}: {}", t, desc)
                    } else {
                        format!("  - {}", t)
                    }
                })
                .collect();

            let remaining = tools.len().saturating_sub(max_tools);
            let tool_section = if tool_details.is_empty() {
                String::new()
            } else if remaining > 0 {
                format!("{}\n  (+{} more)", tool_details.join("\n"), remaining)
            } else {
                tool_details.join("\n")
            };

            if tool_section.is_empty() {
                format!("## {}\n{}", name, config.description)
            } else {
                format!(
                    "## {}\n{}\nTools:\n{}",
                    name, config.description, tool_section
                )
            }
        })
        .collect();

    crate::templates::render_worker_roster(&crate::templates::WorkerRosterVars {
        header_note: "NOTE: Worker names below are role assignments, not callable tool names. Only the tools listed under each worker are MCP tools that workers can execute.\n\n",
        roster_content: &sections.join("\n\n"),
        closing_line: "Assign tasks to the worker whose tools best match the required operations.",
    })
}

// ============================================================================
// Tool Resolution
// ============================================================================

/// The tool names a runtime MCP backend advertises, which each worker's
/// `mcp_filter` selects from.
///
/// The source reads this list from its MCP manager
/// (`Orchestrator::get_all_tool_names`, orchestrator.rs:2048-2051), which
/// answers with an empty vector when `mcp_manager` is `None`. Carrying it as
/// a parameter rather than a binding inside [`resolve_worker_tools`] is what
/// lets one build serve both runtimes this crate has: the ported corpus,
/// which ran MCP-less and therefore resolves against [`ToolInventory::empty`],
/// and the SSE shim, whose inventory is whatever the sidecar's `tools/list`
/// answered at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolInventory(Vec<String>);

impl ToolInventory {
    /// The inventory of a runtime with no MCP backend attached: no worker
    /// resolves an MCP tool, whatever its `mcp_filter` says.
    ///
    /// This is the corpus path. The golden frames are composed against it,
    /// so they stay byte-identical to the MCP-less aura corpus they were
    /// captured from.
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Read an inventory from the tool names a backend advertises, in the
    /// order it advertised them.
    ///
    /// Order is carried through to the rendered roster, where
    /// [`ToolListLimit`] truncates the tail, so an inventory that reorders
    /// its names changes which tools the coordinator sees under a tight
    /// limit.
    ///
    /// A repeated name is kept once, at its first position, and an empty name
    /// is dropped. Both would otherwise survive into a worker's tool list: a
    /// duplicate spends a [`ToolListLimit`] slot twice, and an empty name
    /// names a tool no `tools/call` could reach. Neither is worth failing a
    /// startup over, so this stays infallible.
    pub fn from_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut kept: Vec<String> = Vec::new();
        for name in names {
            let name = name.into();
            if !name.is_empty() && !kept.iter().any(|seen| seen == &name) {
                kept.push(name);
            }
        }
        Self(kept)
    }

    /// The advertised tool names, in advertisement order.
    pub fn names(&self) -> &[String] {
        &self.0
    }
}

/// Resolve which tools each worker can access based on their mcp_filter.
///
/// Returns a map of worker_name -> Vec<tool_name>.
/// Tools that don't match any worker's filter are omitted.
///
/// # Example
///
/// Given workers:
/// - operations: mcp_filter = ["mezmo_*"]
/// - knowledge: mcp_filter = ["ListKnowledgeBases", "QueryKnowledgeBases"]
///
/// And an inventory of: mezmo_logs, mezmo_pipelines, ListKnowledgeBases,
/// QueryKnowledgeBases
///
/// Returns:
/// - "operations" -> ["mezmo_logs", "mezmo_pipelines"]
/// - "knowledge" -> ["ListKnowledgeBases", "QueryKnowledgeBases"]
///
/// `vector_search_{store}` tools from `worker_config.vector_stores` are
/// appended to every worker's list regardless of the inventory: they are
/// config-mirror tools, not MCP tools. Source anchor:
/// orchestrator.rs:2141-2171.
pub fn resolve_worker_tools(
    config: &OrchestrationConfig,
    inventory: &ToolInventory,
) -> HashMap<String, Vec<String>> {
    let all_tools = inventory.names();
    let mut worker_tools = HashMap::new();

    for (worker_name, worker_config) in &config.workers {
        // Match MCP tools via mcp_filter (empty = all MCP tools for backwards compatibility)
        let mut matching_tools: Vec<String> = if worker_config.mcp_filter.is_empty() {
            all_tools.to_vec()
        } else {
            all_tools
                .iter()
                .filter(|tool_name| {
                    worker_config
                        .mcp_filter
                        .iter()
                        .any(|pattern| crate::config::glob_match(pattern, tool_name))
                })
                .cloned()
                .collect()
        };

        // Add vector store tools based on explicit vector_stores assignment
        for store_name in &worker_config.vector_stores {
            matching_tools.push(format!("vector_search_{}", store_name));
        }

        worker_tools.insert(worker_name.clone(), matching_tools);
    }

    worker_tools
}

/// Format a list of tool names with truncation.
///
/// If the list exceeds `max`, truncates and appends "(+N more)".
pub fn format_tool_list(tools: &[String], max: usize) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let display_tools: Vec<&str> = tools.iter().take(max).map(|s| s.as_str()).collect();
    let remaining = tools.len().saturating_sub(max);

    if remaining > 0 {
        format!("{} (+{} more)", display_tools.join(", "), remaining)
    } else {
        display_tools.join(", ")
    }
}

/// Get tool descriptions for full visibility mode.
///
/// Returns a map of tool_name -> description.
/// Used when `tools_in_planning = "full"`.
///
/// MCP-less substitution: the source collects descriptions from the MCP
/// manager (streamable HTTP, SSE, STDIO tools) when `self.mcp_manager` is
/// `Some` (orchestrator.rs:2177-2208). That block is omitted here, so only
/// vector store descriptions from the config mirror are collected and an MCP
/// tool renders under `ToolVisibility::Full` as a bare name. The
/// [`ToolInventory`] carries names only; wiring descriptions through it is
/// the follow-up that closes this gap. Source anchor:
/// orchestrator.rs:2177-2222.
pub fn get_all_tool_descriptions(vector_stores: &[VectorStoreConfig]) -> HashMap<String, String> {
    let mut descriptions = HashMap::new();

    // Collect from vector stores (context_prefix becomes the description)
    for store in vector_stores {
        let tool_name = format!("vector_search_{}", store.name);
        let description = store
            .context_prefix
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Search the {} knowledge base", store.name));
        descriptions.insert(tool_name, description);
    }

    descriptions
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Plan, PlanningResponse, StepInput, Task};

    // Adapted from frame_validation_tests.rs:199 — tests compact_decision_turn
    // fallback tiers.
    #[test]
    fn test_compact_decision_turn_fallback_tiers() {
        let degenerate = PlanningResponse::StepsPlan {
            goal: "Audit ingest".to_string(),
            steps: vec![StepInput::LeafTask {
                task: "Enumerate pods".to_string(),
                worker: None,
            }],
            routing_rationale: "  ".to_string(),
            planning_summary: String::new(),
        };
        assert_eq!(
            compact_decision_turn(&degenerate, "I will plan the pod audit now."),
            "I will plan the pod audit now.",
            "model-text tier records the streamed narration"
        );
        assert_eq!(
            compact_decision_turn(&degenerate, " \n"),
            "StepsPlan",
            "variant-name tier is the structurally task-body-free floor"
        );
    }

    // Adapted from frame_validation_tests.rs:557 — tests build_task_context
    // returns None when no completed ancestors exist.
    #[test]
    fn test_build_task_context_empty_ancestry_returns_none() {
        let mut plan = Plan::new("Standalone task");
        let t0 = Task::new(0, "unstarted predecessor", "Not complete");
        let mut t1 = Task::new(1, "current task", "Does something");
        t1.dependencies = vec![0];
        plan.add_task(t0);
        plan.add_task(t1);

        assert!(
            build_task_context(&plan, 1).is_none(),
            "no completed ancestors means no frame"
        );
    }

    // Adapted from frame_validation_tests.rs — tests build_task_context
    // returns Some when a completed ancestor exists.
    #[test]
    fn test_build_task_context_completed_ancestor_returns_some() {
        let mut plan = Plan::new("Chain");
        let mut t0 = Task::new(0, "root task", "First task");
        t0.complete("root result".to_string());
        let mut t1 = Task::new(1, "child task", "Second task");
        t1.dependencies = vec![0];
        plan.add_task(t0);
        plan.add_task(t1);

        let context = build_task_context(&plan, 1);
        assert!(
            context.is_some(),
            "a completed direct dependency means a frame is rendered"
        );
        let context = context.unwrap();
        assert!(
            context.contains("root result"),
            "the frame includes the completed ancestor's result text"
        );
    }

    // Adapted from orchestrator.rs:5020 — tests resolve_worker_tools includes
    // vector_search tools for workers with assigned vector stores (MCP-less).
    #[test]
    fn test_resolve_worker_tools_includes_vector_stores() {
        use crate::config::{OrchestrationConfig, WorkerConfig};

        let mut config = OrchestrationConfig::default();
        config.workers.insert(
            "documentation".to_string(),
            WorkerConfig {
                description: "docs".to_string(),
                preamble: String::new(),
                mcp_filter: vec![],
                vector_stores: vec!["docs".to_string()],
                turn_depth: None,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        );
        config.workers.insert(
            "operations".to_string(),
            WorkerConfig {
                description: "ops".to_string(),
                preamble: String::new(),
                mcp_filter: vec![],
                vector_stores: vec![],
                turn_depth: None,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        );

        let worker_tools = resolve_worker_tools(&config, &ToolInventory::empty());

        let doc_tools = worker_tools.get("documentation").unwrap();
        assert_eq!(doc_tools.len(), 1);
        assert!(doc_tools.contains(&"vector_search_docs".to_string()));

        let ops_tools = worker_tools.get("operations").unwrap();
        assert!(ops_tools.is_empty());
    }

    /// A worker's `mcp_filter` selects from the inventory the runtime
    /// advertises, so a non-empty inventory reaches the roster while an
    /// unmatched worker still resolves to nothing.
    ///
    /// This is the S74 defect in miniature: the shim's sidecar advertises
    /// `keystrokes` and `capture-pane`, and before the inventory became an
    /// input every worker resolved to an empty tool list no matter what its
    /// filter said.
    #[test]
    fn resolve_worker_tools_filters_a_non_empty_inventory() {
        use crate::config::{OrchestrationConfig, WorkerConfig};

        let worker = |mcp_filter: Vec<String>| WorkerConfig {
            description: "worker".to_string(),
            preamble: String::new(),
            mcp_filter,
            vector_stores: vec![],
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        };

        let mut config = OrchestrationConfig::default();
        config.workers.insert(
            "operator".to_string(),
            worker(vec!["keystrokes".to_string(), "capture-pane".to_string()]),
        );
        config
            .workers
            .insert("globbed".to_string(), worker(vec!["mezmo_*".to_string()]));
        config
            .workers
            .insert("unfiltered".to_string(), worker(vec![]));

        let inventory = ToolInventory::from_names([
            "keystrokes",
            "capture-pane",
            "mezmo_logs",
            "mezmo_pipelines",
        ]);
        let worker_tools = resolve_worker_tools(&config, &inventory);

        assert_eq!(
            worker_tools.get("operator").unwrap(),
            &vec!["keystrokes".to_string(), "capture-pane".to_string()],
            "an exact-name filter resolves exactly its two advertised tools"
        );
        assert_eq!(
            worker_tools.get("globbed").unwrap(),
            &vec!["mezmo_logs".to_string(), "mezmo_pipelines".to_string()],
            "a glob filter resolves every advertised tool it matches"
        );
        assert_eq!(
            worker_tools.get("unfiltered").unwrap(),
            inventory.names(),
            "an empty filter means every advertised tool, in advertisement order"
        );
    }

    /// The empty inventory is what keeps the MCP-less corpus byte-identical:
    /// every worker resolves to no MCP tool whatever its filter says.
    #[test]
    fn empty_inventory_resolves_no_mcp_tools_whatever_the_filter() {
        use crate::config::{OrchestrationConfig, WorkerConfig};

        let mut config = OrchestrationConfig::default();
        config.workers.insert(
            "operator".to_string(),
            WorkerConfig {
                description: "worker".to_string(),
                preamble: String::new(),
                mcp_filter: vec!["keystrokes".to_string()],
                vector_stores: vec![],
                turn_depth: None,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        );

        let worker_tools = resolve_worker_tools(&config, &ToolInventory::empty());
        assert!(worker_tools.get("operator").unwrap().is_empty());
    }

    /// A repeated advertised name is kept once at its first position and an
    /// empty name is dropped, so neither reaches a worker's tool list or
    /// spends a `ToolListLimit` slot.
    #[test]
    fn from_names_deduplicates_in_first_seen_order_and_drops_empty_names() {
        let inventory = ToolInventory::from_names([
            "keystrokes",
            "",
            "capture-pane",
            "keystrokes",
            "capture-pane",
        ]);

        assert_eq!(
            inventory.names(),
            &["keystrokes".to_string(), "capture-pane".to_string()],
            "first-seen order survives the deduplication"
        );
    }

    #[test]
    fn test_format_tool_list_empty() {
        assert_eq!(format_tool_list(&[], 5), "");
    }

    #[test]
    fn test_format_tool_list_under_limit() {
        let tools = vec!["a".to_string(), "b".to_string()];
        assert_eq!(format_tool_list(&tools, 5), "a, b");
    }

    #[test]
    fn test_format_tool_list_over_limit() {
        let tools = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(format_tool_list(&tools, 2), "a, b (+1 more)");
    }
}
