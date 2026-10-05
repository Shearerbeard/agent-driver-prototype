//! Human digest renderer for a validated workflow proposal.
//!
//! The digest is what an approver sees: the workflow's goal, each step's
//! intent, the tool it calls, and how it can be undone. It is also the
//! tool observation the coordinator model receives, so it renders
//! exactly what validation vetted — literal arguments as their wire
//! values, references as `step.export` with any declared bounds — and
//! never invents what the spec does not declare.

use std::fmt::Write as _;

use serde_json::Value;

use super::plan::{ArgValue, Bounds, ValidatedWorkflowSpec, WorkflowStep};

/// Render a validated workflow as a human-readable digest.
///
/// The returned string is the tool observation the coordinator receives;
/// it must name the goal, every step, and any rollback so the approver can
/// authorize exactly what validation vetted.
///
/// Deterministic: the same spec renders to the same string. Steps render
/// in declaration order, exports in name order (the `BTreeMap`), and
/// literal object keys in serde_json's sorted order.
pub fn render_digest(validated: &ValidatedWorkflowSpec) -> String {
    let spec = validated.spec();
    let mut digest = format!("Workflow proposal: {}\n", spec.goal);
    for (position, step) in spec.steps.iter().enumerate() {
        digest.push('\n');
        render_step(&mut digest, position + 1, step);
    }
    digest
}

/// Render one step's block: its id, dependencies, tool, arguments,
/// declared exports, and undo. A dependency line appears only when
/// dependencies are declared; an empty exports map renders as an honest
/// `none` rather than an omission, and a step without a rollback as
/// `nothing to undo` — never invented.
fn render_step(out: &mut String, number: usize, step: &WorkflowStep) {
    let _ = writeln!(out, "Step {number}: {}", step.id);
    if !step.dependencies.is_empty() {
        let dependencies = step
            .dependencies
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  depends on: {dependencies}");
    }
    let _ = writeln!(out, "  tool: {}", step.tool);
    let _ = writeln!(out, "  args: {}", render_arg_node(&step.args));
    if step.exports.is_empty() {
        let _ = writeln!(out, "  exports: none");
    } else {
        let exports = step
            .exports
            .iter()
            .map(|(name, path)| format!("{name} = {path}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  exports: {exports}");
    }
    match &step.rollback {
        Some(rollback) => {
            let _ = writeln!(
                out,
                "  undo: {} args {}",
                rollback.tool,
                render_arg_node(&rollback.args)
            );
        }
        None => {
            let _ = writeln!(out, "  undo: nothing to undo (rollback: null)");
        }
    }
}

/// Render one argument-tree node.
///
/// Reference nodes render as `step.export` — plus their declared bounds,
/// sides in `min`, `max` order — so the approver reads what will be
/// bound, not the `$from` wire envelope. Literals render in a
/// JSON-literal style: object keys bare, scalar values in their compact
/// JSON wire form (`{app: "payments", replicas: 6}`), recursing through
/// composite nodes so a reference bound at any depth renders in
/// reference form; this is the same walk `validate` rule 3 runs to find
/// them.
///
/// The `Err` arm is unreachable through `validate`: rule 3 classifies
/// every node of every argument tree before the spec is constructible.
/// A malformed node renders as its wire JSON rather than panicking.
fn render_arg_node(node: &Value) -> String {
    match ArgValue::parse(node) {
        Ok(ArgValue::Reference { from }) => from.to_string(),
        Ok(ArgValue::BoundedReference { from, bounds }) => {
            format!("{from}{}", render_bounds(&bounds))
        }
        Ok(ArgValue::Literal(_)) | Err(_) => match node {
            Value::Object(entries) => {
                let fields = entries
                    .iter()
                    .map(|(key, value)| format!("{key}: {}", render_arg_node(value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{fields}}}")
            }
            Value::Array(items) => {
                let elements = items
                    .iter()
                    .map(render_arg_node)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{elements}]")
            }
            scalar => scalar.to_string(),
        },
    }
}

/// Render a reference's declared resolve-time envelope: only the sides
/// the model declared. One-sided bounds are legal (panel ruling, R2), so
/// either side may be absent; `Bounds` forbids both absent, so the
/// brackets are never empty for a validated spec.
fn render_bounds(bounds: &Bounds) -> String {
    let mut sides = Vec::new();
    if let Some(min) = bounds.min() {
        sides.push(format!("min {min}"));
    }
    if let Some(max) = bounds.max() {
        sides.push(format!("max {max}"));
    }
    format!(" [{}]", sides.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_client::{SidecarTool, SidecarToolName};
    use crate::workflow::plan::WorkflowSpec;
    use serde_json::json;

    /// A permissive inventory: the renderer only ever sees validated
    /// specs, so the schemas need only admit the demo literals (object
    /// arguments against an object schema; reference nodes are
    /// structural at propose time).
    fn tool(name: &str) -> SidecarTool {
        SidecarTool::new(
            SidecarToolName::new(name).unwrap(),
            String::new(),
            json!({"type": "object"}),
        )
    }

    /// The W6 demo shape: a read step exporting a `$.` path, then a
    /// mutating step whose rollback binds that export through a
    /// `$from` reference carrying both bounds.
    fn demo_spec() -> ValidatedWorkflowSpec {
        serde_json::from_value::<WorkflowSpec>(json!({
            "goal": "Mitigate the payments db-primary connection pool exhaustion",
            "steps": [
                {
                    "id": "state",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "payments"},
                    "exports": {"current_replicas": "$.deployment.replicas"},
                    "rollback": null
                },
                {
                    "id": "scale",
                    "dependencies": ["state"],
                    "tool": "ops_scale_app",
                    "args": {"app": "payments", "replicas": 6},
                    "exports": {},
                    "rollback": {
                        "tool": "ops_scale_app",
                        "args": {"app": "payments",
                                 "replicas": {"$from": "state.current_replicas", "min": 1, "max": 20}}
                    }
                }
            ]
        }))
        .unwrap()
        .validate(&[tool("ops_get_cluster_state"), tool("ops_scale_app")])
        .unwrap()
    }

    #[test]
    fn the_digest_renders_goal_steps_exports_and_the_rollback_reference() {
        let digest = render_digest(&demo_spec());
        assert!(
            digest.contains(
                "Workflow proposal: Mitigate the payments db-primary connection pool exhaustion"
            ),
            "{digest}"
        );
        assert!(digest.contains("Step 1: state\n"), "{digest}");
        assert!(digest.contains("Step 2: scale\n"), "{digest}");
        assert!(digest.contains("depends on: state\n"), "{digest}");
        assert!(
            digest.contains("exports: current_replicas = $.deployment.replicas\n"),
            "{digest}"
        );
        assert!(
            digest.contains("undo: nothing to undo (rollback: null)\n"),
            "{digest}"
        );
        // Literals render as their wire values (keys bare, string
        // values JSON-quoted) ...
        assert!(digest.contains("\"payments\""), "{digest}");
        assert!(digest.contains("replicas: 6"), "{digest}");
        // ... and the rollback's reference renders in step.export form
        // with its declared envelope, never the $from wire syntax.
        assert!(
            digest.contains(
                "undo: ops_scale_app args {app: \"payments\", \
                 replicas: state.current_replicas [min 1, max 20]}\n"
            ),
            "{digest}"
        );
        assert!(!digest.contains("$from"), "{digest}");
    }

    #[test]
    fn absent_declarations_are_omitted_or_declared_honestly_never_invented() {
        let digest = render_digest(&demo_spec());
        // `state` declares no dependencies: no depends-on line at all.
        // `scale` declares no exports: an honest `none`, not an omission.
        assert_eq!(digest.matches("depends on:").count(), 1, "{digest}");
        assert_eq!(digest.matches("exports: none").count(), 1, "{digest}");
        assert_eq!(digest.matches("nothing to undo").count(), 1, "{digest}");
    }

    #[test]
    fn the_digest_is_deterministic() {
        assert_eq!(render_digest(&demo_spec()), render_digest(&demo_spec()));
    }
}
