//! Derived tool truth for every surface where a coordinator prompt speaks
//! (S114 decision 4).
//!
//! Both sides of every assertion are derived: the factory
//! ([`coordinator_tool_definitions`](crate::coordinator_loop::coordinator_tool_definitions))
//! is the single source for the names and definitions a coordinator may
//! claim, and no test hardcodes the four. The checks are section-scoped:
//! they scan the coordinator-owned template sources and framework-owned
//! generated sections rendered with inert inputs, never whole `.rs` files.
//! The inert inputs carry none of the banned vocabulary, so a banned token
//! found in rendered output belongs to the framework, not the caller.
//!
//! Scope is coordinator-only: worker surfaces legitimately keep
//! `read_artifact` and `load_skill`, and the one worker-directed
//! `read_artifact` mention (`guidance::RESULT_FORWARDING`, injected into
//! failure continuations so a worker can pull a spilled artifact) is
//! exempt. No fixture continuation body is scanned for that reason; the
//! continuation surface is covered by its template and its decision-point
//! list.
//!
//! The coordinator templates still describe the retired router world, so
//! these assertions are RED until the migration lands and re-goldens the
//! corpus; the `planning_loop_prompt.md` control assertions are green
//! throughout.

use std::collections::{BTreeSet, HashMap};

use agent_driver_rs::tool::ToolDefinition as PinToolDefinition;

use crate::bounding::ToolListLimit;
use crate::config::{
    OrchestrationConfig, SkillConfig, SkillName, ToolVisibility, VectorStoreConfig, WorkerConfig,
};
use crate::config_builders::build_coordinator_preamble;
use crate::context::PinnedGoal;
use crate::coordinator_loop::{
    CreatePlanArgs, WorkerRoster, WorkerSections, coordinator_tool_definitions,
};
use crate::fixture::{
    CoordinatorCall, CoordinatorScenario, CoordinatorToolConfig, HistoryTools, PreambleFixture,
    ReconTools, SessionHistoryFixture, WorkerRosterFixture, coordinator_envelope,
};
use crate::message::ToolDefinition as MirrorToolDefinition;
use crate::persistence::{
    ArtifactEntry, ArtifactKind, RoutingMode, RunManifest, RunStatus, TaskSummary,
};
use crate::producers::ToolInventory;
use crate::templates::{
    CONTINUATION_PROMPT_TEMPLATE, ContinuationVars, ORCHESTRATOR_PREAMBLE_TEMPLATE,
    PLANNING_LOOP_PROMPT_TEMPLATE, PLANNING_PROMPT_TEMPLATE, PlanningLoopVars, PlanningVars,
    SESSION_HISTORY_TEMPLATE, render_continuation_prompt, render_planning_loop_prompt,
    render_planning_prompt,
};
use crate::types::{StepInput, TaskStatus};

// ============================================================================
// The banned vocabulary (coordinator surfaces only)
// ============================================================================

const BANNED_TOOL_NAMES: [&str; 6] = [
    "respond_directly",
    "request_clarification",
    "read_artifact",
    "list_prior_runs",
    "list_tools",
    "inspect_tool_params",
];
const BANNED_TOOL_PHRASE: &str = "routing tool";
const BANNED_TOOL_PREFIX: &str = "vector_search_";

fn banned_hits(text: &str) -> Vec<&'static str> {
    let mut hits = Vec::new();
    for name in BANNED_TOOL_NAMES {
        if text.contains(name) {
            hits.push(name);
        }
    }
    for token in [BANNED_TOOL_PHRASE, BANNED_TOOL_PREFIX] {
        if text.contains(token) {
            hits.push(token);
        }
    }
    hits
}

fn assert_no_banned_tokens(surface: &str, text: &str) {
    let hits = banned_hits(text);
    assert!(
        hits.is_empty(),
        "{surface} carries retired coordinator tool vocabulary {hits:?}"
    );
}

// ============================================================================
// Inert fixture inputs (carry no banned vocabulary)
// ============================================================================

const INERT_QUERY: &str = "Inert query for tool-truth checks.";
const INERT_PLAYBOOK: &str = "Inert playbook for tool-truth checks.";

fn inert_roster_config(visibility: ToolVisibility) -> OrchestrationConfig {
    OrchestrationConfig {
        enabled: true,
        workers: HashMap::from([(
            "inert_worker".to_owned(),
            WorkerConfig {
                description: "Inert worker for tool-truth checks.".to_owned(),
                preamble: String::new(),
                mcp_filter: Vec::new(),
                vector_stores: Vec::new(),
                turn_depth: None,
                llm: None,
                scratchpad: None,
                skills: None,
            },
        )]),
        tools_in_planning: visibility,
        ..OrchestrationConfig::default()
    }
}

fn inert_goal() -> PinnedGoal {
    PinnedGoal::new(INERT_QUERY).expect("inert query is non-empty")
}

fn inert_preamble(tools: CoordinatorToolConfig) -> PreambleFixture {
    PreambleFixture {
        playbook: INERT_PLAYBOOK.to_owned(),
        tools,
        skills: Vec::new(),
        vector_stores: Vec::new(),
        session_history: None,
    }
}

fn inert_skill() -> SkillConfig {
    SkillConfig {
        name: SkillName::new("inert-skill").expect("valid inert skill name"),
        description: "Inert skill for tool-truth checks.".to_owned(),
        path: std::path::PathBuf::from("/fixtures/skills/inert-skill"),
    }
}

fn inert_manifest() -> RunManifest {
    RunManifest {
        run_id: "run-inert-0001".to_owned(),
        session_id: Some("tool-truth".to_owned()),
        timestamp: "2026-09-29T00:00:00Z".to_owned(),
        goal: "Inert prior goal".to_owned(),
        status: RunStatus::Success,
        iterations: 1,
        routing_mode: Some(RoutingMode::Orchestrated),
        outcome: Some("1/1 tasks completed".to_owned()),
        response_summary: None,
        task_summaries: vec![TaskSummary {
            task_id: 0,
            description: "Inert prior task".to_owned(),
            status: TaskStatus::Complete,
            worker: Some("inert_worker".to_owned()),
            result_preview: Some("Inert prior result".to_owned()),
            confidence: Some("high".to_owned()),
            failure_category: None,
            error: None,
            error_context: None,
            tool_trace: Vec::new(),
            artifacts: vec![ArtifactEntry {
                filename: "task-0-inert_worker-iter-1-result.txt".to_owned(),
                size_bytes: 3200,
                kind: ArtifactKind::Result,
            }],
        }],
        artifact_paths: Vec::new(),
    }
}

/// The coordinator scenarios the envelope assertions compose: one per
/// preamble-append axis. The attached tool set and the composed preamble do
/// not branch on the call kind, so initial calls cover the envelope path.
fn coordinator_scenarios() -> Vec<CoordinatorScenario> {
    let build = |preamble: PreambleFixture, visibility: ToolVisibility| {
        CoordinatorScenario::new(
            preamble,
            inert_goal(),
            WorkerRosterFixture::new(inert_roster_config(visibility), Vec::new()),
            CoordinatorCall::Initial,
        )
        .expect("inert scenarios are production-reachable")
    };

    let mut scenarios = vec![build(
        inert_preamble(CoordinatorToolConfig {
            recon: ReconTools::Excluded,
            history: HistoryTools::Excluded,
        }),
        ToolVisibility::Summary,
    )];

    scenarios.push(build(
        inert_preamble(CoordinatorToolConfig {
            recon: ReconTools::Included,
            history: HistoryTools::Included,
        }),
        ToolVisibility::None,
    ));

    let mut skills = inert_preamble(CoordinatorToolConfig {
        recon: ReconTools::Excluded,
        history: HistoryTools::Excluded,
    });
    skills.skills = vec![inert_skill()];
    scenarios.push(build(skills, ToolVisibility::Summary));

    let mut vector = inert_preamble(CoordinatorToolConfig {
        recon: ReconTools::Excluded,
        history: HistoryTools::Excluded,
    });
    vector.vector_stores = vec![VectorStoreConfig::new(
        "inert_store",
        Some("Inert vector store for tool-truth checks."),
    )];
    scenarios.push(build(vector, ToolVisibility::Summary));

    let mut session = inert_preamble(CoordinatorToolConfig {
        recon: ReconTools::Excluded,
        history: HistoryTools::Excluded,
    });
    session.session_history =
        Some(SessionHistoryFixture::new(vec![inert_manifest()]).expect("one inert prior manifest"));
    scenarios.push(build(session, ToolVisibility::Summary));

    scenarios
}

/// Worker sections for a scenario, built the way the live loop builds them.
fn worker_sections_for(scenario: &CoordinatorScenario) -> WorkerSections {
    let config = scenario.roster().config();
    let roster = WorkerRoster::from_config(
        config,
        ToolListLimit::new(config.max_tools_per_worker),
        scenario.roster().vector_catalog(),
        &ToolInventory::empty(),
    )
    .expect("inert roster configs parse");
    WorkerSections::from_roster(roster)
}

fn factory_names(sections: &WorkerSections) -> BTreeSet<String> {
    coordinator_tool_definitions(sections)
        .iter()
        .map(|definition| definition.name.as_str().to_owned())
        .collect()
}

/// Project a native tool definition onto the fixture's wire-mirror type.
///
/// The mirror carries exactly the three wire fields (name, description,
/// schema); the native `source` provenance field has no mirror counterpart
/// and is deliberately dropped.
fn mirror_definition(definition: &PinToolDefinition) -> MirrorToolDefinition {
    MirrorToolDefinition {
        name: definition.name.as_str().to_owned(),
        description: definition.description.clone(),
        parameters: definition.input_schema.to_value(),
    }
}

// ============================================================================
// Section extraction over rendered coordinator text
// ============================================================================

fn section_between<'a>(text: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let start = text
        .find(start_marker)
        .unwrap_or_else(|| panic!("{start_marker:?} must open the scanned section"));
    let end = text[start..]
        .find(end_marker)
        .map_or(text.len(), |offset| start + offset);
    &text[start..end]
}

fn section_from<'a>(text: &'a str, start_marker: &str) -> &'a str {
    let start = text
        .find(start_marker)
        .unwrap_or_else(|| panic!("{start_marker:?} must open the scanned section"));
    &text[start..]
}

fn backticked_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let Some(close) = rest[open + 1..].find('`') else {
            break;
        };
        let token = &rest[open + 1..open + 1 + close];
        if !token.is_empty() && !token.chars().any(char::is_whitespace) {
            tokens.insert(token.to_owned());
        }
        rest = &rest[open + 2 + close..];
    }
    tokens
}

fn numbered_tool_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| {
            let (index, rest) = line.trim().split_once(". ")?;
            if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            rest.strip_prefix("**")
                .and_then(|rest| rest.split_once("**"))
                .map(|(name, _)| name.to_owned())
        })
        .collect()
}

/// The decision-point list heads: `- `name`` lines between the
/// "decision point" line and the synthesis-rules block.
fn decision_point_heads(text: &str) -> Vec<String> {
    let mut heads = Vec::new();
    let mut in_decision_block = false;
    for line in text.lines() {
        if !in_decision_block {
            in_decision_block = line.contains("decision point");
            continue;
        }
        if line.starts_with("IMPORTANT") {
            break;
        }
        if let Some(head) = line
            .trim()
            .strip_prefix("- `")
            .and_then(|rest| rest.split_once('`'))
            .map(|(head, _)| head.to_owned())
        {
            heads.push(head);
        }
    }
    heads
}

fn fenced_json_blocks(template: &str) -> Vec<serde_json::Value> {
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_fence = false;
    for line in template.lines() {
        if line.trim() == "```json" {
            in_fence = true;
            current.clear();
        } else if in_fence && line.trim() == "```" {
            in_fence = false;
            let block = serde_json::from_str::<serde_json::Value>(&current.join("\n"))
                .unwrap_or_else(|error| panic!("fenced example must be valid JSON: {error}"));
            blocks.push(block);
        } else if in_fence {
            current.push(line);
        }
    }
    blocks
}

/// The create_plan example blocks: fenced JSON objects carrying a goal and
/// steps.
fn create_plan_examples() -> Vec<serde_json::Value> {
    fenced_json_blocks(ORCHESTRATOR_PREAMBLE_TEMPLATE)
        .into_iter()
        .filter(|block| {
            block
                .as_object()
                .is_some_and(|object| object.contains_key("goal") && object.contains_key("steps"))
        })
        .collect()
}

fn steps_name_a_worker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.contains_key("worker") || object.values().any(steps_name_a_worker)
        }
        serde_json::Value::Array(items) => items.iter().any(steps_name_a_worker),
        _ => false,
    }
}

// ============================================================================
// (a)(i) Ban assertions against the coordinator-owned template sources
// ============================================================================

#[test]
fn orchestrator_preamble_template_names_no_retired_tool() {
    assert_no_banned_tokens("orchestrator_preamble.md", ORCHESTRATOR_PREAMBLE_TEMPLATE);
}

#[test]
fn planning_prompt_template_names_no_retired_tool() {
    assert_no_banned_tokens("planning_prompt.md", PLANNING_PROMPT_TEMPLATE);
}

#[test]
fn continuation_prompt_template_names_no_retired_tool() {
    assert_no_banned_tokens("continuation_prompt.md", CONTINUATION_PROMPT_TEMPLATE);
}

#[test]
fn session_history_template_names_no_retired_tool() {
    assert_no_banned_tokens("session_history.md", SESSION_HISTORY_TEMPLATE);
}

// ============================================================================
// (e) Control: the already-truthful loop surface stays clean
// ============================================================================

#[test]
fn planning_loop_prompt_control_names_no_retired_tool() {
    assert_no_banned_tokens(
        "planning_loop_prompt.md (control)",
        PLANNING_LOOP_PROMPT_TEMPLATE,
    );
}

// ============================================================================
// (a)(ii) Ban assertions against framework-owned generated sections,
// rendered in isolation with inert inputs
// ============================================================================

#[test]
fn preamble_tools_section_names_no_retired_tool() {
    for (recon, history) in [(true, true), (true, false), (false, true), (false, false)] {
        let preamble = build_coordinator_preamble(INERT_PLAYBOOK, recon, history);
        let tools_section = section_between(&preamble, "## Your Tools", "## Core Behavior");
        assert_no_banned_tokens("coordinator preamble tools section", tools_section);
    }
}

#[test]
fn composed_coordinator_preamble_names_no_retired_tool() {
    let scenario = coordinator_scenarios()
        .into_iter()
        .find(|scenario| !scenario.preamble().vector_stores.is_empty())
        .expect("a scenario configures vector stores");
    let envelope = coordinator_envelope(&scenario).expect("inert envelope assembles");
    assert_no_banned_tokens("composed coordinator preamble", &envelope.system);
}

#[test]
fn session_history_block_names_no_retired_tool() {
    let scenario = coordinator_scenarios()
        .into_iter()
        .find(|scenario| scenario.preamble().session_history.is_some())
        .expect("a scenario carries session history");
    let envelope = coordinator_envelope(&scenario).expect("inert envelope assembles");
    let block = section_from(&envelope.system, "## Session History");
    assert_no_banned_tokens("rendered session-history block", block);
}

#[test]
fn skill_catalog_append_names_no_retired_tool() {
    let scenario = coordinator_scenarios()
        .into_iter()
        .find(|scenario| !scenario.preamble().skills.is_empty())
        .expect("a scenario configures skills");
    let envelope = coordinator_envelope(&scenario).expect("inert envelope assembles");
    let catalog = section_from(&envelope.system, "Available skills");
    assert_no_banned_tokens("coordinator skill-catalog append", catalog);
}

// ============================================================================
// (b) Rendered tool-name lists equal the factory's names
// ============================================================================

#[test]
fn preamble_tools_section_names_the_registered_tools() {
    let sections = worker_sections_for(&coordinator_scenarios()[0]);
    let factory = factory_names(&sections);
    for (recon, history) in [(true, true), (true, false), (false, true), (false, false)] {
        let preamble = build_coordinator_preamble(INERT_PLAYBOOK, recon, history);
        let tools_section = section_between(&preamble, "## Your Tools", "## Core Behavior");
        assert_eq!(
            backticked_tokens(tools_section),
            factory,
            "preamble tools section must name exactly the registered tools"
        );
    }
}

#[test]
fn planning_wrapper_names_the_registered_tools() {
    let sections = worker_sections_for(&coordinator_scenarios()[0]);
    let wrapper = render_planning_prompt(&PlanningVars {
        timestamp: "inert-timestamp",
        query: INERT_QUERY,
        worker_section: "",
        worker_guidelines: "",
    });
    assert_eq!(
        numbered_tool_names(&wrapper),
        factory_names(&sections),
        "planning wrapper tools list must name exactly the registered tools"
    );
}

#[test]
fn planning_loop_wrapper_names_the_registered_tools() {
    let sections = worker_sections_for(&coordinator_scenarios()[0]);
    let wrapper = render_planning_loop_prompt(&PlanningLoopVars {
        timestamp: "inert-timestamp",
        chat_history: "",
        query: INERT_QUERY,
        worker_section: "",
        worker_guidelines: "",
    });
    assert_eq!(
        numbered_tool_names(&wrapper),
        factory_names(&sections),
        "loop planning wrapper tools list must name exactly the registered tools"
    );
}

#[test]
fn continuation_decision_points_name_registered_tools_only() {
    let sections = worker_sections_for(&coordinator_scenarios()[0]);
    let factory = factory_names(&sections);
    let rendered = render_continuation_prompt(&ContinuationVars {
        iteration: "1",
        max_iterations: "3",
        urgency: "",
        succeeded: "0",
        total: "1",
        goal: "Inert goal",
        completed_section: "",
        blocked_section: "",
        redesign_section: "",
        failure_section: "",
        failure_history: "",
        reuse_guidance: "",
    });
    let heads = decision_point_heads(&rendered);
    assert!(
        !heads.is_empty(),
        "continuation prompt carries a decision-point list"
    );
    for head in &heads {
        assert!(
            factory.contains(head),
            "continuation decision point `{head}` is not a registered tool"
        );
    }
}

// ============================================================================
// (c) Coordinator envelopes attach (and claim) the factory's definitions
// ============================================================================

#[test]
fn coordinator_envelopes_attach_the_registered_definitions() {
    for scenario in coordinator_scenarios() {
        let envelope = coordinator_envelope(&scenario).expect("inert envelope assembles");
        let sections = worker_sections_for(&scenario);
        let expected: Vec<MirrorToolDefinition> = coordinator_tool_definitions(&sections)
            .iter()
            .map(mirror_definition)
            .collect();
        assert_eq!(
            envelope.tools, expected,
            "coordinator envelope must attach the factory's definitions in registration order"
        );
    }
}

#[test]
fn coordinator_envelopes_claim_what_they_attach() {
    for scenario in coordinator_scenarios() {
        let envelope = coordinator_envelope(&scenario).expect("inert envelope assembles");
        let attached: BTreeSet<String> = envelope
            .tools
            .iter()
            .map(|definition| definition.name.clone())
            .collect();
        let tools_section = section_between(&envelope.system, "## Your Tools", "## Core Behavior");
        assert_eq!(
            backticked_tokens(tools_section),
            attached,
            "coordinator preamble must claim exactly the tools the envelope attaches"
        );
    }
}

// ============================================================================
// (d) The create_plan example JSON matches the registered args
// ============================================================================

#[test]
fn create_plan_examples_match_the_registered_args() {
    let examples = create_plan_examples();
    assert!(
        !examples.is_empty(),
        "the coordinator preamble carries create_plan example JSON"
    );
    let reference = CreatePlanArgs {
        goal: "Inert goal".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "Inert task".to_owned(),
            worker: None,
        }],
        planning_rationale: "Inert rationale".to_owned(),
    };
    let declared: BTreeSet<String> = serde_json::to_value(&reference)
        .expect("reference args serialize")
        .as_object()
        .expect("CreatePlanArgs serializes as an object")
        .keys()
        .cloned()
        .collect();
    for example in &examples {
        let keys: BTreeSet<String> = example
            .as_object()
            .expect("example JSON is an object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys, declared,
            "create_plan example keys must be exactly CreatePlanArgs' fields"
        );
        serde_json::from_value::<CreatePlanArgs>(example.clone())
            .expect("example deserializes into CreatePlanArgs");
        assert!(
            !steps_name_a_worker(&example["steps"]),
            "create_plan examples must not name a worker: roster validation lives in to_plan"
        );
    }
}

// ============================================================================
// The pin -> mirror projection
// ============================================================================

#[test]
fn pin_definitions_project_onto_the_mirror_wire_fields() {
    let sections = worker_sections_for(&coordinator_scenarios()[0]);
    for definition in &coordinator_tool_definitions(&sections) {
        let mirror = mirror_definition(definition);
        assert_eq!(mirror.name, definition.name.as_str());
        assert_eq!(mirror.description, definition.description);
        assert_eq!(mirror.parameters, definition.input_schema.to_value());
    }
}
