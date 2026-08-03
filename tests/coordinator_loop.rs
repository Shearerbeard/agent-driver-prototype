//! Integration tests for the continuous coordinator loop.
//!
//! Every test drives the real `AgentLoop` through `MockProvider`, so the
//! provider-call arithmetic is exact: the mock panics when its queue is
//! exhausted, and one tool round costs one queued response plus the opening
//! send.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_driver_rs::agent::{AgentEvent, AgentObserver, AgentOutcome, LoopStopReason};
use agent_driver_rs::provider::mock::{MockProvider, mock_text_response, mock_tool_call_response};
use agent_driver_rs::streaming::CollectedResponse;
use agent_driver_rs::types::{ContentBlock, ModelId, SystemPrompt};
use async_trait::async_trait;

use agent_driver_prototype::artifacts::{ArtifactStore, InlineThreshold};
use agent_driver_prototype::bounding::{ErrorPreviewWidth, ToolListLimit};
use agent_driver_prototype::config::{
    OrchestrationConfig, ToolVisibility, VectorStoreConfig, WorkerConfig,
};
use agent_driver_prototype::context::{
    CorrelationLabel, ErrorPreview, EvidenceEntry, EvidenceText, PinnedGoal, TaskId, WorkerClaim,
    WorkerRole,
};
use agent_driver_prototype::coordinator_loop::{
    CoordinatorLoop, CoordinatorLoopConfig, CoordinatorLoopError, CoordinatorOutcome,
    CreatePlanArgs, ExecutionObservation, FinalResponse, InterruptionReason, LoopBudget,
    OutcomeCounts, PlanExecutor, PlanId, PlanObservation, RunStore, TaskObservation, TerminalSlot,
    WorkerRoster, WorkerSections,
};
use agent_driver_prototype::dag_executor::{DagExecutor, WorkerLoopConfig};
use agent_driver_prototype::mcp_client::SidecarClient;
use agent_driver_prototype::producers::{ToolInventory, build_worker_prompt_sections};
use agent_driver_prototype::templates::{PlanningLoopVars, render_planning_loop_prompt};
use agent_driver_prototype::tools::submit_result::Confidence;
use agent_driver_prototype::types::{FailureCategory, StepInput};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The plan arguments every loop test sends, and the id derived from them.
fn plan_args() -> CreatePlanArgs {
    CreatePlanArgs {
        goal: "Summarise yesterday's error spike".to_owned(),
        steps: vec![
            StepInput::LeafTask {
                task: "Collect the error counts by service".to_owned(),
                worker: Some("operations".to_owned()),
            },
            StepInput::LeafTask {
                task: "Name the top contributor".to_owned(),
                worker: None,
            },
        ],
        planning_rationale: "Counting before naming keeps the answer grounded".to_owned(),
    }
}

fn plan_args_json() -> String {
    serde_json::to_string(&plan_args()).expect("plan arguments serialize")
}

fn roster() -> WorkerRoster {
    test_sections().roster().clone()
}

/// Worker material for a run with one configured worker.
fn test_sections() -> WorkerSections {
    let mut workers = HashMap::new();
    workers.insert(
        "operations".to_owned(),
        WorkerConfig {
            description: "Logs, pipelines and metrics".to_owned(),
            preamble: String::new(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    let config = OrchestrationConfig {
        enabled: true,
        workers,
        ..Default::default()
    };
    WorkerSections::from_roster(
        WorkerRoster::from_config(
            &config,
            ToolListLimit::new(10),
            &[],
            &ToolInventory::empty(),
        )
        .expect("no worker configures a turn depth"),
    )
}

fn goal() -> PinnedGoal {
    PinnedGoal::new("Summarise yesterday's error spike").expect("non-empty query")
}

fn model() -> ModelId {
    ModelId::new("mock-model").expect("valid model id")
}

/// The worker-queue entries for a two-task plan where both workers submit.
/// Each task costs two worker provider calls: one `submit_result` tool round
/// plus one end-of-turn text response. The executor dispatches ready tasks
/// in ascending task-id order, so the queue order is task 0 then task 1.
fn two_task_worker_responses() -> Vec<Vec<agent_driver_rs::StreamEvent>> {
    vec![
        mock_tool_call_response(
            "w0",
            "submit_result",
            r#"{"summary":"Collected 530 errors across 4 services","result":"service-a: 412, service-b: 80, service-c: 30, service-d: 8","confidence":"high"}"#,
        ),
        mock_text_response(""),
        mock_tool_call_response(
            "w1",
            "submit_result",
            r#"{"summary":"checkout is the top contributor","result":"checkout: 412 of 530 errors","confidence":"high"}"#,
        ),
        mock_text_response(""),
    ]
}

/// Build a loop over a scripted coordinator provider, with a
/// [`DagExecutor`] backed by a separate worker `MockProvider`.
///
/// The executor shares the run store with the coordinator loop: the test
/// creates one [`RunStore`], hands it to both, and the loop's `Arc`-shared
/// handle keeps them joined so `execute` can read `latest_plan()` and file
/// per-task records that `inspect_run` reads back. The sidecar is
/// disconnected and the artifact store disabled because the
/// `MockProvider`-backed workers only call `submit_result`; a submission
/// whose body fits the inline threshold never touches the store.
async fn coordinator(
    responses: Vec<Vec<agent_driver_rs::StreamEvent>>,
    worker_responses: Vec<Vec<agent_driver_rs::StreamEvent>>,
    turns: u32,
) -> CoordinatorLoop {
    let runs = RunStore::new();
    let worker_config = WorkerLoopConfig {
        provider: Arc::new(MockProvider::new(worker_responses)),
        model: model(),
        budget: LoopBudget::new(8).expect("non-zero worker budget"),
        system_prompt: SystemPrompt::new("You are a worker. Submit your result."),
    };
    let executor: Arc<dyn PlanExecutor> = Arc::new(DagExecutor::new(
        SidecarClient::disconnected(),
        ArtifactStore::disabled(),
        worker_config,
        test_sections(),
        runs.clone(),
        InlineThreshold::DEFAULT,
        None,
    ));
    CoordinatorLoop::new(CoordinatorLoopConfig {
        provider: Arc::new(MockProvider::new(responses)),
        model: model(),
        system_prompt: SystemPrompt::new("You coordinate one continuous loop."),
        budget: LoopBudget::new(turns).expect("non-zero budget"),
        executor,
        worker_sections: test_sections(),
        runs,
    })
    .await
    .expect("session builds")
}

/// Observer that records tool-call starts for non-invocation assertions.
struct ToolCallRecorder {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AgentObserver for ToolCallRecorder {
    async fn on_event(&self, event: &AgentEvent) {
        if let AgentEvent::ToolCallStart { name, .. } = event {
            self.calls
                .lock()
                .expect("recorder mutex")
                .push(name.as_str().to_owned());
        }
    }
}

fn recorder() -> (Arc<ToolCallRecorder>, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Arc::new(ToolCallRecorder {
            calls: Arc::clone(&calls),
        }),
        calls,
    )
}

// ---------------------------------------------------------------------------
// Card acceptance test 1: one nonterminal round trip, no stream break
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_plan_then_execute_continues_without_a_stream_break() {
    let expected_id = PlanId::derive(&plan_args());

    // Four coordinator responses, four coordinator provider calls, three
    // tool rounds. A fifth coordinator provider call would panic the mock,
    // so reaching the assertions is itself proof the loop made exactly
    // four. The worker queue is separate: two tasks, two provider calls
    // each (a submit_result round plus an end-of-turn), four worker calls
    // consumed inside the execute round. A fifth worker call would panic
    // the worker mock, so the counts assertion also proves the executor
    // dispatched exactly two workers.
    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &plan_args_json()),
        mock_tool_call_response(
            "c2",
            "execute",
            &format!(r#"{{"plan_id":"{expected_id}"}}"#),
        ),
        mock_tool_call_response(
            "c3",
            "respond",
            r#"{"response":"Service checkout produced 412 of yesterday's 530 errors."}"#,
        ),
        mock_text_response(""),
    ];

    let coordinator = coordinator(responses, two_task_worker_responses(), 8).await;
    let runs = coordinator.runs().clone();
    let (observer, calls) = recorder();
    let outcome = coordinator
        .with_observer(observer)
        .run(&goal())
        .await
        .expect("the loop runs");

    let CoordinatorOutcome::Responded { action, turns } = outcome else {
        panic!("expected a coordinator-authored answer, got {outcome:?}");
    };
    assert_eq!(turns, 3, "three tool rounds inside one loop");
    assert!(action.response().contains("412"));

    // The tools ran in order inside one conversation: create_plan did not
    // end the loop, and neither did execute.
    assert_eq!(
        *calls.lock().expect("recorder mutex"),
        vec!["create_plan", "execute", "respond"],
    );

    // create_plan returned a success observation: the plan reached the store
    // under the id the test precomputed.
    assert_eq!(runs.plan_ids(), vec![expected_id]);

    let execution = runs.latest_execution().expect("execute recorded a run");
    let counts = execution.counts();
    assert_eq!(counts.completed(), 2);
    assert_eq!(counts.failed(), 0);
    assert_eq!(counts.blocked(), 0);
}

// ---------------------------------------------------------------------------
// Card acceptance test 2: budget-forced termination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn turn_budget_stops_the_loop_and_the_host_writes_the_answer() {
    let expected_id = PlanId::derive(&plan_args());

    // Budget 2 permits two coordinator tool rounds. The third coordinator
    // response still has to be queued because the depth check runs at the
    // top of the round that reads it, and its tool call is refused rather
    // than executed. The execute round runs the full two-task DAG against
    // the worker mock (four worker calls) before the third coordinator
    // round is reached.
    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &plan_args_json()),
        mock_tool_call_response(
            "c2",
            "execute",
            &format!(r#"{{"plan_id":"{expected_id}"}}"#),
        ),
        mock_tool_call_response(
            "c3",
            "inspect_run",
            r#"{"selector":{"record":"latest_execution"}}"#,
        ),
    ];

    let coordinator = coordinator(responses, two_task_worker_responses(), 2).await;
    let runs = coordinator.runs().clone();
    let (observer, calls) = recorder();
    let outcome = coordinator
        .with_observer(observer)
        .run(&goal())
        .await
        .expect("the loop runs");

    let CoordinatorOutcome::BudgetExhausted { fallback, turns } = outcome else {
        panic!("expected a host-authored answer, got {outcome:?}");
    };
    assert_eq!(turns, 2);

    // Exactly two tool calls started: the third response's call was refused
    // before dispatch.
    assert_eq!(
        *calls.lock().expect("recorder mutex"),
        vec!["create_plan", "execute"],
    );

    // The fallback rendered from the execution that did run, not from the
    // no-execution template. The worker's submitted evidence (not the stub)
    // is what the host writes: "412" appears in the worker's result body.
    assert!(fallback.response().contains("Task 0"));
    assert!(fallback.response().contains("412"));
    assert!(
        !fallback
            .response()
            .contains("No task results are available")
    );
    assert_eq!(runs.execution_count(), 1);
}

// ---------------------------------------------------------------------------
// Budget exhausted before anything executed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_exhausted_before_execution_lists_the_unexecuted_plan() {
    let expected_id = PlanId::derive(&plan_args());

    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &plan_args_json()),
        mock_tool_call_response(
            "c2",
            "execute",
            &format!(r#"{{"plan_id":"{expected_id}"}}"#),
        ),
    ];

    let coordinator = coordinator(responses, Vec::new(), 1).await;
    let outcome = coordinator.run(&goal()).await.expect("the loop runs");

    let CoordinatorOutcome::BudgetExhausted { fallback, turns } = outcome else {
        panic!("expected a host-authored answer, got {outcome:?}");
    };
    assert_eq!(turns, 1);
    assert!(
        fallback
            .response()
            .contains("ended before any plan was executed")
    );
    assert!(fallback.response().contains(expected_id.as_str()));
}

// ---------------------------------------------------------------------------
// Rejection observations
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unknown_worker_is_a_rejection_the_loop_survives() {
    let stray = CreatePlanArgs {
        goal: "Summarise yesterday's error spike".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "Collect the error counts".to_owned(),
            worker: Some("astrology".to_owned()),
        }],
        planning_rationale: "Wrong worker on purpose".to_owned(),
    };
    let stray_json = serde_json::to_string(&stray).expect("arguments serialize");

    let responses = vec![
        mock_tool_call_response("c1", "create_plan", &stray_json),
        mock_tool_call_response("c2", "create_plan", &plan_args_json()),
        mock_text_response("Recovered."),
    ];

    let coordinator = coordinator(responses, Vec::new(), 8).await;
    let runs = coordinator.runs().clone();
    let outcome = coordinator.run(&goal()).await.expect("the loop runs");

    // The rejected plan never reached the store; the revised one did, so the
    // loop continued past the rejection.
    assert_eq!(runs.plan_ids(), vec![PlanId::derive(&plan_args())]);
    assert_eq!(outcome.turns(), 2);
    assert!(matches!(
        outcome,
        CoordinatorOutcome::StoppedWithoutResponse { .. }
    ));
}

#[tokio::test]
async fn a_second_answer_is_refused_and_the_first_stands() {
    let responses = vec![
        mock_tool_call_response("c1", "respond", r#"{"response":"The first answer."}"#),
        mock_tool_call_response("c2", "respond", r#"{"response":"The second answer."}"#),
        mock_text_response(""),
    ];

    let coordinator = coordinator(responses, Vec::new(), 8).await;
    let outcome = coordinator.run(&goal()).await.expect("the loop runs");

    let CoordinatorOutcome::Responded { action, turns } = outcome else {
        panic!("expected a coordinator-authored answer, got {outcome:?}");
    };
    assert_eq!(turns, 2);
    assert_eq!(action.response(), "The first answer.");
}

// ---------------------------------------------------------------------------
// TerminalSlot
// ---------------------------------------------------------------------------

#[test]
fn terminal_slot_keeps_the_first_write() {
    let slot: TerminalSlot<FinalResponse> = TerminalSlot::new();
    assert!(!slot.is_recorded());

    let first = FinalResponse::new("first", None).expect("non-empty");
    let second = FinalResponse::new("second", Some("gloss")).expect("non-empty");

    slot.record(first.clone()).expect("the slot was empty");
    slot.record(second).expect_err("the slot was filled");

    assert!(slot.is_recorded());
    assert_eq!(slot.recorded(), Some(first));
}

#[test]
fn terminal_slot_clones_share_one_answer() {
    let slot: TerminalSlot<FinalResponse> = TerminalSlot::new();
    let handle = slot.clone();
    handle
        .record(FinalResponse::new("written through the clone", None).expect("non-empty"))
        .expect("the slot was empty");
    assert!(slot.is_recorded());
}

#[test]
fn a_blank_summary_is_the_same_as_no_summary() {
    let answer = FinalResponse::new("body", Some("   ")).expect("non-empty response");
    assert_eq!(answer.response_summary(), None);
    assert_eq!(
        FinalResponse::new("   ", None),
        Err(CoordinatorLoopError::EmptyFinalResponse)
    );
}

// ---------------------------------------------------------------------------
// PlanId
// ---------------------------------------------------------------------------

#[test]
fn plan_id_derives_the_same_id_for_the_same_plan() {
    let first = PlanId::derive(&plan_args());
    let second = PlanId::derive(&plan_args());
    assert_eq!(first, second);
}

#[test]
fn plan_id_ignores_the_rationale_and_reads_the_goal() {
    let mut rephrased = plan_args();
    rephrased.planning_rationale = "Completely different words".to_owned();
    assert_eq!(PlanId::derive(&plan_args()), PlanId::derive(&rephrased));

    let mut retargeted = plan_args();
    retargeted.goal = "Summarise last week's error spike".to_owned();
    assert_ne!(PlanId::derive(&plan_args()), PlanId::derive(&retargeted));
}

#[test]
fn every_derived_plan_id_round_trips_through_parse() {
    // Sweeps the small-digest range that `{:x}` would render short of the
    // sixteen characters `parse` demands.
    for nudge in 0..64u32 {
        let mut args = plan_args();
        args.goal = format!("goal {nudge}");
        let id = PlanId::derive(&args);
        assert_eq!(id.as_str().len(), PlanId::HEX_LEN, "id: {id}");
        assert_eq!(PlanId::parse(id.as_str()).expect("round trip"), id);
    }
}

#[test]
fn plan_id_parse_rejects_ids_no_derivation_could_produce() {
    assert_eq!(
        PlanId::parse("not-hex"),
        Err(CoordinatorLoopError::MalformedPlanId)
    );
    assert_eq!(
        PlanId::parse("0123456789ABCDEF"),
        Err(CoordinatorLoopError::MalformedPlanId),
        "uppercase hex is outside the derived alphabet"
    );
    assert_eq!(
        PlanId::parse("0123456789abcde"),
        Err(CoordinatorLoopError::MalformedPlanId),
        "fifteen characters is not the derived width"
    );
}

// ---------------------------------------------------------------------------
// interpret: the (stop reason, slot) table
// ---------------------------------------------------------------------------

fn outcome_with(stop_reason: LoopStopReason, text: &str, turns: u32) -> AgentOutcome {
    let mut response = CollectedResponse::new();
    response.content.push(ContentBlock::Text {
        text: text.to_owned(),
    });
    AgentOutcome {
        final_response: response,
        responses: Vec::new(),
        stop_reason,
        iterations: turns,
    }
}

fn filled_slot() -> TerminalSlot<FinalResponse> {
    let slot = TerminalSlot::new();
    slot.record(FinalResponse::new("committed", None).expect("non-empty"))
        .expect("empty slot");
    slot
}

#[test]
fn a_recorded_answer_outranks_every_stop_reason() {
    let runs = RunStore::new();
    for stop_reason in [
        LoopStopReason::EndTurn,
        LoopStopReason::MaxToolDepthReached,
        LoopStopReason::MaxTokens,
        LoopStopReason::StopSequence,
        LoopStopReason::ContentFilter,
        LoopStopReason::Cancelled,
    ] {
        let label = stop_reason.to_string();
        let outcome = CoordinatorOutcome::interpret(
            outcome_with(stop_reason, "trailing text", 4),
            &filled_slot(),
            &runs,
        );
        let CoordinatorOutcome::Responded { action, turns } = outcome else {
            panic!("expected slot-wins for {label}, got {outcome:?}");
        };
        assert_eq!(action.response(), "committed");
        assert_eq!(turns, 4);
    }
}

#[test]
fn an_empty_slot_reads_the_stop_reason() {
    let runs = RunStore::new();
    let empty: TerminalSlot<FinalResponse> = TerminalSlot::new();

    let ended = CoordinatorOutcome::interpret(
        outcome_with(LoopStopReason::EndTurn, "all done", 2),
        &empty,
        &runs,
    );
    let CoordinatorOutcome::StoppedWithoutResponse { last_text, turns } = ended else {
        panic!("expected a clean stop, got {ended:?}");
    };
    assert_eq!(last_text, "all done");
    assert_eq!(turns, 2);

    let exhausted = CoordinatorOutcome::interpret(
        outcome_with(LoopStopReason::MaxToolDepthReached, "", 2),
        &empty,
        &runs,
    );
    assert!(matches!(
        exhausted,
        CoordinatorOutcome::BudgetExhausted { .. }
    ));
}

#[test]
fn truncation_stop_reasons_become_interruptions() {
    let runs = RunStore::new();
    let empty: TerminalSlot<FinalResponse> = TerminalSlot::new();

    for (stop_reason, expected) in [
        (LoopStopReason::MaxTokens, InterruptionReason::TokenLimit),
        (
            LoopStopReason::StopSequence,
            InterruptionReason::StopSequence,
        ),
        (
            LoopStopReason::ContentFilter,
            InterruptionReason::ContentFilter,
        ),
    ] {
        let outcome =
            CoordinatorOutcome::interpret(outcome_with(stop_reason, "cut short", 1), &empty, &runs);
        let CoordinatorOutcome::Interrupted {
            reason, last_text, ..
        } = outcome
        else {
            panic!("expected an interruption, got {outcome:?}");
        };
        assert_eq!(reason, expected);
        assert_eq!(last_text, "cut short");
    }
}

#[test]
fn an_unmodelled_stop_reason_is_carried_verbatim() {
    let runs = RunStore::new();
    let empty: TerminalSlot<FinalResponse> = TerminalSlot::new();
    let outcome = CoordinatorOutcome::interpret(
        outcome_with(LoopStopReason::Cancelled, "", 1),
        &empty,
        &runs,
    );
    let CoordinatorOutcome::Interrupted { reason, .. } = outcome else {
        panic!("expected an interruption, got {outcome:?}");
    };
    assert_eq!(
        reason,
        InterruptionReason::Unclassified("cancelled".to_owned())
    );
}

// ---------------------------------------------------------------------------
// Plan parsing against the roster
// ---------------------------------------------------------------------------

#[test]
fn a_plan_may_only_assign_configured_workers() {
    let roster = roster();
    plan_args().to_plan(&roster).expect("configured worker");

    let stray = CreatePlanArgs {
        goal: "goal".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "task".to_owned(),
            worker: Some("astrology".to_owned()),
        }],
        planning_rationale: "rationale".to_owned(),
    };
    let rejected = stray.to_plan(&roster).expect_err("unconfigured worker");
    assert_eq!(
        rejected,
        CoordinatorLoopError::UnknownWorker {
            name: "astrology".to_owned(),
            available: "operations".to_owned(),
        }
    );
}

#[test]
fn an_empty_step_list_is_not_a_plan() {
    let empty = CreatePlanArgs {
        goal: "goal".to_owned(),
        steps: Vec::new(),
        planning_rationale: "rationale".to_owned(),
    };
    assert!(matches!(
        empty.to_plan(&roster()),
        Err(CoordinatorLoopError::UnexecutableSteps(_))
    ));
}

// ---------------------------------------------------------------------------
// OutcomeCounts and the observation task list
// ---------------------------------------------------------------------------

fn label(id: usize, worker: Option<&str>) -> CorrelationLabel {
    CorrelationLabel {
        task: TaskId::new(id),
        worker: worker.map(|name| WorkerRole::new(name).expect("non-empty role")),
    }
}

fn completed(id: usize, worker: Option<&str>) -> TaskObservation {
    TaskObservation::Completed {
        label: label(id, worker),
        evidence: EvidenceEntry::InlineResult {
            result: EvidenceText::new("Counted 530 errors across 4 services")
                .expect("non-empty evidence"),
            claim: Some(
                WorkerClaim::new("Checkout dominates the spike", Confidence::High)
                    .expect("non-empty summary"),
            ),
        },
        artifacts: Vec::new(),
    }
}

fn failed(id: usize, category: FailureCategory) -> TaskObservation {
    TaskObservation::Failed {
        label: label(id, Some("operations")),
        category,
        error: ErrorPreview::new("upstream timed out", ErrorPreviewWidth::DEFAULT),
        artifacts: Vec::new(),
    }
}

#[test]
fn the_tally_counts_soft_failures_inside_the_failures() {
    let tasks = vec![
        completed(0, Some("operations")),
        failed(1, FailureCategory::AgentTimeout),
        failed(2, FailureCategory::SoftFailure),
        TaskObservation::Blocked {
            label: label(3, None),
        },
    ];
    let counts = OutcomeCounts::tally(&tasks);
    assert_eq!(counts.completed(), 1);
    assert_eq!(counts.failed(), 2);
    assert_eq!(counts.soft_failed(), 1);
    assert_eq!(counts.blocked(), 1);
}

#[test]
fn a_completed_execution_must_observe_a_task() {
    assert_eq!(
        ExecutionObservation::completed(Vec::new()),
        Err(CoordinatorLoopError::EmptyTaskObservations)
    );
}

// ---------------------------------------------------------------------------
// Observation JSON
// ---------------------------------------------------------------------------

fn pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value).expect("observations are plain JSON data")
}

#[test]
fn plan_observation_json() {
    let plan = plan_args().to_plan(&roster()).expect("valid plan");
    let observation =
        PlanObservation::from_plan(PlanId::derive(&plan_args()), &plan).expect("non-empty plan");
    insta::assert_snapshot!("plan_observation", pretty(&observation));
}

#[test]
fn completed_execution_json_omits_absent_fields() {
    let observation = ExecutionObservation::completed(vec![
        completed(0, Some("operations")),
        TaskObservation::Blocked {
            label: label(1, None),
        },
    ])
    .expect("non-empty task list");
    insta::assert_snapshot!("completed_execution", pretty(&observation));
}

#[test]
fn failed_execution_json_carries_the_category_and_bounded_message() {
    let observation = ExecutionObservation::Failed {
        category: FailureCategory::ProviderOverloaded,
        message: ErrorPreview::new("provider returned 503", ErrorPreviewWidth::DEFAULT),
        tasks_observed: vec![failed(0, FailureCategory::AgentTimeout)],
    };
    insta::assert_snapshot!("failed_execution", pretty(&observation));
}

// ---------------------------------------------------------------------------
// Worker submission
// ---------------------------------------------------------------------------

#[test]
fn a_blank_worker_summary_is_not_a_submission() {
    use agent_driver_prototype::coordinator_loop::{SubmitResultArgs, WorkerSubmission};

    let blank = SubmitResultArgs {
        summary: "  ".to_owned(),
        result: "a real result".to_owned(),
        confidence: Confidence::Low,
    };
    assert!(matches!(
        WorkerSubmission::try_from(blank),
        Err(CoordinatorLoopError::UnusableSubmission(_))
    ));

    let good = SubmitResultArgs {
        summary: "Found the contributor".to_owned(),
        result: "checkout: 412".to_owned(),
        confidence: Confidence::High,
    };
    let submission = WorkerSubmission::try_from(good).expect("usable submission");
    assert_eq!(submission.claim().confidence(), Confidence::High);
    assert_eq!(submission.result().as_str(), "checkout: 412");
}

// ---------------------------------------------------------------------------
// Single-derivation byte parity: from_roster vs the producer oracle
// ---------------------------------------------------------------------------

/// A representative two-worker config with a vector store, so the Full
/// visibility path exercises tool descriptions. Both workers have valid
/// role names so the roster's `filter_map` drops nothing.
fn parity_config(visibility: ToolVisibility) -> OrchestrationConfig {
    let mut workers = HashMap::new();
    workers.insert(
        "operations".to_owned(),
        WorkerConfig {
            description: "Logs, pipelines and metrics".to_owned(),
            preamble: String::new(),
            mcp_filter: vec!["mezmo_*".to_owned()],
            vector_stores: vec!["runbooks".to_owned()],
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    workers.insert(
        "analyst".to_owned(),
        WorkerConfig {
            description: "Log and metric analysis".to_owned(),
            preamble: String::new(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    OrchestrationConfig {
        enabled: true,
        workers,
        tools_in_planning: visibility,
        max_tools_per_worker: 2,
        ..Default::default()
    }
}

fn parity_vector_stores() -> Vec<VectorStoreConfig> {
    vec![VectorStoreConfig::new(
        "runbooks",
        Some("Operational runbooks for the payments platform"),
    )]
}

/// For a representative config, `from_roster(WorkerRoster::from_config(...))`
/// produces the same three strings as the old `build_worker_prompt_sections`
/// oracle. Both paths read the same `config.workers` HashMap within one
/// process, so the iteration order is identical and the comparison is exact.
/// All three visibility modes are exercised, against both an MCP-less
/// inventory (the corpus path) and a populated one (the shim path), so the
/// two derivations cannot drift on either.
#[test]
fn from_roster_matches_the_producer_oracle_byte_for_byte() {
    let limit = ToolListLimit::new(10);
    let stores = parity_vector_stores();
    let inventories = [
        ToolInventory::empty(),
        ToolInventory::from_names(["mezmo_logs", "mezmo_pipelines", "keystrokes"]),
    ];

    for inventory in &inventories {
        for visibility in [
            ToolVisibility::None,
            ToolVisibility::Summary,
            ToolVisibility::Full,
        ] {
            let config = parity_config(visibility);

            let (oracle_section, oracle_field, oracle_guidelines) =
                build_worker_prompt_sections(&config, limit, &stores, inventory);

            let sections = WorkerSections::from_roster(
                WorkerRoster::from_config(&config, limit, &stores, inventory)
                    .expect("no parity worker configures a turn depth"),
            );

            assert_eq!(
                sections.roster_section(),
                oracle_section,
                "roster_section diverges for {visibility:?} over {inventory:?}"
            );
            assert_eq!(
                sections.worker_field(),
                oracle_field,
                "worker_field diverges for {visibility:?} over {inventory:?}"
            );
            assert_eq!(
                sections.guidelines(),
                oracle_guidelines,
                "guidelines diverge for {visibility:?} over {inventory:?}"
            );
        }
    }
}

/// A populated inventory reaches each worker's `WorkerSpec::tools` through
/// `mcp_filter`, and an MCP-less one leaves every worker with nothing.
///
/// This is the S74 defect at the roster seam: the shim's coordinator planned
/// against a roster that advertised zero tools for every worker, so it
/// reported a tool gap instead of dispatching terminal work.
#[test]
fn roster_resolves_worker_tools_from_the_advertised_inventory() {
    let limit = ToolListLimit::new(10);
    let stores = parity_vector_stores();
    let config = parity_config(ToolVisibility::Summary);
    let inventory = ToolInventory::from_names(["mezmo_logs", "mezmo_pipelines", "keystrokes"]);

    let roster = WorkerRoster::from_config(&config, limit, &stores, &inventory)
        .expect("no parity worker configures a turn depth");
    let tool_names = |role: &str| -> Vec<String> {
        roster
            .workers()
            .iter()
            .find(|spec| spec.role().as_str() == role)
            .unwrap_or_else(|| panic!("{role} is configured"))
            .tools()
            .iter()
            .map(|tool| tool.name().to_owned())
            .collect()
    };

    // `operations` filters on `mezmo_*` and owns the `runbooks` store.
    assert_eq!(
        tool_names("operations"),
        vec![
            "mezmo_logs".to_owned(),
            "mezmo_pipelines".to_owned(),
            "vector_search_runbooks".to_owned(),
        ],
        "the mezmo_* filter selects both advertised mezmo tools, and the \
         configured vector store is appended"
    );
    // `analyst` has an empty filter, which means every advertised tool.
    assert_eq!(
        tool_names("analyst"),
        inventory.names(),
        "an empty mcp_filter resolves the whole advertised inventory"
    );

    let mcp_less = WorkerRoster::from_config(&config, limit, &stores, &ToolInventory::empty())
        .expect("no parity worker configures a turn depth");
    let mcp_less_names: Vec<&str> = mcp_less
        .workers()
        .iter()
        .find(|spec| spec.role().as_str() == "analyst")
        .expect("analyst is configured")
        .tools()
        .iter()
        .map(|tool| tool.name())
        .collect();
    assert!(
        mcp_less_names.is_empty(),
        "the MCP-less inventory leaves the corpus path resolving nothing"
    );
}

/// A worker's configured `turn_depth` reaches its `WorkerSpec` as a budget,
/// a worker without one carries `None` for the executor to fall back on, and
/// a zero depth is rejected at the parse rather than reaching a worker that
/// could take no turn.
#[test]
fn roster_carries_per_worker_turn_depth_and_rejects_zero() {
    let limit = ToolListLimit::new(10);

    let spec_with_depth = |depth: Option<usize>| WorkerConfig {
        description: "Verification".to_owned(),
        preamble: String::new(),
        mcp_filter: Vec::new(),
        vector_stores: Vec::new(),
        turn_depth: depth,
        llm: None,
        scratchpad: None,
        skills: None,
    };

    let mut workers = HashMap::new();
    workers.insert("verifier".to_owned(), spec_with_depth(Some(16)));
    workers.insert("analyst".to_owned(), spec_with_depth(None));
    let config = OrchestrationConfig {
        enabled: true,
        workers,
        ..Default::default()
    };

    let roster = WorkerRoster::from_config(&config, limit, &[], &ToolInventory::empty())
        .expect("16 is a spendable turn depth");
    let budget = |role: &str| {
        roster
            .workers()
            .iter()
            .find(|spec| spec.role().as_str() == role)
            .unwrap_or_else(|| panic!("{role} is configured"))
            .budget()
    };
    assert_eq!(
        budget("verifier").map(|b| b.turns()),
        Some(16),
        "the configured depth reaches the spec"
    );
    assert_eq!(
        budget("analyst"),
        None,
        "a worker with no configured depth defers to the run-wide budget"
    );

    let mut zero_workers = HashMap::new();
    zero_workers.insert("verifier".to_owned(), spec_with_depth(Some(0)));
    let zero_config = OrchestrationConfig {
        enabled: true,
        workers: zero_workers,
        ..Default::default()
    };
    assert!(
        matches!(
            WorkerRoster::from_config(&zero_config, limit, &[], &ToolInventory::empty()),
            Err(CoordinatorLoopError::ZeroTurnBudget)
        ),
        "a zero turn depth is rejected at the roster parse"
    );
}

/// A configured turn depth wider than the budget's `u32` is rejected rather
/// than saturated to `u32::MAX`.
///
/// Saturating turns a typo into a worker that runs four billion turns deep,
/// and the config that caused it is nowhere near the eventual stop. The parse
/// is the only place the original number is still in hand.
#[test]
fn roster_rejects_a_turn_depth_wider_than_the_budget() {
    let out_of_range = usize::try_from(u32::MAX).expect("a 64-bit usize") + 1;
    let mut workers = HashMap::new();
    workers.insert(
        "verifier".to_owned(),
        WorkerConfig {
            description: "Verification".to_owned(),
            preamble: String::new(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth: Some(out_of_range),
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    let config = OrchestrationConfig {
        enabled: true,
        workers,
        ..Default::default()
    };

    assert_eq!(
        WorkerRoster::from_config(
            &config,
            ToolListLimit::new(10),
            &[],
            &ToolInventory::empty()
        ),
        Err(CoordinatorLoopError::TurnDepthOutOfRange(out_of_range)),
        "an out-of-range turn depth fails loud at the parse, carrying the \
         configured number"
    );
}

/// A worker whose name the role parse refuses fails the roster build rather
/// than disappearing from it.
///
/// A dropped worker leaves the planning schema offering a shorter roster than
/// the configuration named, and the only symptom is a coordinator that never
/// assigns the missing worker any work.
#[test]
fn roster_rejects_a_worker_name_that_is_not_a_role() {
    let mut workers = HashMap::new();
    workers.insert(
        "   ".to_owned(),
        WorkerConfig {
            description: "Nameless".to_owned(),
            preamble: String::new(),
            mcp_filter: Vec::new(),
            vector_stores: Vec::new(),
            turn_depth: None,
            llm: None,
            scratchpad: None,
            skills: None,
        },
    );
    let config = OrchestrationConfig {
        enabled: true,
        workers,
        ..Default::default()
    };

    let error = WorkerRoster::from_config(
        &config,
        ToolListLimit::new(10),
        &[],
        &ToolInventory::empty(),
    )
    .expect_err("a whitespace-only worker name is not a role");
    assert!(
        matches!(error, CoordinatorLoopError::UnusableWorkerName(_)),
        "the rejection travels through the roster's own error channel, got: {error}"
    );
}

/// An empty roster renders three empty strings, matching the oracle's
/// no-workers path.
#[test]
fn from_roster_with_no_workers_renders_empty_sections() {
    let config = OrchestrationConfig::default();
    let limit = ToolListLimit::new(10);
    let (oracle_section, oracle_field, oracle_guidelines) =
        build_worker_prompt_sections(&config, limit, &[], &ToolInventory::empty());
    let sections = WorkerSections::from_roster(
        WorkerRoster::from_config(&config, limit, &[], &ToolInventory::empty())
            .expect("an empty config configures no turn depth"),
    );
    assert_eq!(sections.roster_section(), oracle_section);
    assert_eq!(sections.worker_field(), oracle_field);
    assert_eq!(sections.guidelines(), oracle_guidelines);
    assert!(sections.roster_section().is_empty());
}

// ---------------------------------------------------------------------------
// R6: golden pinning the loop-shaped planning message through from_roster
// ---------------------------------------------------------------------------

/// The rendered loop-shaped planning message through the `from_roster`
/// sections, pinned by an insta snapshot. A single-worker config keeps the
/// roster deterministic across process runs (HashMap iteration order is
/// trivial with one entry). The timestamp is fixed so the snapshot does not
/// depend on wall-clock time.
#[test]
fn planning_loop_message_through_from_roster() {
    let sections = test_sections();
    let message = render_planning_loop_prompt(&PlanningLoopVars {
        timestamp: "2026-07-27T12:00:00Z",
        query: "Summarise yesterday's error spike",
        worker_section: sections.roster_section(),
        worker_guidelines: sections.guidelines(),
    });
    insta::assert_snapshot!("planning_loop_message", message);
}
