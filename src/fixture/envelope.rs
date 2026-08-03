//! The envelope-composition seam: build the complete request envelope for a
//! scenario by calling the REAL production assembly functions (ported as free
//! functions in this crate).
//!
//! Ported from `crates/aura/src/orchestration/context_fixture/envelope.rs`.
//! Key adaptations: rig `Message`/`ToolDefinition` → crate-local mirror types;
//! async `Tool::definition().await` → sync static constructors; `Orchestrator`
//! methods → free functions in `crate::producers` and `crate::config_builders`.

use std::collections::HashMap;

use crate::bounding::ToolListLimit;
use crate::config::{OrchestrationConfig, VectorStoreConfig};
use crate::config_builders::build_vector_store_context;
use crate::config_builders::{build_coordinator_preamble, build_worker_preamble};
use crate::message::{Message, ToolDefinition};
use crate::persistence::ToolTraceEntry;
use crate::producers::{
    ToolInventory, build_continuation_wrapper, build_planning_wrapper, build_task_context,
    build_worker_prompt_sections, compact_decision_turn,
};
use crate::templates::{
    WorkerPreambleVars, WorkerTaskVars, render_worker_preamble, render_worker_task_prompt,
};
use crate::types::{IterationContext, Plan, StructuredTaskOutput};

use super::helpers::{SCRATCHPAD_PREAMBLE, build_session_context, render_skill_catalog};
use super::scenario::{
    CoordinatorCall, CoordinatorScenario, CoordinatorToolConfig, FixtureError, HistoryTools,
    IterationFixture, PreambleFixture, ReconTools, ScratchpadWiring, TaskOutcome,
    WorkerFrameFixture, WorkerPreambleAppends, WorkerPreambleFixture, WorkerRosterFixture,
    WorkerScenario,
};
use super::scenario::{FailedResultFixture, PlanDecision};
use super::tool_definitions;

/// The complete request envelope for one model call.
pub(crate) use crate::message::RequestEnvelope;

/// Compose the coordinator system preamble for a [`PreambleFixture`].
pub(crate) fn compose_coordinator_preamble(fixture: &PreambleFixture) -> String {
    crate::corpus_configuration::assert_corpus_configuration();
    let mut preamble = build_coordinator_preamble(
        &fixture.playbook,
        fixture.tools.recon == ReconTools::Included,
        fixture.tools.history == HistoryTools::Included,
    );
    if let Some(catalog) = render_skill_catalog(&fixture.skills) {
        preamble.push_str(&catalog);
    }
    if !fixture.vector_stores.is_empty() {
        preamble.push_str(&build_vector_store_context(&fixture.vector_stores));
    }
    if let Some(session) = &fixture.session_history {
        preamble.push('\n');
        preamble.push_str(&build_session_context(session.manifests()));
    }
    preamble
}

/// Apply an iteration's outcomes to its decision's flattened plan, exactly
/// as the execute loop records them.
pub(crate) fn executed_plan(iteration: &IterationFixture) -> Plan {
    let mut plan = iteration.decision().plan();
    for (task, outcome) in plan.tasks.iter_mut().zip(iteration.outcomes()) {
        match outcome {
            TaskOutcome::Complete { result, .. } => {
                task.complete(result.raw_result());
                task.structured_output = result.claim().map(|claim| StructuredTaskOutput {
                    summary: claim.summary().to_owned(),
                    confidence: claim.confidence(),
                });
            }
            TaskOutcome::Failed { report, .. } => match report {
                FailedResultFixture::Hard { error, category } => {
                    task.fail(error.clone(), *category);
                }
                FailedResultFixture::Soft { claim, artifact } => {
                    let error = match artifact {
                        Some(artifact) => format!("{}\n\n{artifact}", claim.summary()),
                        None => claim.summary().to_owned(),
                    };
                    task.fail(error, crate::types::FailureCategory::SoftFailure);
                    task.structured_output = Some(StructuredTaskOutput {
                        summary: claim.summary().to_owned(),
                        confidence: claim.confidence(),
                    });
                }
            },
            TaskOutcome::Blocked => {}
        }
    }
    plan
}

/// The tool traces an outcome carries (blocked tasks never ran).
fn outcome_traces(outcome: &TaskOutcome) -> &[ToolTraceEntry] {
    match outcome {
        TaskOutcome::Complete { traces, .. } | TaskOutcome::Failed { traces, .. } => traces,
        TaskOutcome::Blocked => &[],
    }
}

/// The in-memory re-statement of `load_tool_traces_for_plan`'s run-wide
/// merge.
pub(crate) fn merged_traces(
    iterations: &[IterationFixture],
    current_plan: &Plan,
) -> HashMap<usize, Vec<ToolTraceEntry>> {
    let mut merged = HashMap::new();
    for task in &current_plan.tasks {
        let entries: Vec<ToolTraceEntry> = iterations
            .iter()
            .flat_map(|iteration| {
                iteration
                    .outcomes()
                    .get(task.id)
                    .map(outcome_traces)
                    .unwrap_or(&[])
            })
            .cloned()
            .collect();
        if entries.is_empty() {
            continue;
        }
        merged.insert(task.id, entries);
    }
    merged
}

/// Collect failed tasks from this iteration into failure records.
/// Ported from `Orchestrator::collect_iteration_failures`.
fn collect_iteration_failures(
    plan: &Plan,
    iteration: usize,
) -> Vec<crate::types::FailedTaskRecord> {
    plan.tasks
        .iter()
        .filter_map(|t| match &t.state {
            crate::types::TaskState::Failed { error, category } => {
                Some(crate::types::FailedTaskRecord {
                    description: t.description.clone(),
                    error: error.clone(),
                    iteration,
                    worker: t.worker.clone(),
                    category: *category,
                })
            }
            _ => None,
        })
        .collect()
}

/// Build the coordinator envelope for the scenario's planning call.
pub(crate) fn coordinator_envelope(
    scenario: &CoordinatorScenario,
) -> Result<RequestEnvelope, FixtureError> {
    let system = compose_coordinator_preamble(scenario.preamble());

    let (worker_section, _worker_field, worker_guidelines) = build_worker_prompt_sections(
        scenario.roster().config(),
        ToolListLimit::new(scenario.roster().config().max_tools_per_worker),
        scenario.roster().vector_catalog(),
        // The corpus is MCP-less: aura's `get_all_tool_names` answered with
        // an empty vector for every scenario these envelopes were captured
        // from.
        &ToolInventory::empty(),
    );

    let planning_wrapper = build_planning_wrapper(
        scenario.query().as_str(),
        &worker_section,
        &worker_guidelines,
    );

    let mut messages = vec![Message::user(planning_wrapper)];
    if let CoordinatorCall::Continuation(thread) = scenario.call() {
        let iterations = thread.iterations();
        let config = scenario.roster().config();
        let mut failure_history = Vec::new();
        for (idx, iteration) in iterations.iter().enumerate() {
            let iteration_number = idx + 1;
            let plan = executed_plan(iteration);
            failure_history.extend(collect_iteration_failures(&plan, iteration_number));
            let traces = merged_traces(&iterations[..=idx], &plan);
            let context = IterationContext::new(
                iteration_number,
                plan,
                iteration.failure_summary().cloned(),
                failure_history.clone(),
                traces,
            )
            .with_pinned_goal(scenario.query().clone());

            let assistant_text = compact_decision_turn(iteration.decision().as_response(), "");
            messages.push(Message::assistant(assistant_text));
            messages.push(Message::user(build_continuation_wrapper(
                &context,
                scenario.budget().get(),
                config.show_tool_reasoning_in_continuation(),
                config.result_summary_length(),
            )));
        }
    }

    let tools = coordinator_tool_definitions(scenario)?;
    Ok(RequestEnvelope {
        system,
        messages,
        tools,
    })
}

/// Build the worker envelope.
pub(crate) fn worker_envelope(scenario: &WorkerScenario) -> Result<RequestEnvelope, FixtureError> {
    let system = compose_worker_preamble(&scenario.preamble);

    let context_str = match &scenario.frame {
        WorkerFrameFixture::EmptyFirstTurn { .. }
        | WorkerFrameFixture::EmptyReplanBoundary { .. } => String::new(),
        WorkerFrameFixture::Populated(graph) => {
            let context = build_task_context(graph.plan(), graph.task_id())
                .expect("populated frame renders: validated at FrameGraph construction");
            format!("{context}\n\n")
        }
    };
    let prompt = render_worker_task_prompt(&WorkerTaskVars {
        context: &context_str,
        your_task: scenario.frame.task_text(),
    });

    let tools = worker_tool_definitions(scenario);
    Ok(RequestEnvelope {
        system,
        messages: vec![Message::user(prompt)],
        tools,
    })
}

/// Compose the worker system preamble for a [`WorkerPreambleFixture`].
pub(crate) fn compose_worker_preamble(fixture: &WorkerPreambleFixture) -> String {
    let (mut preamble, appends) = match fixture {
        WorkerPreambleFixture::Role {
            role_preamble,
            vector_stores,
            appends,
        } => {
            let mut preamble = render_worker_preamble(&WorkerPreambleVars {
                worker_system_prompt: role_preamble,
            });
            if !vector_stores.is_empty() {
                preamble.push_str(&build_vector_store_context(vector_stores));
            }
            (preamble, appends)
        }
        WorkerPreambleFixture::Generic {
            custom_prompt,
            appends,
        } => {
            let config = OrchestrationConfig {
                worker_system_prompt: custom_prompt.clone(),
                ..Default::default()
            };
            (build_worker_preamble(&config), appends)
        }
    };
    append_shared_worker_sections(&mut preamble, appends);
    preamble
}

/// The config-conditional appends shared by both worker branches, in
/// constructor order: scratchpad preamble, then skill catalog.
fn append_shared_worker_sections(preamble: &mut String, appends: &WorkerPreambleAppends) {
    if appends.scratchpad == ScratchpadWiring::Wired {
        preamble.push_str(SCRATCHPAD_PREAMBLE);
    }
    if let Some(catalog) = render_skill_catalog(&appends.skills) {
        preamble.push_str(&catalog);
    }
}

/// The coordinator's in-repo tool definitions in production registration order
/// (recon, routing, read_artifact, history, skills).
fn coordinator_tool_definitions(
    scenario: &CoordinatorScenario,
) -> Result<Vec<ToolDefinition>, FixtureError> {
    let mut tools = Vec::new();
    if scenario.preamble().tools.recon == ReconTools::Included {
        tools.push(tool_definitions::list_tools_definition());
        tools.push(tool_definitions::inspect_tool_params_definition());
    }

    tools.push(tool_definitions::respond_directly_definition());
    tools.push(tool_definitions::create_plan_definition());
    tools.push(tool_definitions::request_clarification_definition());

    tools.push(tool_definitions::read_artifact_definition());

    if scenario.preamble().tools.history == HistoryTools::Included {
        tools.push(tool_definitions::list_prior_runs_definition());
    }

    if !scenario.preamble().skills.is_empty() {
        tools.push(tool_definitions::load_skill_definition(
            &scenario.preamble().skills,
        ));
        tools.push(tool_definitions::read_skill_file_definition());
    }

    Ok(tools)
}

/// The worker's in-repo tool definitions in production registration order
/// (`read_artifact`, `submit_result`, then skills).
pub(crate) fn worker_tool_definitions(scenario: &WorkerScenario) -> Vec<ToolDefinition> {
    let mut tools = Vec::new();
    tools.push(tool_definitions::read_artifact_definition());
    tools.push(tool_definitions::submit_result_definition());

    let skills = match &scenario.preamble {
        WorkerPreambleFixture::Role { appends, .. }
        | WorkerPreambleFixture::Generic { appends, .. } => &appends.skills,
    };
    if !skills.is_empty() {
        tools.push(tool_definitions::load_skill_definition(skills));
        tools.push(tool_definitions::read_skill_file_definition());
    }

    tools
}
