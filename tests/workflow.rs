//! Offline integration tests for the deterministic workflow executor.
//!
//! Six scenarios over the `test-support` scripted-server rig, one per
//! acceptance line on card W3: in-order apply with captured exports;
//! step failure with reverse-order unwind; rollback failure with a loud
//! stop and a residual set; bounds rejection at resolve time; a binding
//! miss on a missing export path; and cancellation mid-apply.
//!
//! W4 adds four approval-wire legs and one end-to-end approval-gates-apply
//! leg against an in-process axum approval receiver.

#![allow(clippy::unused_async)]

use agent_driver_prototype::mcp_client::{SidecarTool, SidecarToolName};
use agent_driver_prototype::workflow::{
    ApprovalClient, ApprovalOutcome, ApprovalPayload, DecisionId, ExecuteError,
    ProposeWorkflowTool, ResolveError, RollbackOutcome, RunOutcome, RunRecord, StepId, StepStatus,
    ValidatedWorkflowSpec, WorkflowSpec, execute_workflow,
};
use agent_driver_rs::tool::{Tool, ToolContext, ToolInput};
use serde_json::{Value as JsonValue, json};
use tokio_util::sync::CancellationToken;

// ============================================================================
// The rig: an in-process rmcp server over an in-memory duplex
// ============================================================================

mod rig {
    use std::sync::Arc;

    use agent_driver_prototype::mcp_client::SidecarClient;
    use rmcp::{
        ErrorData as McpError, RoleServer, ServerHandler, ServiceExt as _,
        model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
            ServerCapabilities, ServerInfo,
        },
        service::RequestContext,
    };
    use serde_json::{Value as JsonValue, json};
    use tokio::sync::Mutex;

    /// The scripted answer function: tool name and args in, JSON result
    /// or error string out.
    type Script = Arc<dyn Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync>;

    /// A scripted MCP server: advertises a fixed tool list and answers
    /// `tools/call` according to the script.
    #[derive(Clone)]
    pub struct ScriptedServer {
        info: ServerInfo,
        tools: Vec<JsonValue>,
        script: Script,
        calls: Arc<Mutex<Vec<(String, JsonValue)>>>,
    }

    impl ScriptedServer {
        pub fn new(
            tools: Vec<JsonValue>,
            script: impl Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync + 'static,
        ) -> Self {
            Self {
                info: ServerInfo::new(ServerCapabilities::builder().enable_tools().build()),
                tools,
                script: Arc::new(script),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub async fn calls(&self) -> Vec<(String, JsonValue)> {
            self.calls.lock().await.clone()
        }
    }

    impl ServerHandler for ScriptedServer {
        fn get_info(&self) -> ServerInfo {
            self.info.clone()
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, McpError> {
            let tools = self
                .tools
                .iter()
                .map(|raw| {
                    serde_json::from_value(raw.clone())
                        .expect("rig tool fixture deserializes as rmcp Tool")
                })
                .collect();
            Ok(ListToolsResult {
                tools,
                ..ListToolsResult::default()
            })
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, McpError> {
            let arguments = request.arguments.clone().unwrap_or_default();
            let args = serde_json::to_value(arguments).unwrap_or(JsonValue::Null);
            self.calls
                .lock()
                .await
                .push((request.name.to_string(), args.clone()));
            match (self.script)(&request.name, &args) {
                Ok(value) => {
                    Ok(CallToolResult::success(vec![ContentBlock::text(value.to_string())]).into())
                }
                Err(message) => Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into()),
            }
        }
    }

    /// Serve the rig and connect a real client over the other half.
    ///
    /// The server half is spawned, not awaited: `serve` only completes
    /// once the client's initialize round trip runs, so awaiting it
    /// before connecting deadlocks the pair. The returned handle keeps
    /// the server alive; dropping or ignoring it ends it with the test.
    pub async fn boot(
        server: ScriptedServer,
    ) -> (
        SidecarClient,
        tokio::task::JoinHandle<rmcp::service::RunningService<rmcp::RoleServer, ScriptedServer>>,
    ) {
        use rmcp::transport::IntoTransport as _;

        let (client_half, server_half) = tokio::io::duplex(4096);
        let served = tokio::spawn(async move {
            server
                .serve(server_half.into_transport())
                .await
                .expect("rig server serves its half")
        });
        let client = SidecarClient::connect_stream(client_half)
            .await
            .expect("rig client completes the handshake");
        (client, served)
    }

    /// The `tools/list` entry shape the rig advertises: name,
    /// description, and `inputSchema`, exactly as a real server sends.
    pub fn tool_entry(name: &str, description: &str, schema: JsonValue) -> JsonValue {
        json!({ "name": name, "description": description, "inputSchema": schema })
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn permissive_schema() -> JsonValue {
    json!({ "type": "object" })
}

fn validate(workflow: JsonValue, tools: &[SidecarTool]) -> ValidatedWorkflowSpec {
    let spec: WorkflowSpec = serde_json::from_value(workflow).expect("workflow parses");
    spec.validate(tools).expect("workflow validates")
}

/// The discovered inventory every scenario validates against. Both ops
/// tools take the permissive object schema: these scenarios exercise the
/// executor's apply/unwind semantics, not propose-time schema checking
/// (W1's covered ground).
fn inventory() -> Vec<SidecarTool> {
    let schema = permissive_schema();
    vec![
        SidecarTool::new(
            SidecarToolName::new("ops_get_cluster_state").expect("tool name is non-empty"),
            String::new(),
            schema.clone(),
        ),
        SidecarTool::new(
            SidecarToolName::new("ops_scale_app").expect("tool name is non-empty"),
            String::new(),
            schema,
        ),
    ]
}

/// The `tools/list` entries the rig advertises, mirroring the inventory
/// `validate` checked against — a real sidecar advertises what proposal
/// discovered.
fn advertised() -> Vec<JsonValue> {
    let schema = permissive_schema();
    vec![
        rig::tool_entry(
            "ops_get_cluster_state",
            "read the cluster state",
            schema.clone(),
        ),
        rig::tool_entry("ops_scale_app", "scale an app's deployment", schema),
    ]
}

/// The scripted cluster-state read: three replicas under
/// `$.deployment.replicas`.
fn read_result() -> JsonValue {
    json!({"deployment": {"replicas": 3}})
}

/// The scripted scale result: the applied replica count at `$.replicas`.
fn scaled_result() -> JsonValue {
    json!({"status": "scaled", "replicas": 6})
}

/// The read step every scenario opens with: it exports the deployment's
/// replica count as `state.current_replicas`. `rollback` is `json!(null)`
/// for scenarios that need no observable unwind, and [`state_rollback`]
/// where a dispatched or withheld unwind attempt must be visible in the
/// call log.
fn state_step(rollback: JsonValue) -> JsonValue {
    json!({
        "id": "state",
        "dependencies": [],
        "tool": "ops_get_cluster_state",
        "args": {"app": "payments"},
        "exports": {"current_replicas": "$.deployment.replicas"},
        "rollback": rollback
    })
}

/// The state step's declared rollback: a literal rescale to 99. The
/// out-of-band replica count is the rollback's signature in every
/// scripted answer fn and call-log assertion.
fn state_rollback() -> JsonValue {
    json!({
        "tool": "ops_scale_app",
        "args": {"app": "payments", "replicas": 99}
    })
}

/// The scale step's declared rollback: a `$from`-bound rescale back to
/// the captured original replica count, inside the demo's 1..20 envelope.
fn bound_scale_rollback() -> JsonValue {
    json!({
        "tool": "ops_scale_app",
        "args": {"app": "payments",
                 "replicas": {"$from": "state.current_replicas", "min": 1, "max": 20}}
    })
}

/// Validate `workflow` against the permissive inventory, boot the
/// scripted server behind a real client, and execute the workflow
/// against the pair.
///
/// Returns the run record and a clone of the server sharing the call
/// log (the server itself is moved into its serving task by `boot`).
async fn run(
    script: impl Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync + 'static,
    workflow: JsonValue,
    cancel: &CancellationToken,
) -> (RunRecord, rig::ScriptedServer) {
    let validated = validate(workflow, &inventory());
    let server = rig::ScriptedServer::new(advertised(), script);
    let recorder = server.clone();
    let (client, _served) = rig::boot(server).await;
    let record = execute_workflow(&validated, &client, cancel)
        .await
        .expect("the executor returns the run record for every scripted outcome");
    (record, recorder)
}

/// The run record's per-step statuses, in declaration order.
fn statuses(record: &RunRecord) -> Vec<StepStatus> {
    record.steps.iter().map(|step| step.status).collect()
}

// ============================================================================
// Acceptance scenarios
// ============================================================================

/// Card acceptance 1: steps apply in declaration (topological) order and
/// the exports environment materializes — proven by behavior. The scale
/// step's spec carries only a `$from` reference where `replicas` goes,
/// so a dispatched `replicas: 3` can only have come from capturing the
/// state step's result; likewise `verify`'s `expected_replicas: 6` can
/// only have come from capturing scale's result.
#[tokio::test]
async fn success_steps_apply_in_order_and_exports_captured() {
    let cancel = CancellationToken::new();
    let workflow = json!({
        "goal": "scale payments and verify",
        "steps": [
            state_step(json!(null)),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments",
                         "replicas": {"$from": "state.current_replicas", "min": 1, "max": 20}},
                "exports": {"applied": "$.replicas"},
                "rollback": bound_scale_rollback()
            },
            {
                "id": "verify",
                "dependencies": ["scale"],
                "tool": "ops_get_cluster_state",
                "args": {"app": "verify", "expected_replicas": {"$from": "scale.applied"}},
                "exports": {},
                "rollback": null
            }
        ]
    });
    let (record, server) = run(
        |name, args| match name {
            "ops_get_cluster_state" => Ok(read_result()),
            "ops_scale_app" if args.get("replicas") != Some(&json!(3)) => Err(format!(
                "scale dispatched with {args}; expected the captured replicas 3"
            )),
            "ops_scale_app" => Ok(scaled_result()),
            other => Err(format!("unexpected tool call {other}")),
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(record.goal, "scale payments and verify");
    assert_eq!(
        statuses(&record),
        vec![
            StepStatus::Applied,
            StepStatus::Applied,
            StepStatus::Applied
        ]
    );
    assert!(matches!(record.outcome, RunOutcome::Complete));

    // The dispatch order and the resolved arguments: the apply ran
    // state -> scale -> verify, and both `$from` bindings substituted
    // the values captured from the earlier steps' results.
    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 3})
            ),
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "verify", "expected_replicas": 6})
            ),
        ]
    );
}

/// Card acceptance 2: a step failure unwinds the completed steps in
/// REVERSE completion order. Three steps complete (state, scale,
/// resize) before the fourth fails, each carrying a rollback with a
/// distinct dispatch signature, so the call log proves the unwind
/// order (resize's 98, then scale's bound 3, then state's 99) rather
/// than leaning on a single-completed-step case.
#[tokio::test]
async fn step_failure_runs_reverse_order_unwind() {
    let cancel = CancellationToken::new();
    let workflow = json!({
        "goal": "scale payments",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            },
            {
                "id": "resize",
                "dependencies": ["scale"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 7},
                "exports": {},
                "rollback": {
                    "tool": "ops_scale_app",
                    "args": {"app": "payments", "replicas": 98}
                }
            },
            {
                "id": "verify",
                "dependencies": ["resize"],
                "tool": "ops_get_cluster_state",
                "args": {"app": "verify"},
                "exports": {},
                "rollback": null
            }
        ]
    });
    let (record, server) = run(
        |name, args| match name {
            "ops_get_cluster_state" if args.get("app") == Some(&json!("verify")) => {
                Err("the cluster rejected the verification read".to_owned())
            }
            "ops_get_cluster_state" => Ok(read_result()),
            "ops_scale_app" => Ok(scaled_result()),
            other => Err(format!("unexpected tool call {other}")),
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![
            StepStatus::Unwound,
            StepStatus::Unwound,
            StepStatus::Unwound,
            StepStatus::Failed
        ]
    );
    match &record.outcome {
        RunOutcome::Failed {
            step,
            error,
            rollback_failure,
            residual,
        } => {
            assert_eq!(step.as_str(), "verify");
            assert!(
                error.to_string().contains("verification read"),
                "the step failure names its cause: {error}"
            );
            assert!(rollback_failure.is_none(), "the unwind itself succeeded");
            assert!(residual.is_empty(), "every applied step was unwound");
        }
        other => panic!("expected a Failed outcome, got {other:?}"),
    }
    assert_eq!(record.steps[0].rollback, Some(RollbackOutcome::Success));
    assert_eq!(record.steps[1].rollback, Some(RollbackOutcome::Success));
    assert_eq!(record.steps[2].rollback, Some(RollbackOutcome::Success));
    assert_eq!(
        record.steps[3].rollback, None,
        "only applied steps unwind; the failing step has no rollback attempt"
    );

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 7})
            ),
            ("ops_get_cluster_state".to_owned(), json!({"app": "verify"})),
            // The unwind, in reverse completion order: resize, scale,
            // state (98, then the bound 3, then 99).
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 98})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 3})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 99})
            ),
        ],
        "the unwind dispatches rollbacks in reverse completion order"
    );
}

/// Card acceptance 3: with two steps applied, a third-step failure
/// starts the unwind at the most recently applied step (reverse
/// completion order). That rollback fails loudly; the unwind STOPS —
/// the first step's rollback is never dispatched, proven by the call
/// log — and the outcome carries both failures plus the residual
/// applied set naming step one.
#[tokio::test]
async fn rollback_failure_reports_both_failures_and_residual_set() {
    let cancel = CancellationToken::new();
    let workflow = json!({
        "goal": "scale payments and verify",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            },
            {
                "id": "verify",
                "dependencies": ["scale"],
                "tool": "ops_get_cluster_state",
                "args": {"app": "verify"},
                "exports": {},
                "rollback": null
            }
        ]
    });
    let (record, server) = run(
        |name, args| match name {
            "ops_get_cluster_state" if args.get("app") == Some(&json!("verify")) => {
                Err("verification exploded".to_owned())
            }
            "ops_get_cluster_state" => Ok(read_result()),
            "ops_scale_app" if args.get("replicas") == Some(&json!(6)) => Ok(scaled_result()),
            // The scale rollback (bound to the captured 3) fails loudly.
            "ops_scale_app" if args.get("replicas") == Some(&json!(3)) => {
                Err("the cluster refused the rollback rescale".to_owned())
            }
            // If the unwind kept going past the failed rollback, the
            // state rollback (99) would arrive; fail loud if it ever does.
            "ops_scale_app" => Err("state rollback dispatched after the unwind stopped".to_owned()),
            other => Err(format!("unexpected tool call {other}")),
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![
            StepStatus::NotUnwound,
            StepStatus::RollbackFailed,
            StepStatus::Failed
        ]
    );
    match &record.outcome {
        RunOutcome::Failed {
            step,
            error,
            rollback_failure,
            residual,
        } => {
            assert_eq!(step.as_str(), "verify");
            assert!(
                error.to_string().contains("verification exploded"),
                "the step failure names its cause: {error}"
            );
            let (rollback_step, rollback_error) = rollback_failure
                .as_ref()
                .expect("the rollback failure is reported alongside the step failure");
            assert_eq!(rollback_step.as_str(), "scale");
            assert!(
                rollback_error
                    .to_string()
                    .contains("refused the rollback rescale"),
                "the rollback failure names its own cause: {rollback_error}"
            );
            assert_eq!(
                residual.iter().map(StepId::as_str).collect::<Vec<_>>(),
                vec!["state"],
                "the residual set names the applied step whose rollback never ran"
            );
        }
        other => panic!("expected a Failed outcome, got {other:?}"),
    }
    assert!(matches!(
        record.steps[1].rollback,
        Some(RollbackOutcome::Failed { .. })
    ));
    assert_eq!(
        record.steps[0].rollback, None,
        "the unwind stopped before step one's rollback was attempted"
    );

    // Reverse completion order: the unwind's first (and only) attempt is
    // the most recently applied step's rollback. Nothing follows it.
    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
            ("ops_get_cluster_state".to_owned(), json!({"app": "verify"})),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 3})
            ),
        ]
    );
}

/// Card acceptance 4: a `$from` binding whose captured value violates
/// the declared bounds is a step failure at resolve time — the failing
/// step's tool call is never dispatched (the call log proves it) — and
/// the prior applied step unwinds.
#[tokio::test]
async fn bounds_rejection_unwinds_prior_steps() {
    let cancel = CancellationToken::new();
    let workflow = json!({
        "goal": "scale payments inside a tight envelope",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments",
                         "replicas": {"$from": "state.current_replicas", "min": 5, "max": 20}},
                "exports": {},
                "rollback": bound_scale_rollback()
            }
        ]
    });
    let (record, server) = run(
        |name, args| match name {
            "ops_get_cluster_state" => Ok(read_result()),
            // Only the unwind's 99 may arrive; the rejected apply must not.
            "ops_scale_app" if args.get("replicas") == Some(&json!(99)) => Ok(scaled_result()),
            other => Err(format!("unexpected tool call {other}: {args:?}")),
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![StepStatus::Unwound, StepStatus::Failed]
    );
    match &record.outcome {
        RunOutcome::Failed {
            step,
            error,
            rollback_failure,
            residual,
        } => {
            assert_eq!(step.as_str(), "scale");
            assert!(
                matches!(
                    error,
                    ExecuteError::Resolve(ResolveError::BoundsViolation { .. })
                ),
                "the bounds rejection is a resolve-time failure, not a tool failure: {error:?}"
            );
            assert!(rollback_failure.is_none());
            assert!(residual.is_empty());
        }
        other => panic!("expected a Failed outcome, got {other:?}"),
    }

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 99})
            ),
        ],
        "the rejected step is never dispatched; only the prior step's rollback runs"
    );
}

/// Card acceptance 5: a declared export path the step's own result does
/// not carry is a step failure — the tool call went out and returned,
/// but the capture missed — and the prior applied step unwinds.
#[tokio::test]
async fn binding_miss_missing_path_is_a_step_failure() {
    let cancel = CancellationToken::new();
    let workflow = json!({
        "goal": "scale payments and capture the applied count",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {"applied": "$.replicas"},
                "rollback": bound_scale_rollback()
            }
        ]
    });
    let (record, server) = run(
        |name, _args| match name {
            "ops_get_cluster_state" => Ok(read_result()),
            // The scale result deliberately carries no `replicas` key:
            // the declared export path misses.
            "ops_scale_app" => Ok(json!({"status": "scaled"})),
            other => Err(format!("unexpected tool call {other}")),
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![StepStatus::Unwound, StepStatus::Failed]
    );
    match &record.outcome {
        RunOutcome::Failed {
            step,
            error,
            rollback_failure,
            residual,
        } => {
            assert_eq!(step.as_str(), "scale");
            assert!(
                matches!(
                    error,
                    ExecuteError::Resolve(ResolveError::ExportPathMissing { .. })
                ),
                "the binding miss is a missing-export-path failure: {error:?}"
            );
            assert!(rollback_failure.is_none());
            assert!(residual.is_empty());
        }
        other => panic!("expected a Failed outcome, got {other:?}"),
    }

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
            // The step call went out before the capture missed; then the unwind.
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 99})
            ),
        ]
    );
}

/// Card acceptance 6 (D11): cancelling mid-apply — the scripted server
/// cancels the token while the second step's call is in flight — halts
/// dispatch. No rollback runs, the outcome is `Cancelled` enumerating
/// the applied residual, and the unstarted step stays `NotStarted`.
#[tokio::test]
async fn cancellation_mid_apply_halts_and_records_residual_state() {
    let cancel = CancellationToken::new();
    let script_token = cancel.clone();
    let workflow = json!({
        "goal": "scale payments and verify",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            },
            {
                "id": "verify",
                "dependencies": ["scale"],
                "tool": "ops_get_cluster_state",
                "args": {"app": "verify"},
                "exports": {},
                "rollback": null
            }
        ]
    });
    let (record, server) = run(
        move |name, _args| {
            if name == "ops_scale_app" {
                // Deterministic cancel point: strictly after step one
                // completed and strictly before this call's response
                // reaches the executor, so the check before the next
                // dispatch cannot miss it.
                script_token.cancel();
            }
            match name {
                "ops_get_cluster_state" => Ok(read_result()),
                "ops_scale_app" => Ok(scaled_result()),
                other => Err(format!("unexpected tool call {other}")),
            }
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![
            StepStatus::Applied,
            StepStatus::Applied,
            StepStatus::NotStarted
        ]
    );
    match &record.outcome {
        RunOutcome::Cancelled { residual } => {
            assert_eq!(
                residual.iter().map(StepId::as_str).collect::<Vec<_>>(),
                vec!["state", "scale"],
                "the residual enumerates the steps still applied"
            );
        }
        other => panic!("expected a Cancelled outcome, got {other:?}"),
    }

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
        ],
        "apply halts after the in-flight step: no verify dispatch, no rollback"
    );
}

/// D11 edge: a cancellation
/// that lands while a FAILING call is in flight still halts — the
/// failed step is recorded `Failed`, but no rollback dispatches after
/// the interrupt, and the outcome is `Cancelled` with the applied
/// steps as residual.
#[tokio::test]
async fn cancellation_during_a_failing_call_halts_without_unwinding() {
    let cancel = CancellationToken::new();
    let script_token = cancel.clone();
    let workflow = json!({
        "goal": "scale payments",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            }
        ]
    });
    let (record, server) = run(
        move |name, _args| {
            if name == "ops_scale_app" {
                // Cancel lands inside the failing call: the executor
                // observes the failure with the token already cancelled.
                script_token.cancel();
                return Err("the cluster rejected the scale".to_owned());
            }
            Ok(read_result())
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![StepStatus::Applied, StepStatus::Failed],
        "the failed step is recorded truthfully; the applied step is never unwound"
    );
    match &record.outcome {
        RunOutcome::Cancelled { residual } => {
            assert_eq!(
                residual.iter().map(StepId::as_str).collect::<Vec<_>>(),
                vec!["state"],
                "the interrupt, not the failure, ends the run"
            );
        }
        other => panic!("expected a Cancelled outcome, got {other:?}"),
    }

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
        ],
        "no rollback dispatches after the operator interrupted the apply"
    );
}

/// D11 edge, final-call case: a
/// cancellation that lands during the FINAL in-flight call records the
/// interrupt — every step applied, none unwound, `Cancelled` with the
/// full applied set as residual — rather than a clean `Complete`.
#[tokio::test]
async fn cancellation_during_the_final_call_records_the_interrupt() {
    let cancel = CancellationToken::new();
    let script_token = cancel.clone();
    let workflow = json!({
        "goal": "scale payments",
        "steps": [
            state_step(state_rollback()),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            }
        ]
    });
    let (record, server) = run(
        move |name, _args| {
            if name == "ops_scale_app" {
                // Cancel lands inside the final call, which succeeds.
                script_token.cancel();
                return Ok(scaled_result());
            }
            Ok(read_result())
        },
        workflow,
        &cancel,
    )
    .await;

    assert_eq!(
        statuses(&record),
        vec![StepStatus::Applied, StepStatus::Applied]
    );
    match &record.outcome {
        RunOutcome::Cancelled { residual } => {
            assert_eq!(
                residual.iter().map(StepId::as_str).collect::<Vec<_>>(),
                vec!["state", "scale"],
                "the interrupt is recorded, not swallowed into a Complete"
            );
        }
        other => panic!("expected a Cancelled outcome, got {other:?}"),
    }

    let log = server.calls().await;
    assert_eq!(
        log,
        vec![
            (
                "ops_get_cluster_state".to_owned(),
                json!({"app": "payments"})
            ),
            (
                "ops_scale_app".to_owned(),
                json!({"app": "payments", "replicas": 6})
            ),
        ],
        "nothing follows the interrupted apply: no rollback on cancel"
    );
}

// ============================================================================
// Approval wire rig
// ============================================================================

mod approval_rig {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::{
        Json, Router,
        extract::{Path, State},
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use serde::Deserialize;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// The notify body as the receiver sees it: the wire shape the client
    /// serializes, keyed by its own `decision_id` field exactly the way the
    /// governance mirror reads it.
    #[derive(Deserialize)]
    struct NotifyBody {
        #[allow(dead_code)]
        workflow: serde_json::Value,
        #[allow(dead_code)]
        digest: String,
        decision_id: String,
        #[allow(dead_code)]
        rendered: String,
        #[allow(dead_code)]
        session_id: String,
    }

    #[derive(Clone)]
    pub struct ServerState {
        decisions: Arc<Mutex<HashMap<String, DecisionRow>>>,
        pending_status: StatusCode,
    }

    impl Default for ServerState {
        fn default() -> Self {
            Self {
                decisions: Arc::new(Mutex::new(HashMap::new())),
                pending_status: StatusCode::ACCEPTED,
            }
        }
    }

    struct DecisionRow {
        #[allow(dead_code)]
        body: NotifyBody,
        status: DecisionStatus,
    }

    #[derive(Clone)]
    enum DecisionStatus {
        Pending,
        Approved,
        Denied(Option<String>),
    }

    pub struct ApprovalServerHandle {
        base_url: reqwest::Url,
        state: ServerState,
    }

    impl ApprovalServerHandle {
        pub fn notify_url(&self) -> reqwest::Url {
            self.base_url.join("/decisions").unwrap()
        }

        /// The poll base: ends in `/` so the client joins
        /// `<status_url><decision_id>/status` onto it without dropping the
        /// last segment. Deliberately a different convention from
        /// `notify_url` so the two URL derivations are both exercised.
        pub fn status_url(&self) -> reqwest::Url {
            self.base_url.join("/decisions/").unwrap()
        }

        pub async fn approve(&self, decision_id: &str) {
            let mut map = self.state.decisions.lock().await;
            if let Some(row) = map.get_mut(decision_id) {
                row.status = DecisionStatus::Approved;
            }
        }

        pub async fn deny(&self, decision_id: &str, reason: &str) {
            let mut map = self.state.decisions.lock().await;
            if let Some(row) = map.get_mut(decision_id) {
                row.status = DecisionStatus::Denied(Some(reason.to_owned()));
            }
        }

        pub async fn deny_without_reason(&self, decision_id: &str) {
            let mut map = self.state.decisions.lock().await;
            if let Some(row) = map.get_mut(decision_id) {
                row.status = DecisionStatus::Denied(None);
            }
        }

        /// Decide `approved` once the notify POST has landed its row.
        ///
        /// For legs that drive the tool (whose execute call blocks inside
        /// the hold): the decision must arrive while the hold is pending,
        /// so this waits for the row instead of racing the POST.
        pub async fn approve_when_present(&self, decision_id: &str) {
            for _ in 0..500 {
                {
                    let mut map = self.state.decisions.lock().await;
                    if let Some(row) = map.get_mut(decision_id) {
                        row.status = DecisionStatus::Approved;
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("approval row {decision_id} never landed");
        }
    }

    pub async fn boot() -> ApprovalServerHandle {
        boot_with_pending_status(StatusCode::ACCEPTED).await
    }

    /// Boot with a chosen pending status code, so both halves of the
    /// pending contract (202 and 207) are exercised across the legs.
    pub async fn boot_with_pending_status(pending: StatusCode) -> ApprovalServerHandle {
        let state = ServerState {
            pending_status: pending,
            ..ServerState::default()
        };
        let app = Router::new()
            .route("/decisions", post(notify))
            .route("/decisions/{id}/status", get(status))
            .route("/admin/decisions/{id}/approve", post(admin_approve))
            .route("/admin/decisions/{id}/deny", post(admin_deny))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::task::yield_now().await;
        ApprovalServerHandle {
            base_url: reqwest::Url::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            state,
        }
    }

    async fn notify(
        State(state): State<ServerState>,
        Json(body): Json<NotifyBody>,
    ) -> impl IntoResponse {
        // Insert-if-absent: a re-POST addresses the same row and never
        // reopens a decided one - the first decision on an id is final,
        // matching the governance mirror's contract.
        state
            .decisions
            .lock()
            .await
            .entry(body.decision_id.clone())
            .or_insert(DecisionRow {
                body,
                status: DecisionStatus::Pending,
            });
        StatusCode::ACCEPTED
    }

    async fn status(State(state): State<ServerState>, Path(id): Path<String>) -> impl IntoResponse {
        let map = state.decisions.lock().await;
        let (status, body) = match map.get(&id) {
            Some(row) => match &row.status {
                DecisionStatus::Pending => (state.pending_status, json!({"pending": true})),
                DecisionStatus::Approved => {
                    (StatusCode::OK, json!({"approved": true, "reason": null}))
                }
                DecisionStatus::Denied(reason) => {
                    (StatusCode::OK, json!({"approved": false, "reason": reason}))
                }
            },
            None => (StatusCode::NOT_FOUND, json!({"error": "not found"})),
        };
        (status, Json(body))
    }

    async fn admin_approve(State(state): State<ServerState>, Path(id): Path<String>) {
        let mut map = state.decisions.lock().await;
        if let Some(row) = map.get_mut(&id) {
            row.status = DecisionStatus::Approved;
        }
    }

    #[derive(Deserialize, Default)]
    struct DenyBody {
        reason: Option<String>,
    }

    async fn admin_deny(
        State(state): State<ServerState>,
        Path(id): Path<String>,
        Json(body): Json<DenyBody>,
    ) {
        let mut map = state.decisions.lock().await;
        if let Some(row) = map.get_mut(&id) {
            row.status = DecisionStatus::Denied(body.reason);
        }
    }
}

// ============================================================================
// Approval wire helpers
// ============================================================================

fn approval_payload(workflow: JsonValue) -> ApprovalPayload {
    let workflow: agent_driver_prototype::workflow::WorkflowSpec =
        serde_json::from_value(workflow).expect("workflow parses");
    ApprovalPayload::new(
        workflow,
        "rendered digest".to_owned(),
        "test-session".to_owned(),
        &DecisionId::Digest,
    )
}

// ============================================================================
// W4 acceptance scenarios
// ============================================================================

/// W4 offline: an approved decision resolves the hold to
/// [`ApprovalOutcome::Approved`]. The row is decided after the notify POST
/// lands and while the hold is pending.
#[tokio::test]
async fn approve_leg_decides_approved() {
    let server = approval_rig::boot().await;
    let client = ApprovalClient::new(
        server.notify_url(),
        server.status_url(),
        60,
        DecisionId::Digest,
    );
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));
    let decision_id = payload.decision_id().to_owned();

    let hold = client.notify(payload).await.expect("notify returns a hold");
    server.approve(&decision_id).await;
    let outcome = hold
        .outcome(&CancellationToken::new())
        .await
        .expect("outcome resolves");

    assert_eq!(outcome, ApprovalOutcome::Approved);
}

/// W4 offline: a denied decision resolves the hold to
/// [`ApprovalOutcome::Denied`] and carries the receiver's reason.
#[tokio::test]
async fn deny_leg_decides_denied_with_reason() {
    let server = approval_rig::boot().await;
    let client = ApprovalClient::new(
        server.notify_url(),
        server.status_url(),
        60,
        DecisionId::Digest,
    );
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));
    let decision_id = payload.decision_id().to_owned();

    let hold = client.notify(payload).await.expect("notify returns a hold");
    server.deny(&decision_id, "not today").await;
    let outcome = hold
        .outcome(&CancellationToken::new())
        .await
        .expect("outcome resolves");

    assert_eq!(
        outcome,
        ApprovalOutcome::Denied {
            reason: Some("not today".to_owned())
        }
    );
}

/// W4 offline: a denial without a reason resolves to
/// [`ApprovalOutcome::Denied`] with `reason: None` - the type keeps the
/// "receiver gave no reason" case distinct from an empty one.
#[tokio::test]
async fn deny_leg_without_reason_decides_denied() {
    let server = approval_rig::boot().await;
    let client = ApprovalClient::new(
        server.notify_url(),
        server.status_url(),
        60,
        DecisionId::Digest,
    );
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));
    let decision_id = payload.decision_id().to_owned();

    let hold = client.notify(payload).await.expect("notify returns a hold");
    server.deny_without_reason(&decision_id).await;
    let outcome = hold
        .outcome(&CancellationToken::new())
        .await
        .expect("outcome resolves");

    assert_eq!(outcome, ApprovalOutcome::Denied { reason: None });
}

/// W4 offline: if no decision arrives before the hold budget expires, the
/// hold resolves to [`ApprovalOutcome::TimedOut`]. This leg boots the
/// receiver with 207-pending so both halves of the pending contract are
/// exercised (the other legs run 202).
#[tokio::test]
async fn hold_timeout_leg_decides_timed_out() {
    let server = approval_rig::boot_with_pending_status(axum::http::StatusCode::MULTI_STATUS).await;
    let client = ApprovalClient::new(
        server.notify_url(),
        server.status_url(),
        1,
        DecisionId::Digest,
    );
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));

    let hold = client.notify(payload).await.expect("notify returns a hold");
    let outcome = hold
        .outcome(&CancellationToken::new())
        .await
        .expect("outcome resolves");

    assert_eq!(outcome, ApprovalOutcome::TimedOut);
}

/// W4 offline: if the request cancellation token fires while the hold is
/// pending, the hold resolves to [`ApprovalOutcome::Cancelled`] and never
/// hangs. The token fires mid-flight (after notify, during the pending
/// hold), not before it.
#[tokio::test]
async fn cancel_hold_leg_decides_cancelled() {
    let server = approval_rig::boot().await;
    let client = ApprovalClient::new(
        server.notify_url(),
        server.status_url(),
        60,
        DecisionId::Digest,
    );
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));
    let cancel = CancellationToken::new();

    let hold = client.notify(payload).await.expect("notify returns a hold");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    cancel.cancel();
    let outcome = hold.outcome(&cancel).await.expect("outcome resolves");

    assert_eq!(outcome, ApprovalOutcome::Cancelled);
}

/// W4 offline: the payload's digest binding verifies - the sha256 over the
/// workflow bytes is the digest the approver authorizes.
#[tokio::test]
async fn payload_binding_verifies() {
    let payload = approval_payload(json!({ "goal": "g", "steps": [] }));

    assert!(payload.verify_binding());
    assert_eq!(payload.decision_id(), payload.digest());
}

/// W4 end-to-end: a proposed workflow is validated, the approval hold is
/// approved while pending, and the W3 executor applies the steps.
#[tokio::test]
async fn end_to_end_approval_gates_apply() {
    let server = rig::ScriptedServer::new(advertised(), |name, _args| match name {
        "ops_get_cluster_state" => Ok(read_result()),
        "ops_scale_app" => Ok(scaled_result()),
        other => Err(format!("unexpected tool call {other}")),
    });
    let (sidecar, _served) = rig::boot(server).await;

    let approval_server = approval_rig::boot().await;
    let approval_client = ApprovalClient::new(
        approval_server.notify_url(),
        approval_server.status_url(),
        60,
        DecisionId::Digest,
    );
    let tool = ProposeWorkflowTool::new(sidecar).with_approval(approval_client);

    let workflow = json!({
        "goal": "scale payments",
        "steps": [
            state_step(json!(null)),
            {
                "id": "scale",
                "dependencies": ["state"],
                "tool": "ops_scale_app",
                "args": {"app": "payments", "replicas": 6},
                "exports": {},
                "rollback": bound_scale_rollback()
            }
        ]
    });
    let input = ToolInput::from_value(json!({ "workflow": workflow })).unwrap();

    // The payload's decision id is deterministic under the Digest policy,
    // so the approver can pre-compute the row it will approve.
    let parsed: agent_driver_prototype::workflow::WorkflowSpec =
        serde_json::from_value(workflow).expect("workflow parses");
    let probe = ApprovalPayload::new(parsed, String::new(), "e2e".to_owned(), &DecisionId::Digest);
    let decision_id = probe.decision_id().to_owned();

    // execute blocks inside the approval hold; the decision must land
    // while it is pending.
    let executing = tokio::spawn(async move {
        tool.execute(&input, &ToolContext::new(CancellationToken::new()))
            .await
            .expect("propose_workflow returns every outcome as an observation")
    });
    approval_server.approve_when_present(&decision_id).await;
    let result = executing.await.expect("execute joins");

    assert!(result.is_success(), "{result:?}");
    assert!(
        result.content().contains("Complete") || result.content().contains("Applied"),
        "expected a successful run record, got: {}",
        result.content()
    );
}
