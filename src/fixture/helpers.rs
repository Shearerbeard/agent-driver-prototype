//! Helper closures ported from aura: `render_skill_catalog`,
//! `build_session_context` + `render_task_summary`, and the
//! `SCRATCHPAD_PREAMBLE` constant. These are the render-only paths the
//! fixture builders call with in-memory data; no rig, MCP, or file IO.

use crate::persistence::{RunManifest, TaskSummary, ToolOutcome, ToolTraceEntry};
use crate::templates::{SessionHistoryVars, render_session_history};
use crate::types::TaskStatus;

/// Scratchpad usage instructions appended to worker preambles. Ported
/// verbatim from `crates/aura/src/scratchpad/mod.rs`.
pub const SCRATCHPAD_PREAMBLE: &str = r#"
## Scratchpad Tools

Some tool outputs are too large for the context window and have been saved to scratchpad files.
When you see a `[scratchpad: ...]` message instead of direct output, use these tools to explore:

1. **schema** — See the structure with line ranges. Works on JSON (keys, types) and Markdown (sections, keys). Start here.
2. **item_schema** — See all unique keys across items in a JSON array (e.g., `item_schema(file, 'results')`).
3. **head** — Preview the first N lines.
4. **grep** — Search for specific content with regex.
5. **get_in** — Extract a value at a nested JSON path (e.g., `results.0.title`). For large string values, use `offset` and `limit` to paginate by line.
6. **iterate_over** — Extract selected fields from every item in a JSON array (e.g., `iterate_over(file, 'results', 'id,title')`).
7. **slice** — Extract a specific line range.
8. **read** — Read the entire file (WARNING: may be large, prefer targeted tools).

**Companion files**: Large structured string values inside JSON (escaped JSON → `.json`, markdown → `.md`) are automatically extracted to companion files. Use `schema` on the companion file to see its structure, then `slice` or `grep` to explore specific sections.

**Strategy**: Use `schema` first to understand structure. For JSON arrays, use `item_schema` to discover fields, then `iterate_over` to extract them. For companion `.md` files, use `schema` to see sections, then `slice` to extract a specific section by line range. Use `get_in` or `grep` for targeted lookups. Avoid `read` unless the file is small.
"#;

/// Render the system-prompt skill catalog, or `None` when no skills are
/// configured. Ported verbatim from `crates/aura/src/skill_tool.rs`.
pub fn render_skill_catalog(skills: &[crate::config::SkillConfig]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut catalog = String::from(
        "\n\nAvailable skills (use the `load_skill` tool to load before answering):\n",
    );
    for skill in skills {
        catalog.push_str(&format!("- {}: {}\n", skill.name, skill.description));
    }
    Some(catalog)
}

/// Build a session context string from prior run manifests. Ported verbatim
/// from `crates/aura/src/orchestration/persistence.rs::build_session_context`.
pub fn build_session_context(manifests: &[RunManifest]) -> String {
    if manifests.is_empty() {
        return String::new();
    }

    let mut turn_entries = String::new();

    // Manifests are sorted most-recent-first; number turns chronologically
    for (i, manifest) in manifests.iter().rev().enumerate() {
        let turn_num = i + 1;
        let status = format!("{:?}", manifest.status);

        turn_entries.push_str(&format!(
            "### Turn {} ({}) — {}\n",
            turn_num, manifest.timestamp, status
        ));
        turn_entries.push_str(&format!("Goal: \"{}\"\n", manifest.goal));

        if let Some(outcome) = &manifest.outcome {
            turn_entries.push_str(&format!("Outcome: {}\n", outcome));
        }

        if let Some(summary) = &manifest.response_summary {
            turn_entries.push_str(&format!("Response: \"{}\"\n", summary));
        }

        if !manifest.task_summaries.is_empty() {
            turn_entries.push_str("Tasks:\n");
            for task in &manifest.task_summaries {
                render_task_summary(task, &mut turn_entries);
            }
        }

        let has_artifacts = manifest
            .task_summaries
            .iter()
            .any(|t| !t.artifacts.is_empty());
        if has_artifacts {
            turn_entries.push_str(&format!(
                "  (use run_id=\"{}\" with read_artifact for cross-run access)\n",
                manifest.run_id
            ));
        }

        turn_entries.push('\n');
    }

    render_session_history(&SessionHistoryVars {
        turn_count: &manifests.len().to_string(),
        turn_entries: turn_entries.trim_end(),
    })
}

/// Render one task summary into the session context. Ported verbatim from
/// `crates/aura/src/orchestration/persistence.rs::render_task_summary`.
fn render_task_summary(task: &TaskSummary, out: &mut String) {
    let worker = task.worker.as_deref().unwrap_or("unassigned");

    match task.status {
        TaskStatus::Complete => {
            let confidence_tag = task
                .confidence
                .as_deref()
                .map(|c| format!(" ({})", c))
                .unwrap_or_default();
            out.push_str(&format!(
                "  Task {} [{}] — Complete{}\n",
                task.task_id, worker, confidence_tag
            ));
            out.push_str(&format!("    \"{}\"\n", task.description));
            if let Some(preview) = &task.result_preview {
                out.push_str(&format!("    Summary: \"{}\"\n", preview));
            }
        }
        TaskStatus::Failed => {
            let category_tag = task
                .failure_category
                .as_ref()
                .map(|c| format!(" ({})", c))
                .unwrap_or_default();
            out.push_str(&format!(
                "  Task {} [{}] — FAILED{}\n",
                task.task_id, worker, category_tag
            ));
            out.push_str(&format!("    \"{}\"\n", task.description));
            if let Some(error) = &task.error {
                out.push_str(&format!("    Error: {}\n", error));
            }
            if let Some(ctx) = &task.error_context {
                if let Some(tool) = &ctx.last_tool_call {
                    out.push_str(&format!("    Last tool: {}\n", tool));
                }
                if let Some(partial) = &ctx.partial_result {
                    out.push_str(&format!("    Partial progress: {}\n", partial));
                }
            }
        }
        _ => {
            out.push_str(&format!(
                "  Task {} [{}] — {}\n",
                task.task_id, worker, task.status
            ));
            out.push_str(&format!("    \"{}\"\n", task.description));
        }
    }

    if !task.tool_trace.is_empty() {
        let chain: Vec<String> = task
            .tool_trace
            .iter()
            .map(|t| {
                let duration = format!("{:.1}s", t.duration_ms as f64 / 1000.0);
                match &t.outcome {
                    ToolOutcome::Success { .. } => format!("{} ({})", t.tool, duration),
                    ToolOutcome::Error { message } => {
                        format!("{} (FAILED: {})", t.tool, message)
                    }
                }
            })
            .collect();
        out.push_str(&format!("    Tool chain: {}\n", chain.join(" → ")));
    }

    if !task.artifacts.is_empty() {
        let listing: Vec<String> = task
            .artifacts
            .iter()
            .map(|a| format!("{} ({}B)", a.filename, a.size_bytes))
            .collect();
        out.push_str(&format!("    Artifacts: {}\n", listing.join(", ")));
    }
}

// Silence unused import warning for ToolTraceEntry (used only via ToolOutcome
// in render_task_summary, which the compiler sees through the alias).
#[allow(unused_imports)]
use ToolTraceEntry as _ToolTraceEntryAlias;
