//! Sync approval wire: the blocking-hold client that mirrors aura's HITL
//! park/reify type structure while keeping the transport synchronous-polling.
//!
//! The serializable core is [`ApprovalPayload`]: everything the receiver needs
//! to re-render and re-validate the proposed instance, splittable from the
//! runtime-only poll handle.  The runtime half is [`ApprovalClient`] +
//! [`ApprovalHold`]: the client POSTs the payload and polls the receiver;
//! the hold is a consumable typestate that selects on poll completion,
//! wall-clock deadline, and request cancellation.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::workflow::plan::WorkflowSpec;

/// Seconds between status polls while the hold is pending.
///
/// The sync contract is polling, not streaming; this pacing keeps network
/// traffic quiet while still resolving a decision within a small multiple of
/// the interval.
pub const POLL_INTERVAL_SECONDS: u64 = 2;

/// The terminal result of awaiting an approval hold.
///
/// Fail-closed is structural: only [`ApprovalOutcome::Approved`] leads to
/// `execute_workflow`.  Every other variant is an ordinary tool observation
/// the coordinator replans against.
#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalOutcome {
    /// The human approved the exact proposed instance; the executor may apply.
    Approved,
    /// The human denied the proposal.  The `reason` comes from the wire.
    Denied {
        /// The receiver's explanation for the denial.
        reason: String,
    },
    /// The hold budget expired before a decision arrived.
    TimedOut,
    /// The request's cancellation token fired before a decision arrived.
    Cancelled,
}

/// The serializable notify-payload core: everything the receiver needs to
/// render the proposal and the approver needs to bind the decision to the
/// instance.
///
/// This type is intentionally independent of runtime handles (`reqwest`
/// client, cancellation token) so it can be logged, replayed, or persisted
/// without capturing process-local state.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ApprovalPayload {
    /// The full workflow spec being proposed for authorization.
    pub workflow: WorkflowSpec,
    /// sha256 over `serde_json::to_vec(&workflow)`, hex-encoded.
    ///
    /// The digest binds the approval to the exact proposed instance.
    pub digest: String,
    /// The human digest rendered at propose time.
    ///
    /// The receiver may echo this without re-rendering.
    pub rendered: String,
    /// The shim session id that created the proposal, for correlation.
    pub session_id: String,
}

impl ApprovalPayload {
    /// Build the payload from the validated spec, rendered digest, and
    /// session id.
    ///
    /// The digest is the binding: the approver authorizes the exact bytes
    /// that produced it.
    pub fn new(workflow: WorkflowSpec, rendered: String, session_id: String) -> Self {
        let digest = compute_digest(&workflow);
        Self {
            workflow,
            digest,
            rendered,
            session_id,
        }
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
/// POST body, while the minted payload carries only the instance digest.
/// This enum is the single place the two readings meet:
///
/// - Reading A (digest-as-id): the sha256 digest doubles as the decision id,
///   so re-posting the same proposal updates the same row.
/// - Reading B (distinct field): a generated decision id rides alongside the
///   digest, keeping binding and routing separate.
///
/// The user gate `U(wire-contract)` adjudicates which reading wins.
#[derive(Clone, Debug)]
pub enum DecisionId {
    /// The digest is the decision id.
    Digest,
    /// A distinct generated id.
    Generated(String),
}

impl DecisionId {
    /// Resolve this seam to the identifier sent on the wire.
    pub fn for_payload(&self, payload: &ApprovalPayload) -> String {
        match self {
            Self::Digest => payload.digest.clone(),
            Self::Generated(id) => id.clone(),
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
    decision_id: String,
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
/// `workflow_tool_for`); `notify` applies it rather than re-deriving it.
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
    /// Tests and the Layer-2 fill use this constructor; production call
    /// sites use [`Self::for_section`], which is the decision-identifier
    /// seam's single decision point.
    pub fn new(
        notify_url: reqwest::Url,
        status_url: reqwest::Url,
        hold_secs: u64,
        decision_id: DecisionId,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            notify_url,
            status_url,
            hold_secs,
            decision_id,
        }
    }

    /// Create a client from the configured approval URL and hold budget.
    ///
    /// This is the decision-identifier seam: the status URL is derived from
    /// the base URL together with the chosen identifier policy.
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
    /// The decision identifier applied to the wire comes from the
    /// policy this client was constructed with.
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
