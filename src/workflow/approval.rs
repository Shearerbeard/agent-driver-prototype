//! Sync approval wire: the blocking-hold client that mirrors aura's HITL
//! park/reify type structure while keeping the transport synchronous-polling.
//!
//! The serializable core is [`ApprovalPayload`]: everything the receiver
//! needs to re-render and re-validate the proposed instance, splittable from
//! the runtime-only poll handle.  The runtime half is [`ApprovalClient`] +
//! [`ApprovalHold`]: the client POSTs the payload and polls the receiver;
//! the hold is a consumable typestate that selects on poll completion,
//! wall-clock deadline, and request cancellation.
//!
//! Wire semantics pinned at this seam:
//!
//! - Re-POST idempotency follows the governance mirror's contract: a
//!   re-POST of the same decision id updates a still-pending row and never
//!   reopens a decided one - the first decision on an id is final, so a
//!   transport retry cannot clobber a decision that already landed.
//! - The status URL is a base ending in `/`; the poll leg joins
//!   `<status_url><decision_id>/status` onto it.
//! - A client clone may notify again (it is a handle); each notify mints a
//!   fresh [`ApprovalHold`], and a hold is awaited at most once because
//!   [`ApprovalHold::outcome`] consumes it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::mcp_client::SidecarClient;
use crate::workflow::executor::{ExecuteError, RunRecord};
use crate::workflow::plan::{ValidatedWorkflowSpec, WorkflowSpec};

/// Seconds between status polls while the hold is pending.
///
/// The sync contract is polling, not streaming; this pacing keeps network
/// traffic quiet while still resolving a decision within a small multiple of
/// the interval.
pub const POLL_INTERVAL_SECONDS: u64 = 2;

/// Per-request HTTP timeout for notify and poll calls.
///
/// Bounds a stalled transport independently of the hold budget, so a hung
/// POST surfaces as an [`ApprovalError::Transport`] observation rather than
/// an unbounded wait.
pub const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Counter making generated decision ids unique within a process even when
/// the wall clock has not ticked.
static GENERATED_SEQ: AtomicU64 = AtomicU64::new(0);

/// The terminal result of awaiting an approval hold.
///
/// Fail-closed is structural: only the [`Approved`] witness carried by
/// [`ApprovalOutcome::Approved`] can drive an apply through
/// [`apply_authorized`].  Every other variant is an ordinary tool
/// observation the coordinator replans against.
#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalOutcome {
    /// The human approved the exact proposed instance; the executor may apply.
    Approved,
    /// The human denied the proposal.  `reason` is the receiver's explanation
    /// when it gave one.
    Denied {
        /// The receiver's explanation for the denial, if any.
        reason: Option<String>,
    },
    /// The hold budget expired before a decision arrived.
    TimedOut,
    /// The request's cancellation token fired before a decision arrived.
    Cancelled,
}

impl ApprovalOutcome {
    /// Consume the outcome into the apply authorization it carries.
    ///
    /// This is the only constructor of [`Approved`] reachable from an
    /// outcome, which is what makes fail-closed structural at this seam.
    pub fn into_approved(self) -> Option<Approved> {
        match self {
            Self::Approved => Some(Approved { _private: () }),
            _ => None,
        }
    }
}

/// The apply authorization only [`ApprovalOutcome::Approved`] can produce.
///
/// [`apply_authorized`] requires this witness, so the approval-gated apply
/// path cannot be reached without a terminal approval.  (W3's
/// `execute_workflow` remains public for its own test contract; production
/// apply goes through this witness.  The residual is recorded on the card
/// for the U(wire-contract) gate.)
#[derive(Clone, Copy, Debug)]
pub struct Approved {
    _private: (),
}

/// Apply a workflow whose approval produced the witness.
///
/// Delegates to the W3 executor; the witness parameter is the type-level
/// record that an approval terminal state was reached first.
pub async fn apply_authorized(
    _witness: Approved,
    spec: &ValidatedWorkflowSpec,
    sidecar: &SidecarClient,
    cancel: &CancellationToken,
) -> Result<RunRecord, ExecuteError> {
    crate::workflow::executor::execute_workflow(spec, sidecar, cancel).await
}

/// The serializable notify-payload core: everything the receiver needs to
/// render the proposal and the approver needs to bind the decision to the
/// instance.
///
/// The digest and decision id are computed by the constructor over the
/// workflow bytes and the configured identifier policy; they cannot be set
/// to disagree with the payload by construction (there is no public field
/// access, and no `Deserialize`).  The type is intentionally independent of
/// runtime handles (`reqwest` client, cancellation token) so it can be
/// logged, replayed, or persisted without capturing process-local state.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ApprovalPayload {
    /// The full workflow spec being proposed for authorization.
    workflow: WorkflowSpec,
    /// sha256 over `serde_json::to_vec(&workflow)`, hex-encoded.
    ///
    /// The digest binds the approval to the exact proposed instance.
    digest: String,
    /// The identifier the receiver keys its approval row by, realized from
    /// the [`DecisionId`] policy at construction.
    ///
    /// Reading A (digest-as-id) leaves this equal to `digest`; Reading B
    /// mints a distinct per-proposal id.  The U(wire-contract) gate
    /// adjudicates which reading ships.
    decision_id: String,
    /// The human digest rendered at propose time.
    ///
    /// The receiver may echo this without re-rendering.
    rendered: String,
    /// The shim session id that created the proposal, for correlation.
    session_id: String,
}

impl ApprovalPayload {
    /// Build the payload from the validated spec, rendered digest, session
    /// id, and the decision-identifier policy.
    ///
    /// The digest is the binding: the approver authorizes the exact bytes
    /// that produced it.  The decision id is realized here, once per
    /// proposal, so a generated policy cannot collapse two proposals onto
    /// one row.
    pub fn new(
        workflow: WorkflowSpec,
        rendered: String,
        session_id: String,
        policy: &DecisionId,
    ) -> Self {
        let digest = compute_digest(&workflow);
        let decision_id = policy.assign(&digest);
        Self {
            workflow,
            digest,
            decision_id,
            rendered,
            session_id,
        }
    }

    /// The workflow spec being proposed.
    pub fn workflow(&self) -> &WorkflowSpec {
        &self.workflow
    }

    /// The instance digest binding this approval.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The decision id the receiver keys this approval's row by.
    pub fn decision_id(&self) -> &str {
        &self.decision_id
    }

    /// The rendered human digest.
    pub fn rendered(&self) -> &str {
        &self.rendered
    }

    /// The proposing shim session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Recompute the digest over the workflow bytes and compare.
    ///
    /// The receiver (or a skeptical log reader) proves the binding with
    /// this; the constructor already guarantees it in-process.
    pub fn verify_binding(&self) -> bool {
        compute_digest(&self.workflow) == self.digest
    }
}

/// Compute the instance digest: sha256 over the serde_json bytes of the
/// workflow spec.
fn compute_digest(workflow: &WorkflowSpec) -> String {
    let _ = workflow;
    todo!()
}

/// The decision-identifier seam.
///
/// The governance mirror keys approval rows by a `decision_id` read from the
/// POST body, while the minted payload carries the instance digest.  This
/// enum is the single policy from which the payload's decision id is
/// realized, once per proposal:
///
/// - Reading A (digest-as-id): the sha256 digest doubles as the decision id,
///   so re-posting the same proposal addresses the same row.
/// - Reading B (distinct field): a fresh id is minted per proposal, keeping
///   binding (digest) and routing (decision id) separate.
///
/// The user gate `U(wire-contract)` adjudicates which reading wins; the
/// production choice is made at exactly one call site (`workflow_tool_for`).
#[derive(Clone, Debug, Default)]
pub enum DecisionId {
    /// The digest is the decision id.
    #[default]
    Digest,
    /// A distinct id is minted per proposal.
    Generated,
}

impl DecisionId {
    /// Realize the wire identifier for one proposal.
    ///
    /// Called once per [`ApprovalPayload::new`], never per client, so a
    /// generated policy distinguishes every proposal.
    pub fn assign(&self, digest: &str) -> String {
        match self {
            Self::Digest => digest.to_owned(),
            Self::Generated => {
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default();
                let seq = GENERATED_SEQ.fetch_add(1, Ordering::Relaxed);
                format!("wf-{nanos:x}-{seq:x}")
            }
        }
    }
}

/// A consumable typestate representing a pending approval hold.
///
/// Constructed by [`ApprovalClient::notify`].  The only way to obtain an
/// [`ApprovalOutcome`] is to consume `self` through [`ApprovalHold::outcome`],
/// so a hold cannot be awaited twice.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ApprovalHold {
    client: ApprovalClient,
    payload: ApprovalPayload,
    deadline: SystemTime,
}

impl ApprovalHold {
    /// Wait for the approval decision, consuming the hold.
    ///
    /// Selects on:
    /// - a successful status poll that returns `200 { approved, reason }`;
    /// - the wall-clock deadline for the configured `hold_secs` budget;
    /// - the request cancellation token.
    ///
    /// Any wire or HTTP failure is returned as an [`ApprovalError`]
    /// observation; it is never converted silently to a denial.
    pub async fn outcome(
        self,
        cancel: &CancellationToken,
    ) -> Result<ApprovalOutcome, ApprovalError> {
        let _ = (self, cancel);
        todo!()
    }

    /// The wall-clock deadline against which the hold is bounded.
    pub fn deadline(&self) -> SystemTime {
        self.deadline
    }
}

/// The runtime client for the approval receiver.
///
/// Owns the decision-identifier policy so it is chosen at exactly one
/// construction site ([`ApprovalClient::for_section`] via
/// `workflow_tool_for`); the payload realizes it per proposal.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ApprovalClient {
    http: reqwest::Client,
    notify_url: reqwest::Url,
    status_url: reqwest::Url,
    hold_secs: u64,
    decision_id: DecisionId,
}

/// Why the approval wire failed.
///
/// A wire failure is returned as a tool-error observation; it is not a
/// denial, so the coordinator can replan against it.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ApprovalError {
    /// The HTTP client could not deliver the request or read the response.
    #[error("approval wire transport failed: {0}")]
    Transport(String),
    /// The receiver returned a response the contract does not define.
    #[error("approval receiver returned unexpected status {status}: {body}")]
    UnexpectedStatus {
        /// The HTTP status code.
        status: u16,
        /// The response body, for diagnostics.
        body: String,
    },
    /// Building the approval URL failed.
    #[error("approval URL is invalid: {0}")]
    InvalidUrl(String),
}

impl ApprovalClient {
    /// Build a client from explicit notify/status URLs, a hold budget, and
    /// the decision-identifier policy.
    ///
    /// `status_url` must end in `/`; the poll leg joins
    /// `<status_url><decision_id>/status`.  Tests and the Layer-2 fill use
    /// this constructor; production call sites use [`Self::for_section`],
    /// which is the decision-identifier seam's single decision point.
    pub fn new(
        notify_url: reqwest::Url,
        status_url: reqwest::Url,
        hold_secs: u64,
        decision_id: DecisionId,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .expect("a timeout-only client builder is always valid"),
            notify_url,
            status_url,
            hold_secs,
            decision_id,
        }
    }

    /// The decision-identifier policy this client realizes per proposal.
    pub fn decision_id_policy(&self) -> &DecisionId {
        &self.decision_id
    }

    /// Create a client from the configured approval URL and hold budget.
    ///
    /// This is the decision-identifier seam: the notify and status URLs are
    /// derived from the base URL together with the chosen identifier
    /// policy.
    pub fn for_section(
        approval_url: &str,
        hold_secs: u64,
        decision_id: DecisionId,
    ) -> Result<Self, ApprovalError> {
        let _ = (approval_url, hold_secs, decision_id);
        todo!()
    }

    /// Post the notify payload and return a hold that can be awaited.
    ///
    /// The decision identifier applied to the wire comes from the policy
    /// this client was constructed with, realized into the payload at its
    /// construction.  A re-POST of an already-decided id returns the
    /// existing decision; it never reopens the row.
    pub async fn notify(&self, payload: ApprovalPayload) -> Result<ApprovalHold, ApprovalError> {
        let _ = payload;
        todo!()
    }

    /// Poll the status endpoint once.
    ///
    /// Returns the decoded decision for `200`, [`None`] for `202`/`207`
    /// pending, and an error for any other status.
    #[expect(dead_code)]
    async fn poll(&self, decision_id: &str) -> Result<Option<DecisionResponse>, ApprovalError> {
        let _ = decision_id;
        todo!()
    }

    /// The configured wall-clock deadline from now.
    #[expect(dead_code)]
    fn deadline(&self) -> SystemTime {
        SystemTime::now() + Duration::from_secs(self.hold_secs)
    }
}

/// The receiver's status response body.
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct DecisionResponse {
    approved: bool,
    #[serde(default)]
    reason: Option<String>,
}
