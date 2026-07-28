//! The S2 golden-frame snapshot corpus, ported from
//! `crates/aura/src/orchestration/context_fixture/golden_tests.rs`.
//!
//! 21 snapshot tests (13 coordinator + 8 worker) prove byte-identity of the
//! spike's generated envelopes against the canonical aura corpus. The R3/R5/R8
//! comparison gates that call live production `Orchestrator` constructors are
//! SKIPPED — the spike has no `Orchestrator` to compare against; the
//! byte-diff proof against the canonical snapshots is the spike's
//! equivalent gate.

use std::collections::HashMap;

use crate::config::OrchestrationConfig;
use crate::config::{SkillConfig, SkillName, ToolVisibility, VectorStoreConfig, WorkerConfig};
use crate::context::{
    ContextError, EvidenceText, PinnedGoal, ResultPreview, SpilledArtifact, WorkerClaim,
};
use crate::persistence::{
    ArtifactEntry, ArtifactKind, ErrorContext, RoutingMode, RunManifest, RunStatus, TaskSummary,
    ToolOutcome, ToolTraceEntry,
};
use crate::tools::submit_result::Confidence;
use crate::types::{
    FailureCategory, FailureSummary, Plan, PlanningResponse, StepInput, Task, TaskStatus,
};

use crate::fixture::{
    CompletedResultFixture, ContinuationThread, CoordinatorCall, CoordinatorScenario,
    CoordinatorToolConfig, FailedResultFixture, FixtureError, FrameGraph, HistoryTools,
    IterationFixture, NormalizedSnapshot, PlanDecision, PlanningBudget, PreambleFixture,
    ReconTools, ScratchpadWiring, SessionHistoryFixture, SpilledStandIn, TaskOutcome,
    WorkerFrameFixture, WorkerPreambleAppends, WorkerPreambleFixture, WorkerRosterFixture,
    WorkerScenario, assert_envelope_snapshot, coordinator_envelope, normalize, worker_envelope,
};

/// The shared coordinator playbook, preserving the 14 headed blocks of
/// MANIFEST §1 rows 5-18.
const SOURCE_PLAYBOOK: &str = "\
You coordinate SRE investigations for the payments platform. Decompose \
queries into worker tasks and route decisively.

ROUTING
Route log questions to the analyst and shell work to the operator.

PHASE BOUNDARY PRINCIPLE
Finish investigation before remediation; never mix the two in one plan.

OPERATING STRATEGY
Start from the narrowest signal that can falsify the leading hypothesis.

INITIAL PLAN CONTRACT
The first plan gathers evidence only; it makes no changes to any system.

EXACT-DATA HANDOFF
Copy exact identifiers, values, and file names into task descriptions.

Decision-packet checklist:
- evidence collected
- hypothesis stated
- next action named

After each iteration, weigh the new evidence before planning further work.

SINGLE-ACTION TASK CONTRACT
Each task performs exactly one action a single worker can complete.

DEPTH-FAILURE RECOVERY
When a worker exhausts its turn budget, split the task rather than retry.

REPLAN BUDGET
Spend replans on new evidence, never on repeating a failed approach.

WORKER SELECTION
Match each task to the worker whose tools cover the task's data source.

PLAN STRUCTURE
Prefer short sequential chains; parallelize only independent lookups.

TASK DESCRIPTIONS
Write self-contained descriptions; workers see no conversation history.";

/// The verbatim user query pinned across every coordinator fixture.
const QUERY: &str = "Investigate the elevated error rates in the payments service \
and report the top failure groups with supporting evidence.";

// ============================================================================
// Shared fixture inputs
// ============================================================================

fn worker(description: &str, preamble: &str, vector_stores: &[&str]) -> WorkerConfig {
    WorkerConfig {
        description: description.to_owned(),
        preamble: preamble.to_owned(),
        mcp_filter: Vec::new(),
        vector_stores: vector_stores.iter().map(|s| (*s).to_owned()).collect(),
        turn_depth: None,
        llm: None,
        scratchpad: None,
        skills: None,
    }
}

fn analyst_operator_workers() -> HashMap<String, WorkerConfig> {
    HashMap::from([
        (
            "analyst".to_owned(),
            worker(
                "Log and metric analysis for the payments platform",
                "You are the payments log analyst. Ground every claim in log evidence.",
                &[],
            ),
        ),
        (
            "operator".to_owned(),
            worker(
                "Shell and deployment operations",
                "You are the operations specialist. Report exact commands and outputs.",
                &[],
            ),
        ),
    ])
}

fn roster_config(
    workers: HashMap<String, WorkerConfig>,
    tools_in_planning: ToolVisibility,
) -> OrchestrationConfig {
    OrchestrationConfig {
        enabled: true,
        workers,
        tools_in_planning,
        ..Default::default()
    }
}

fn vector_store(name: &str, context_prefix: Option<&str>) -> VectorStoreConfig {
    VectorStoreConfig::new(name, context_prefix)
}

fn skill(name: &str, description: &str) -> SkillConfig {
    SkillConfig {
        name: SkillName::new(name).expect("valid fixture skill name"),
        description: description.to_owned(),
        path: std::path::PathBuf::from(format!("/fixtures/skills/{name}")),
    }
}

fn fixture_skills() -> Vec<SkillConfig> {
    vec![
        skill("log-triage", "Structured log triage playbook"),
        skill("postmortem-draft", "Incident postmortem drafting guide"),
    ]
}

fn preamble(tools: CoordinatorToolConfig) -> PreambleFixture {
    PreambleFixture {
        playbook: SOURCE_PLAYBOOK.to_owned(),
        tools,
        skills: Vec::new(),
        vector_stores: Vec::new(),
        session_history: None,
    }
}

fn no_optional_tools() -> CoordinatorToolConfig {
    CoordinatorToolConfig {
        recon: ReconTools::Excluded,
        history: HistoryTools::Excluded,
    }
}

fn goal() -> PinnedGoal {
    PinnedGoal::new(QUERY).expect("fixture query is non-empty")
}

fn claim(summary: &str, confidence: Confidence) -> WorkerClaim {
    WorkerClaim::new(summary, confidence).expect("fixture claim summary is non-empty")
}

fn evidence(text: &str) -> EvidenceText {
    EvidenceText::new(text).expect("fixture evidence is inline-parseable")
}

fn spilled(filename: &str, full_chars: usize) -> SpilledArtifact {
    SpilledArtifact::new(filename, full_chars).expect("fixture artifact filename is non-empty")
}

fn leaf(task: &str, worker: Option<&str>) -> StepInput {
    StepInput::LeafTask {
        task: task.to_owned(),
        worker: worker.map(str::to_owned),
    }
}

fn decision(rationale: &str, steps: Vec<StepInput>) -> PlanDecision {
    PlanDecision::new(PlanningResponse::StepsPlan {
        goal: QUERY.to_owned(),
        steps,
        routing_rationale: rationale.to_owned(),
        planning_summary: "Gather evidence, then correlate and summarize.".to_owned(),
    })
    .expect("fixture decisions are steps plans that flatten")
}

fn success_trace(tool: &str, reasoning: &str, ms: u64, artifact: Option<&str>) -> ToolTraceEntry {
    ToolTraceEntry {
        tool: tool.to_owned(),
        reasoning: reasoning.to_owned(),
        duration_ms: ms,
        outcome: ToolOutcome::Success { output_bytes: 512 },
        artifact_filename: artifact.map(str::to_owned),
    }
}

fn failed_trace(tool: &str, reasoning: &str, ms: u64, message: &str) -> ToolTraceEntry {
    ToolTraceEntry {
        tool: tool.to_owned(),
        reasoning: reasoning.to_owned(),
        duration_ms: ms,
        outcome: ToolOutcome::Error {
            message: message.to_owned(),
        },
        artifact_filename: None,
    }
}

// ============================================================================
// Session-history manifests
// ============================================================================

fn routed_manifest() -> RunManifest {
    RunManifest {
        run_id: "run-routed-0001".to_owned(),
        session_id: Some("s2-session".to_owned()),
        timestamp: "2026-07-08T09:15:00Z".to_owned(),
        goal: "Triage the payments error spike".to_owned(),
        status: RunStatus::PartialSuccess,
        iterations: 1,
        routing_mode: Some(RoutingMode::Orchestrated),
        outcome: Some("1/2 tasks completed".to_owned()),
        response_summary: None,
        task_summaries: vec![
            TaskSummary {
                task_id: 0,
                description: "Search payments logs for error patterns".to_owned(),
                status: TaskStatus::Complete,
                worker: Some("analyst".to_owned()),
                result_preview: Some("Found 47 error groups; top: connection timeouts".to_owned()),
                confidence: Some("high".to_owned()),
                failure_category: None,
                error: None,
                error_context: None,
                tool_trace: vec![
                    success_trace(
                        "log_search",
                        "searching error patterns",
                        8200,
                        Some("task-0-analyst-iter-1-log_search-0-output.txt"),
                    ),
                    failed_trace(
                        "get_metrics",
                        "pool utilization",
                        3100,
                        "408 upstream timeout",
                    ),
                ],
                artifacts: vec![
                    ArtifactEntry {
                        filename: "task-0-analyst-iter-1-result.txt".to_owned(),
                        size_bytes: 3200,
                        kind: ArtifactKind::Result,
                    },
                    ArtifactEntry {
                        filename: "task-0-analyst-iter-1-log_search-0-output.txt".to_owned(),
                        size_bytes: 48291,
                        kind: ArtifactKind::ToolOutput {
                            tool_name: "log_search".to_owned(),
                        },
                    },
                ],
            },
            TaskSummary {
                task_id: 1,
                description: "Query deployment history for the error window".to_owned(),
                status: TaskStatus::Failed,
                worker: None,
                result_preview: None,
                confidence: None,
                failure_category: Some(FailureCategory::AgentError),
                error: Some("403 Forbidden from the deployment API".to_owned()),
                error_context: Some(ErrorContext {
                    category: FailureCategory::AgentError,
                    last_tool_call: Some("get_deployments".to_owned()),
                    attempt_count: 1,
                    partial_result: Some("Staging query succeeded".to_owned()),
                }),
                tool_trace: vec![],
                artifacts: vec![],
            },
        ],
        artifact_paths: vec![],
    }
}

fn direct_manifest() -> RunManifest {
    RunManifest {
        run_id: "run-direct-0002".to_owned(),
        session_id: Some("s2-session".to_owned()),
        timestamp: "2026-07-09T18:30:00Z".to_owned(),
        goal: "What did the last triage conclude?".to_owned(),
        status: RunStatus::Success,
        iterations: 0,
        routing_mode: Some(RoutingMode::DirectAnswer),
        outcome: Some("Answered directly".to_owned()),
        response_summary: Some("Summarized the prior triage results.".to_owned()),
        task_summaries: vec![],
        artifact_paths: vec![],
    }
}

fn catch_all_manifest() -> RunManifest {
    RunManifest {
        run_id: "run-catch-all-0001".to_owned(),
        session_id: Some("s2-session".to_owned()),
        timestamp: "2026-07-10T09:15:00Z".to_owned(),
        goal: "Triage the payments error spike".to_owned(),
        status: RunStatus::PartialSuccess,
        iterations: 1,
        routing_mode: Some(RoutingMode::Orchestrated),
        outcome: Some("0/2 tasks completed".to_owned()),
        response_summary: None,
        task_summaries: vec![
            TaskSummary {
                task_id: 0,
                description: "Search payments logs for error patterns".to_owned(),
                status: TaskStatus::Running,
                worker: Some("analyst".to_owned()),
                result_preview: None,
                confidence: None,
                failure_category: None,
                error: None,
                error_context: None,
                tool_trace: vec![],
                artifacts: vec![],
            },
            TaskSummary {
                task_id: 1,
                description: "Query deployment history for the error window".to_owned(),
                status: TaskStatus::Pending,
                worker: Some("analyst".to_owned()),
                result_preview: None,
                confidence: None,
                failure_category: None,
                error: None,
                error_context: None,
                tool_trace: vec![],
                artifacts: vec![],
            },
        ],
        artifact_paths: vec![],
    }
}

fn session_history() -> SessionHistoryFixture {
    SessionHistoryFixture::new(vec![direct_manifest(), routed_manifest()])
        .expect("fixture manifests are non-empty and recent-first")
}

// ============================================================================
// Coordinator scenarios
// ============================================================================

fn scenario(
    preamble: PreambleFixture,
    roster: WorkerRosterFixture,
    call: CoordinatorCall,
) -> CoordinatorScenario {
    CoordinatorScenario::new(preamble, goal(), roster, call)
        .expect("corpus scenarios are production-reachable")
}

fn snapshot_coordinator(name: &str, scenario: &CoordinatorScenario) {
    let envelope = coordinator_envelope(scenario).expect("corpus envelopes assemble");
    assert_envelope_snapshot(name, &envelope);
}

fn snapshot_worker(name: &str, scenario: &WorkerScenario) {
    let envelope = worker_envelope(scenario).expect("corpus envelopes assemble");
    assert_envelope_snapshot(name, &envelope);
}

#[test]
fn coordinator_call1_recon() {
    let preamble = preamble(CoordinatorToolConfig {
        recon: ReconTools::Included,
        history: HistoryTools::Excluded,
    });
    let roster = WorkerRosterFixture::new(
        roster_config(analyst_operator_workers(), ToolVisibility::None),
        Vec::new(),
    );
    let scenario = scenario(preamble, roster, CoordinatorCall::Initial);
    snapshot_coordinator("coordinator_call1_recon", &scenario);
}

#[test]
fn coordinator_call1_nonrecon_summary() {
    let mut workers = HashMap::new();
    workers.insert(
        "search".to_owned(),
        worker(
            "Knowledge-base search across incident history",
            "You are the search specialist.",
            &["runbooks", "incidents", "postmortems", "telemetry"],
        ),
    );
    workers.insert(
        "triage".to_owned(),
        worker(
            "First-pass triage without external systems",
            "You are the triage specialist.",
            &[],
        ),
    );
    let config = OrchestrationConfig {
        max_tools_per_worker: 2,
        ..roster_config(workers, ToolVisibility::Summary)
    };
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(config, Vec::new()),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("coordinator_call1_nonrecon_summary", &scenario);
}

#[test]
fn coordinator_call1_full_visibility() {
    let mut workers = HashMap::new();
    workers.insert(
        "search".to_owned(),
        worker(
            "Knowledge-base search across incident history",
            "You are the search specialist.",
            &["runbooks", "scratch", "archive"],
        ),
    );
    workers.insert(
        "triage".to_owned(),
        worker(
            "First-pass triage without external systems",
            "You are the triage specialist.",
            &[],
        ),
    );
    let config = OrchestrationConfig {
        max_tools_per_worker: 2,
        ..roster_config(workers, ToolVisibility::Full)
    };
    let catalog = vec![vector_store(
        "runbooks",
        Some("Operational runbooks for the payments platform"),
    )];
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(config, catalog),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("coordinator_call1_full_visibility", &scenario);
}

#[test]
fn coordinator_call1_no_workers() {
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(
            roster_config(HashMap::new(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("coordinator_call1_no_workers", &scenario);
}

#[test]
fn coordinator_preamble_full_appends() {
    let preamble = PreambleFixture {
        playbook: SOURCE_PLAYBOOK.to_owned(),
        tools: CoordinatorToolConfig {
            recon: ReconTools::Excluded,
            history: HistoryTools::Included,
        },
        skills: fixture_skills(),
        vector_stores: vec![vector_store(
            "runbooks",
            Some("Operational runbooks for the payments platform"),
        )],
        session_history: Some(session_history()),
    };
    let scenario = scenario(
        preamble,
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("coordinator_preamble_full_appends", &scenario);
}

/// Session-history block with catch-all Running and Pending task summaries.
#[test]
fn session_history_catch_all() {
    let preamble = PreambleFixture {
        playbook: SOURCE_PLAYBOOK.to_owned(),
        tools: no_optional_tools(),
        skills: Vec::new(),
        vector_stores: Vec::new(),
        session_history: Some(
            SessionHistoryFixture::new(vec![catch_all_manifest()]).expect("one prior manifest"),
        ),
    };
    let scenario = scenario(
        preamble,
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("session_history_catch_all", &scenario);
}

#[test]
fn tools_coordinator_recon_history() {
    let preamble = preamble(CoordinatorToolConfig {
        recon: ReconTools::Included,
        history: HistoryTools::Included,
    });
    let scenario = scenario(
        preamble,
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::None),
            Vec::new(),
        ),
        CoordinatorCall::Initial,
    );
    snapshot_coordinator("tools_coordinator_recon_history", &scenario);
}

/// The clean iteration behind `coordinator_call2_clean`.
fn clean_iteration() -> IterationFixture {
    let decision = decision(
        "Two evidence-gathering lookups are required before answering.",
        vec![
            leaf(
                "Collect the error groups from the payments logs for the last six hours",
                Some("analyst"),
            ),
            leaf(
                "Assemble the deployment timeline for the same window",
                Some("operator"),
            ),
        ],
    );
    let outcomes = vec![
        TaskOutcome::Complete {
            result: CompletedResultFixture::Inline {
                result: evidence(
                    "Found 47 error groups across 3 services; top: connection timeouts (38%).",
                ),
                claim: Some(claim(
                    "Found 47 error groups; connection timeouts dominate",
                    Confidence::High,
                )),
            },
            traces: vec![
                success_trace(
                    "log_search",
                    "searching payments error patterns",
                    8200,
                    Some("task-0-analyst-iter-1-log_search-0-output.txt"),
                ),
                success_trace("get_metrics", "", 3100, None),
            ],
        },
        TaskOutcome::Complete {
            result: CompletedResultFixture::Inline {
                result: evidence(
                    "Deployment timeline assembled: 3 deploys landed inside the error window.",
                ),
                claim: None,
            },
            traces: vec![],
        },
    ];
    IterationFixture::new(decision, outcomes, None).expect("clean iteration validates")
}

#[test]
fn coordinator_call2_clean() {
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Continuation(
            ContinuationThread::new(vec![clean_iteration()]).expect("one iteration"),
        ),
    );
    snapshot_coordinator("coordinator_call2_clean", &scenario);
}

#[test]
fn coordinator_call_completed_task_tool_chain() {
    let mut config = roster_config(analyst_operator_workers(), ToolVisibility::Summary);
    config.artifacts.show_tool_reasoning_in_continuation = true;
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(config, Vec::new()),
        CoordinatorCall::Continuation(
            ContinuationThread::new(vec![clean_iteration()]).expect("one iteration"),
        ),
    );
    snapshot_coordinator("coordinator_call_completed_task_tool_chain", &scenario);
}

#[test]
fn coordinator_call2_all_failed() {
    let decision = decision(
        "Both lookups need tool execution.",
        vec![
            leaf(
                "Collect the error groups from the payments logs for the last six hours",
                Some("analyst"),
            ),
            leaf(
                "Assemble the deployment timeline for the same window",
                Some("operator"),
            ),
        ],
    );
    let long_error = format!("upstream error while streaming logs: {}", "x".repeat(2100));
    let outcomes = vec![
        TaskOutcome::Failed {
            report: FailedResultFixture::Hard {
                error: long_error,
                category: FailureCategory::AgentError,
            },
            traces: vec![],
        },
        TaskOutcome::Failed {
            report: FailedResultFixture::Hard {
                error: "worker timed out before producing a result".to_owned(),
                category: FailureCategory::AgentTimeout,
            },
            traces: vec![],
        },
    ];
    let iteration = IterationFixture::new(
        decision,
        outcomes,
        Some(FailureSummary {
            reasoning: "Execution failed: 2 task(s) failed, 0 task(s) blocked by dependencies."
                .to_owned(),
            gaps: vec!["Some tasks could not complete due to errors".to_owned()],
        }),
    )
    .expect("all-failed iteration validates");
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Continuation(ContinuationThread::new(vec![iteration]).expect("one")),
    );
    snapshot_coordinator("coordinator_call2_all_failed", &scenario);
}

#[test]
fn coordinator_call_all_failure_categories() {
    let categories = vec![
        (FailureCategory::AgentTimeout, "agent timeout"),
        (FailureCategory::ContextOverflow, "context overflow"),
        (FailureCategory::DepthExhausted, "depth exhausted"),
        (FailureCategory::LoopDetected, "loop detected"),
        (FailureCategory::ProviderOverloaded, "provider overloaded"),
        (FailureCategory::ProviderAuthError, "provider auth error"),
        (FailureCategory::ProviderNotFound, "provider not found"),
        (FailureCategory::DependencyFailed, "dependency failed"),
        (FailureCategory::SoftFailure, "soft failure"),
        (FailureCategory::AgentError, "agent error"),
    ];
    let steps: Vec<StepInput> = categories
        .iter()
        .enumerate()
        .map(|(i, (_, msg))| leaf(&format!("Task {i}: {msg}"), Some("analyst")))
        .collect();
    let decision = decision("Exercise every failure category in one iteration.", steps);
    let outcomes: Vec<TaskOutcome> = categories
        .iter()
        .map(|(category, msg)| TaskOutcome::Failed {
            report: FailedResultFixture::Hard {
                error: format!("error: {msg}"),
                category: *category,
            },
            traces: vec![],
        })
        .collect();
    let iteration = IterationFixture::new(
        decision,
        outcomes,
        Some(FailureSummary {
            reasoning: "Execution failed: all tasks failed.".to_owned(),
            gaps: vec!["All failure categories exercised".to_owned()],
        }),
    )
    .expect("all-failure iteration validates");
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(
            roster_config(analyst_operator_workers(), ToolVisibility::Summary),
            Vec::new(),
        ),
        CoordinatorCall::Continuation(ContinuationThread::new(vec![iteration]).expect("one")),
    );
    snapshot_coordinator("coordinator_call_all_failure_categories", &scenario);
}

fn failure_thread_iterations() -> Vec<IterationFixture> {
    let iteration_one = IterationFixture::new(
        decision(
            "Evidence gathering requires log access.",
            vec![
                leaf(
                    "Collect the error groups from the payments logs for the last six hours",
                    Some("analyst"),
                ),
                leaf("Query deployment history for the error window", None),
            ],
        ),
        vec![
            TaskOutcome::Complete {
                result: CompletedResultFixture::Spilled {
                    stand_in: SpilledStandIn::ClaimEcho {
                        claim: claim(
                            "Found 47 error groups; connection timeouts dominate",
                            Confidence::High,
                        ),
                    },
                    artifact: spilled("task-0-analyst-iter-1-result.txt", 5200),
                },
                traces: vec![success_trace(
                    "log_search",
                    "searching payments error patterns",
                    8200,
                    Some("task-0-analyst-iter-1-log_search-0-output.txt"),
                )],
            },
            TaskOutcome::Failed {
                report: FailedResultFixture::Hard {
                    error: "403 Forbidden from the deployment API".to_owned(),
                    category: FailureCategory::AgentError,
                },
                traces: vec![],
            },
        ],
        Some(FailureSummary {
            reasoning: "Execution failed: 1 task(s) failed, 0 task(s) blocked by dependencies."
                .to_owned(),
            gaps: vec!["Deployment history unavailable due to permissions".to_owned()],
        }),
    )
    .expect("iteration 1 validates");

    let iteration_two = IterationFixture::new(
        decision(
            "Retry the deployment lookup and correlate the evidence.",
            vec![
                leaf(
                    "Re-run the error-group collection over the widened twelve-hour window",
                    Some("analyst"),
                ),
                leaf(
                    "Query deployment history for the error window",
                    Some("analyst"),
                ),
                leaf(
                    "Correlate the deployment timeline with the error groups",
                    Some("operator"),
                ),
                leaf(
                    "Draft the incident summary from the correlated evidence",
                    Some("operator"),
                ),
            ],
        ),
        vec![
            TaskOutcome::Complete {
                result: CompletedResultFixture::Spilled {
                    stand_in: SpilledStandIn::RawPreview {
                        preview: ResultPreview::new(
                            "Widened window confirms 52 error groups; timeouts still dominate.",
                        )
                        .expect("non-empty preview"),
                        claim: None,
                    },
                    artifact: spilled("task-0-analyst-iter-2-result.txt", 6100),
                },
                traces: vec![success_trace(
                    "log_search",
                    "re-running with the widened window",
                    9400,
                    Some("task-0-analyst-iter-2-log_search-0-output.txt"),
                )],
            },
            TaskOutcome::Failed {
                report: FailedResultFixture::Hard {
                    error: "403 Forbidden from the deployment API".to_owned(),
                    category: FailureCategory::AgentError,
                },
                traces: vec![
                    success_trace("get_deployments", "checking staging first", 1200, None),
                    success_trace("get_deployments", "", 900, None),
                    failed_trace(
                        "get_deployments",
                        "querying prod-us-east-1",
                        30200,
                        "403 Forbidden",
                    ),
                ],
            },
            TaskOutcome::Failed {
                report: FailedResultFixture::Soft {
                    claim: claim(
                        "Only partial correlation evidence was recoverable",
                        Confidence::Low,
                    ),
                    artifact: Some(spilled("task-2-operator-iter-2-result.txt", 5200)),
                },
                traces: vec![],
            },
            TaskOutcome::Blocked,
        ],
        Some(FailureSummary {
            reasoning: "Execution failed: 2 task(s) failed, 1 task(s) blocked by dependencies."
                .to_owned(),
            gaps: vec![
                "Deployment history remains unavailable".to_owned(),
                "Incident summary blocked on the correlation task".to_owned(),
            ],
        }),
    )
    .expect("iteration 2 validates");

    vec![iteration_one, iteration_two]
}

#[test]
fn coordinator_call3_failures() {
    let config = OrchestrationConfig {
        max_planning_cycles: 4,
        ..roster_config(analyst_operator_workers(), ToolVisibility::Summary)
    };
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(config, Vec::new()),
        CoordinatorCall::Continuation(
            ContinuationThread::new(failure_thread_iterations()).expect("two iterations"),
        ),
    );
    snapshot_coordinator("coordinator_call3_failures", &scenario);
}

#[test]
fn coordinator_call4_final_urgency() {
    let iteration = |window: &str| {
        IterationFixture::new(
            decision(
                "One focused lookup continues the investigation.",
                vec![leaf(
                    &format!("Collect the error groups for the {window} window"),
                    Some("analyst"),
                )],
            ),
            vec![TaskOutcome::Complete {
                result: CompletedResultFixture::Inline {
                    result: evidence(&format!(
                        "The {window} window shows the same three dominant failure groups."
                    )),
                    claim: None,
                },
                traces: vec![],
            }],
            None,
        )
        .expect("urgency iterations validate")
    };
    let config = OrchestrationConfig {
        max_planning_cycles: 4,
        ..roster_config(analyst_operator_workers(), ToolVisibility::Summary)
    };
    let scenario = scenario(
        preamble(no_optional_tools()),
        WorkerRosterFixture::new(config, Vec::new()),
        CoordinatorCall::Continuation(
            ContinuationThread::new(vec![
                iteration("six-hour"),
                iteration("twelve-hour"),
                iteration("twenty-four-hour"),
            ])
            .expect("three iterations"),
        ),
    );
    snapshot_coordinator("coordinator_call4_final_urgency", &scenario);
}

// ============================================================================
// Worker scenarios
// ============================================================================

const ROLE_PREAMBLE: &str = "You are the payments log analyst. Ground every claim in log evidence.";

fn no_appends() -> WorkerPreambleAppends {
    WorkerPreambleAppends {
        scratchpad: ScratchpadWiring::NotWired,
        skills: Vec::new(),
    }
}

fn bare_role_preamble() -> WorkerPreambleFixture {
    WorkerPreambleFixture::Role {
        role_preamble: ROLE_PREAMBLE.to_owned(),
        vector_stores: Vec::new(),
        appends: no_appends(),
    }
}

fn completed_task(id: usize, description: &str, result: &CompletedResultFixture) -> Task {
    let mut task = Task::new(id, description, "fixture ancestor");
    task.complete(result.raw_result());
    task.structured_output = result
        .claim()
        .map(|claim| crate::types::StructuredTaskOutput {
            summary: claim.summary().to_owned(),
            confidence: claim.confidence(),
        });
    task
}

fn direct_frame(ancestor: &CompletedResultFixture, target_description: &str) -> FrameGraph {
    let mut plan = Plan::new(QUERY);
    let mut ancestor_task = completed_task(
        0,
        "Collect the error groups from the payments logs",
        ancestor,
    );
    ancestor_task.worker = Some("analyst".to_owned());
    plan.add_task(ancestor_task);
    plan.add_task(Task::new(1, target_description, "fixture target").with_dependency(0));
    FrameGraph::new(plan, 1).expect("direct frame renders")
}

#[test]
fn worker_role_frame_direct() {
    let scenario = WorkerScenario {
        preamble: WorkerPreambleFixture::Role {
            role_preamble: ROLE_PREAMBLE.to_owned(),
            vector_stores: vec![
                vector_store(
                    "runbooks",
                    Some("Operational runbooks for the payments platform"),
                ),
                vector_store("telemetry", None),
            ],
            appends: WorkerPreambleAppends {
                scratchpad: ScratchpadWiring::Wired,
                skills: fixture_skills(),
            },
        },
        frame: WorkerFrameFixture::Populated(direct_frame(
            &CompletedResultFixture::Inline {
                result: evidence(
                    "Found 47 error groups across 3 services; top: connection timeouts (38%).",
                ),
                claim: Some(claim(
                    "Found 47 error groups; connection timeouts dominate",
                    Confidence::High,
                )),
            },
            "Correlate the error groups with the deployment timeline",
        )),
    };
    snapshot_worker("worker_role_frame_direct", &scenario);
}

#[test]
fn worker_role_frame_transitive() {
    let mut plan = Plan::new(QUERY);
    let mut task0 = completed_task(
        0,
        "Collect the error groups from the payments logs",
        &CompletedResultFixture::Inline {
            result: evidence("Found 47 error groups across 3 services."),
            claim: None,
        },
    );
    task0.worker = Some("analyst".to_owned());
    plan.add_task(task0);
    let mut task1 = completed_task(
        1,
        "Assemble the deployment timeline",
        &CompletedResultFixture::Inline {
            result: evidence("Deployment timeline assembled: 3 deploys in the error window."),
            claim: Some(claim(
                "Three deploys landed in the window",
                Confidence::Medium,
            )),
        },
    );
    task1.worker = Some("operator".to_owned());
    task1.dependencies = vec![0];
    plan.add_task(task1);
    plan.add_task(
        Task::new(
            2,
            "Correlate the deployment timeline with the error groups",
            "fixture target",
        )
        .with_dependency(1),
    );
    let scenario = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::Populated(
            FrameGraph::new(plan, 2).expect("transitive frame renders"),
        ),
    };
    snapshot_worker("worker_role_frame_transitive", &scenario);
}

#[test]
fn worker_role_frame_spilled_claim_echo() {
    let scenario = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::Populated(direct_frame(
            &CompletedResultFixture::Spilled {
                stand_in: SpilledStandIn::ClaimEcho {
                    claim: claim(
                        "Found 47 error groups; connection timeouts dominate",
                        Confidence::High,
                    ),
                },
                artifact: spilled("task-0-analyst-iter-1-result.txt", 5200),
            },
            "Correlate the error groups with the deployment timeline",
        )),
    };
    snapshot_worker("worker_role_frame_spilled_claim_echo", &scenario);
}

#[test]
fn worker_frame_spilled_no_preview() {
    let scenario = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::Populated(direct_frame(
            &CompletedResultFixture::Spilled {
                stand_in: SpilledStandIn::NoPreview,
                artifact: spilled("task-0-analyst-iter-1-result.txt", 5200),
            },
            "Correlate the error groups with the deployment timeline",
        )),
    };
    snapshot_worker("worker_frame_spilled_no_preview", &scenario);
}

const EMPTY_FRAME_TASK: &str =
    "Collect the error groups from the payments logs for the last six hours";

#[test]
fn worker_first_turn_empty() {
    let scenario = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::EmptyFirstTurn {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    snapshot_worker("worker_first_turn_empty", &scenario);
}

#[test]
fn worker_replan_boundary_empty() {
    let scenario = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::EmptyReplanBoundary {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    snapshot_worker("worker_replan_boundary_empty", &scenario);
}

#[test]
fn worker_generic_fallback() {
    let scenario = WorkerScenario {
        preamble: WorkerPreambleFixture::Generic {
            custom_prompt: None,
            appends: WorkerPreambleAppends {
                scratchpad: ScratchpadWiring::Wired,
                skills: fixture_skills(),
            },
        },
        frame: WorkerFrameFixture::EmptyFirstTurn {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    snapshot_worker("worker_generic_fallback", &scenario);
}

#[test]
fn worker_generic_custom() {
    let scenario = WorkerScenario {
        preamble: WorkerPreambleFixture::Generic {
            custom_prompt: Some(
                "Prefer structured summaries over prose; cite exact values.".to_owned(),
            ),
            appends: no_appends(),
        },
        frame: WorkerFrameFixture::EmptyFirstTurn {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    snapshot_worker("worker_generic_custom", &scenario);
}

// ============================================================================
// Constructor validation (parse-don't-validate spot checks)
// ============================================================================

#[test]
fn fixture_constructors_reject_unreachable_states() {
    assert!(matches!(
        PlanningBudget::new(0),
        Err(FixtureError::ZeroPlanningBudget)
    ));
    assert!(matches!(
        SessionHistoryFixture::new(vec![]),
        Err(FixtureError::EmptySessionHistory)
    ));
    assert!(matches!(
        SessionHistoryFixture::new(vec![routed_manifest(), direct_manifest()]),
        Err(FixtureError::SessionHistoryNotRecentFirst)
    ));
    assert!(matches!(
        ContinuationThread::new(vec![]),
        Err(FixtureError::EmptyContinuationThread)
    ));
    assert!(matches!(
        PlanDecision::new(PlanningResponse::Direct {
            response: "done".to_owned(),
            routing_rationale: "r".to_owned(),
            response_summary: None,
        }),
        Err(FixtureError::TerminalDecisionMidThread)
    ));
    assert!(matches!(
        IterationFixture::new(
            decision("one task", vec![leaf("a task", Some("analyst"))]),
            vec![],
            None
        ),
        Err(FixtureError::OutcomeCountMismatch {
            tasks: 1,
            outcomes: 0
        })
    ));
    assert!(matches!(
        IterationFixture::new(
            decision("one task", vec![leaf("a task", Some("analyst"))]),
            vec![TaskOutcome::Complete {
                result: CompletedResultFixture::Inline {
                    result: evidence("done"),
                    claim: None,
                },
                traces: vec![],
            }],
            Some(FailureSummary::default())
        ),
        Err(FixtureError::FailureSummaryWithoutFailure)
    ));
    assert!(matches!(
        CoordinatorScenario::new(
            preamble(CoordinatorToolConfig {
                recon: ReconTools::Included,
                history: HistoryTools::Excluded,
            }),
            goal(),
            WorkerRosterFixture::new(
                roster_config(analyst_operator_workers(), ToolVisibility::Summary),
                Vec::new(),
            ),
            CoordinatorCall::Initial,
        ),
        Err(FixtureError::ReconRequiresUninlinedTools)
    ));
    assert!(matches!(
        CoordinatorScenario::new(
            preamble(no_optional_tools()),
            goal(),
            WorkerRosterFixture::new(
                roster_config(HashMap::new(), ToolVisibility::Summary),
                Vec::new(),
            ),
            CoordinatorCall::Continuation(
                ContinuationThread::new(vec![clean_iteration()]).expect("one iteration"),
            ),
        ),
        Err(FixtureError::CompletedTaskUnknownWorker { task_id: 0, .. })
    ));
    let one_cycle = OrchestrationConfig {
        max_planning_cycles: 1,
        ..roster_config(analyst_operator_workers(), ToolVisibility::Summary)
    };
    assert!(matches!(
        CoordinatorScenario::new(
            preamble(no_optional_tools()),
            goal(),
            WorkerRosterFixture::new(one_cycle, Vec::new()),
            CoordinatorCall::Continuation(
                ContinuationThread::new(failure_thread_iterations()).expect("two iterations"),
            ),
        ),
        Err(FixtureError::IterationsExhaustBudget {
            iterations: 2,
            budget: 1
        })
    ));
    let mut plan = Plan::new(QUERY);
    plan.add_task(Task::new(0, "unstarted predecessor", "fixture"));
    plan.add_task(Task::new(1, "target", "fixture").with_dependency(0));
    assert!(matches!(
        FrameGraph::new(plan, 1),
        Err(FixtureError::FrameHasNoCompletedAncestor { task_id: 1 })
    ));
    assert!(matches!(
        EvidenceText::new(
            "body\n\n[Full result (5200 chars) saved to artifact: task-0-analyst-iter-1-result.txt]"
        ),
        Err(ContextError::InlineEvidenceCarriesSpillFooter)
    ));
}

/// The two empty-`%%CONTEXT%%` snapshots are byte-identical (pre-approved
/// decision 4): the branches differ causally, not mechanically.
#[test]
fn empty_frame_branches_render_byte_identically() {
    let fresh = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::EmptyFirstTurn {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    let replan = WorkerScenario {
        preamble: bare_role_preamble(),
        frame: WorkerFrameFixture::EmptyReplanBoundary {
            task: EMPTY_FRAME_TASK.to_owned(),
        },
    };
    let fresh_snapshot: NormalizedSnapshot =
        normalize(&worker_envelope(&fresh).expect("fresh envelope"));
    let replan_snapshot: NormalizedSnapshot =
        normalize(&worker_envelope(&replan).expect("replan envelope"));
    assert_eq!(fresh_snapshot, replan_snapshot);
}

// ============================================================================
// SKIPPED comparison gates (R3/R5/R8)
// ============================================================================
//
// The following tests from the aura golden_tests.rs are SKIPPED because they
// call live production `Orchestrator` constructors that the spike does not
// port:
//
// - `gate_r3_coordinator_preamble_matches_create_coordinator` — builds a real
//   `Orchestrator` via `Orchestrator::new` and compares `create_coordinator`
//   output against the harness-composed preamble.
// - `gate_r3_worker_preamble_matches_create_worker` — builds a real
//   `Orchestrator` and compares `create_worker` output for both role and
//   generic branches.
// - `gate_r5_trace_merge_matches_persistence_loader` — writes tool records
//   through a tempdir-backed `ExecutionPersistence` and compares the
//   production disk-persistence merge against the harness's in-memory fold.
// - `gate_r8_coordinator_tool_order` — calls `CoordinatorTools::new_for_golden_test`
//   and `Orchestrator::coordinator_tool_order_for_golden` to verify tool
//   registration order.
// - `gate_r8_worker_tool_order` — calls `worker_tool_definitions` and checks
//   the tool name order against `Agent::add_all_tools`.
// - `gate_r8_conversation_growth` — builds a real `Orchestrator` and
//   compares the envelope's message list against a hand-grown expected list.
//
// The spike's equivalent gate is the byte-diff proof against the canonical
// aura snapshots (TASK 5).
