//! Integration tests for the DAG executor with MockProvider-backed workers.
//!
//! The mock provider arithmetic is exact: the mock panics when its queue is
//! exhausted, so reaching the assertions proves the executor made exactly the
//! queued number of provider calls.

use std::collections::HashMap;
use std::sync::Arc;

use agent_driver_rs::provider::mock::{MockProvider, mock_text_response, mock_tool_call_response};
use agent_driver_rs::tool::ToolContext;
use agent_driver_rs::types::{ModelId, SystemPrompt};
use tokio_util::sync::CancellationToken;

use agent_driver_prototype::artifacts::{ArtifactFilename, ArtifactStore, InlineThreshold};
use agent_driver_prototype::bounding::ToolListLimit;
use agent_driver_prototype::config::{OrchestrationConfig, WorkerConfig};
use agent_driver_prototype::context::{ErrorPreview, EvidenceEntry};
use agent_driver_prototype::coordinator_loop::{
    Attempt, CreatePlanArgs, ExecutionObservation, LoopBudget, PlanExecutor, RunStore,
    TaskObservation, WorkerRoster, WorkerSections,
};
use agent_driver_prototype::dag_executor::{DagExecutor, WorkerLoopConfig};
use agent_driver_prototype::mcp_client::SidecarClient;
use agent_driver_prototype::types::{FailureCategory, StepInput};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn model() -> ModelId {
    ModelId::new("mock-model").expect("valid model id")
}

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
    WorkerSections::from_roster(WorkerRoster::from_config(
        &config,
        ToolListLimit::new(10),
        &[],
    ))
}

/// The plan arguments for a two-task sequential workflow.
///
/// `flatten_steps` (`src/types.rs`) wires a sequential list of `LeafTask`
/// steps so that each step depends on the previous frontier: task 0 has
/// no dependencies, task 1 gets `dependencies = [0]`. The two tasks are
/// not independent — task 1 cannot run until task 0 completes.
fn two_task_args() -> CreatePlanArgs {
    CreatePlanArgs {
        goal: "Complete the two-task workflow".to_owned(),
        steps: vec![
            StepInput::LeafTask {
                task: "Collect the data".to_owned(),
                worker: Some("operations".to_owned()),
            },
            StepInput::LeafTask {
                task: "Summarize the data".to_owned(),
                worker: None,
            },
        ],
        planning_rationale: "Sequential: collect then summarise".to_owned(),
    }
}

fn ctx() -> ToolContext {
    ToolContext::new(CancellationToken::new())
}

fn worker_config(responses: Vec<Vec<agent_driver_rs::StreamEvent>>) -> WorkerLoopConfig {
    WorkerLoopConfig {
        provider: Arc::new(MockProvider::new(responses)),
        model: model(),
        budget: LoopBudget::new(8).expect("non-zero budget"),
        system_prompt: SystemPrompt::new("You are a worker. Call submit_result when done."),
    }
}

fn submit_result_json(summary: &str, result: &str, confidence: &str) -> String {
    format!(r#"{{"summary":"{summary}","result":"{result}","confidence":"{confidence}"}}"#)
}

// ---------------------------------------------------------------------------
// Card acceptance test: two-task DAG runs to completion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_task_dag_with_dependency_runs_to_completion() {
    let runs = RunStore::new();
    let args = two_task_args();
    let plan = args
        .to_plan(&test_sections().roster().clone())
        .expect("valid plan");
    let plan_id = runs.record_plan(&args, plan.clone());

    // flatten_steps wires sequential dependencies: task 1 depends on task 0.
    assert_eq!(plan.tasks[0].dependencies, Vec::<usize>::new());
    assert_eq!(plan.tasks[1].dependencies, vec![0]);

    // 4 queued responses, 4 provider calls. Task 0: submit_result + end-turn.
    // Task 1: submit_result + end-turn. A fifth call would panic the mock.
    let responses = vec![
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("Found 42 records", "Detailed data output", "high"),
        ),
        mock_text_response(""),
        mock_tool_call_response(
            "w1",
            "submit_result",
            &submit_result_json("Summary complete", "Based on the collected data", "medium"),
        ),
        mock_text_response(""),
    ];

    let dir = tempfile::TempDir::new().expect("temp dir");
    let executor = DagExecutor::new(
        SidecarClient::disconnected(),
        ArtifactStore::new(dir.path().to_path_buf()),
        worker_config(responses),
        test_sections(),
        runs.clone(),
        InlineThreshold::DEFAULT,
        None,
    );

    let observation = executor.execute(&plan, &ctx()).await;

    let ExecutionObservation::Completed { tasks } = &observation else {
        panic!("expected completed execution, got {observation:?}");
    };
    let tasks = tasks.as_slice();
    assert_eq!(tasks.len(), 2);

    // Task 0 completed with inline evidence.
    let TaskObservation::Completed {
        evidence,
        artifacts,
        ..
    } = &tasks[0]
    else {
        panic!("expected task 0 completed, got {:?}", tasks[0]);
    };
    assert!(artifacts.is_empty(), "inline result has no artifacts");
    assert!(
        matches!(evidence, EvidenceEntry::InlineResult { .. }),
        "short result stays inline"
    );

    // Task 1 completed with inline evidence.
    assert!(
        matches!(tasks[1], TaskObservation::Completed { .. }),
        "expected task 1 completed, got {:?}",
        tasks[1]
    );

    // Attempt numbering: task records filed with attempt 1.
    let attempt = Attempt::new(1).expect("1 is non-zero");
    let record0 = runs
        .task(&plan_id, 0, attempt)
        .expect("task 0 record filed");
    assert_eq!(record0.attempt().get(), 1);
    let record1 = runs
        .task(&plan_id, 1, attempt)
        .expect("task 1 record filed");
    assert_eq!(record1.attempt().get(), 1);
}

// ---------------------------------------------------------------------------
// Spill test: long result spills to artifact, full body is retrievable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spilled_full_body_is_retrievable_via_artifact_handle() {
    let runs = RunStore::new();
    let args = two_task_args();
    let plan = args
        .to_plan(&test_sections().roster().clone())
        .expect("valid plan");
    let _plan_id = runs.record_plan(&args, plan.clone());

    // flatten_steps wires sequential dependencies: task 1 depends on task 0.
    assert_eq!(plan.tasks[1].dependencies, vec![0]);

    let long_result = "X".repeat(50);
    let responses = vec![
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("Long output", &long_result, "high"),
        ),
        mock_text_response(""),
        mock_tool_call_response(
            "w1",
            "submit_result",
            &submit_result_json("Summary", "short", "medium"),
        ),
        mock_text_response(""),
    ];

    let dir = tempfile::TempDir::new().expect("temp dir");
    let store = ArtifactStore::new(dir.path().to_path_buf());
    let executor = DagExecutor::new(
        SidecarClient::disconnected(),
        store.clone(),
        worker_config(responses),
        test_sections(),
        runs,
        InlineThreshold::new(10).expect("non-zero threshold"),
        None,
    );

    let observation = executor.execute(&plan, &ctx()).await;

    let ExecutionObservation::Completed { tasks } = &observation else {
        panic!("expected completed execution, got {observation:?}");
    };
    let tasks = tasks.as_slice();

    // Task 0: spilled — ArtifactPointer evidence, one artifact handle.
    let TaskObservation::Completed {
        evidence,
        artifacts,
        ..
    } = &tasks[0]
    else {
        panic!("expected task 0 completed, got {:?}", tasks[0]);
    };
    assert!(
        matches!(evidence, EvidenceEntry::ArtifactPointer { .. }),
        "long result must spill"
    );
    assert_eq!(
        artifacts.len(),
        1,
        "spilled result carries one artifact handle"
    );

    // The full body is retrievable via the artifact store.
    let filename = ArtifactFilename::new(artifacts[0].filename()).expect("valid filename");
    let full_body = store
        .read_artifact(&filename)
        .await
        .expect("read spilled body");
    assert_eq!(full_body, long_result);

    // Task 1: short result stays inline.
    let TaskObservation::Completed {
        evidence: ev1,
        artifacts: art1,
        ..
    } = &tasks[1]
    else {
        panic!("expected task 1 completed, got {:?}", tasks[1]);
    };
    assert!(art1.is_empty());
    assert!(
        matches!(ev1, EvidenceEntry::InlineResult { .. }),
        "short result stays inline"
    );
}

// ---------------------------------------------------------------------------
// Dependency failure: blocked task carries no failure category
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dependency_failure_blocks_descendant_without_failure_category() {
    let runs = RunStore::new();
    let args = two_task_args();
    let plan = args
        .to_plan(&test_sections().roster().clone())
        .expect("valid plan");
    let _plan_id = runs.record_plan(&args, plan.clone());

    // 1 response: task 0's worker stops without submitting (text only, no
    // tool call). Task 1 never runs — it is blocked by task 0's failure.
    // The single-response queue is load-bearing: if task 1 were
    // independent (no dependency on task 0), the executor would dispatch
    // it next, consume a second worker response, and panic the mock.
    let responses = vec![mock_text_response("I cannot complete this task.")];

    let dir = tempfile::TempDir::new().expect("temp dir");
    let executor = DagExecutor::new(
        SidecarClient::disconnected(),
        ArtifactStore::new(dir.path().to_path_buf()),
        worker_config(responses),
        test_sections(),
        runs,
        InlineThreshold::DEFAULT,
        None,
    );

    let observation = executor.execute(&plan, &ctx()).await;

    let ExecutionObservation::Completed { tasks } = &observation else {
        panic!("expected completed execution, got {observation:?}");
    };
    let tasks = tasks.as_slice();
    assert_eq!(tasks.len(), 2);

    // Task 0 failed with DepthExhausted (StoppedWithoutSubmission mapping).
    let TaskObservation::Failed { category, .. } = &tasks[0] else {
        panic!("expected task 0 failed, got {:?}", tasks[0]);
    };
    assert_eq!(*category, FailureCategory::DepthExhausted);

    // Task 1 blocked — no failure category.
    match &tasks[1] {
        TaskObservation::Blocked { .. } => {}
        other => panic!("expected task 1 blocked, got {other:?}"),
    }
    assert!(
        tasks[1].failure_category().is_none(),
        "blocked task carries no failure category"
    );
}

// ---------------------------------------------------------------------------
// WorkerOutcome mapping: BudgetExhausted -> DepthExhausted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn budget_exhausted_maps_to_depth_exhausted() {
    let runs = RunStore::new();
    let args = CreatePlanArgs {
        goal: "Single task budget test".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "Do the work".to_owned(),
            worker: Some("operations".to_owned()),
        }],
        planning_rationale: "One task".to_owned(),
    };
    let plan = args
        .to_plan(&test_sections().roster().clone())
        .expect("valid plan");
    let _plan_id = runs.record_plan(&args, plan.clone());

    // Budget 1: the worker calls submit_result with an empty summary (rejected
    // by the tool), then calls again. The second round hits the depth limit
    // before executing, so the loop stops with MaxToolDepthReached. 2 provider
    // calls total.
    let responses = vec![
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("  ", "result", "high"),
        ),
        mock_tool_call_response(
            "w1",
            "submit_result",
            &submit_result_json("  ", "result", "high"),
        ),
    ];

    let config = WorkerLoopConfig {
        provider: Arc::new(MockProvider::new(responses)),
        model: model(),
        budget: LoopBudget::new(1).expect("non-zero budget"),
        system_prompt: SystemPrompt::new("You are a worker."),
    };

    let dir = tempfile::TempDir::new().expect("temp dir");
    let executor = DagExecutor::new(
        SidecarClient::disconnected(),
        ArtifactStore::new(dir.path().to_path_buf()),
        config,
        test_sections(),
        runs,
        InlineThreshold::DEFAULT,
        None,
    );

    let observation = executor.execute(&plan, &ctx()).await;

    let ExecutionObservation::Completed { tasks } = &observation else {
        panic!("expected completed execution, got {observation:?}");
    };
    let tasks = tasks.as_slice();
    assert_eq!(tasks.len(), 1);

    let TaskObservation::Failed { category, .. } = &tasks[0] else {
        panic!("expected task 0 failed, got {:?}", tasks[0]);
    };
    assert_eq!(
        *category,
        FailureCategory::DepthExhausted,
        "BudgetExhausted maps to DepthExhausted"
    );
}

// ---------------------------------------------------------------------------
// Spill failure: disabled store with oversize result produces bounded Failed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spill_failure_with_disabled_store_produces_bounded_failed_observation() {
    let runs = RunStore::new();
    let args = CreatePlanArgs {
        goal: "Spill failure test".to_owned(),
        steps: vec![StepInput::LeafTask {
            task: "Produce a large result".to_owned(),
            worker: Some("operations".to_owned()),
        }],
        planning_rationale: "One task".to_owned(),
    };
    let plan = args
        .to_plan(&test_sections().roster().clone())
        .expect("valid plan");
    let _plan_id = runs.record_plan(&args, plan.clone());

    let long_result = "X".repeat(50);
    let responses = vec![
        mock_tool_call_response(
            "w0",
            "submit_result",
            &submit_result_json("Long output", &long_result, "high"),
        ),
        mock_text_response(""),
    ];

    let executor = DagExecutor::new(
        SidecarClient::disconnected(),
        ArtifactStore::disabled(),
        worker_config(responses),
        test_sections(),
        runs,
        InlineThreshold::new(10).expect("non-zero threshold"),
        None,
    );

    let observation = executor.execute(&plan, &ctx()).await;

    let ExecutionObservation::Completed { tasks } = &observation else {
        panic!("expected completed execution, got {observation:?}");
    };
    let tasks = tasks.as_slice();
    assert_eq!(tasks.len(), 1);

    let TaskObservation::Failed {
        category,
        error,
        artifacts,
        ..
    } = &tasks[0]
    else {
        panic!("expected task 0 failed, got {:?}", tasks[0]);
    };

    assert_eq!(
        *category,
        FailureCategory::AgentError,
        "spill failure maps to AgentError"
    );
    assert!(
        artifacts.is_empty(),
        "failed spill produces no artifact handles"
    );

    let preview_text = error.to_string();
    assert!(
        preview_text.contains("artifact write failed"),
        "failure preview must name the spill failure: {preview_text}"
    );
    assert!(
        preview_text.contains("disabled"),
        "failure preview must carry the underlying error: {preview_text}"
    );

    assert!(
        !preview_text.contains(&long_result),
        "the full unbounded result body must NOT appear in the failure preview"
    );
    assert!(
        error.as_str().chars().count() <= ErrorPreview::MAX_CHARS,
        "failure preview must be bounded"
    );
}
