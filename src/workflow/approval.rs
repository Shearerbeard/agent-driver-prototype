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
//! - The decision id is minted fresh per proposal as a UUID v7 string
//!   (Mike's ruling at the U(wire-contract) gate, 2026-10-06: a decision
//!   needs a unique id in the standard-aura shape, and the digest is not
//!   reused for it). The digest remains the binding of the approval to
//!   the exact proposed bytes; the two identifiers carry distinct roles -
//!   routing and binding.
//! - Re-POST idempotency follows the governance mirror's contract: a
//!   re-POST of the same decision id updates a still-pending row and never
//!   reopens a decided one - the first decision on an id is final, so a
//!   transport retry cannot clobber a decision that already landed.
//! - The status URL is a base ending in `/`; the poll leg joins
//!   `<status_url><decision_id>/status` onto it.
//! - A client clone may notify again (it is a handle); each notify mints a
//!   fresh [`ApprovalHold`] with a fresh decision id, and a hold is
//!   awaited at most once because [`ApprovalHold::outcome`] consumes it.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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

/// Lowercase hex alphabet for [`compute_digest`]'s encoding of the sha256
/// output.
const HEX_DIGITS: [u8; 16] = *b"0123456789abcdef";

/// Mint the identifier for one approval decision: a UUID v7 string.
///
/// Mike's U(wire-contract) ruling (2026-10-06): the decision id is a
/// unique id per decision in the standard-aura shape; the digest is not
/// reused. v7 matches aura's `DecisionId` (time-ordered, registry-key
/// friendly).
fn new_decision_id() -> String {
    Uuid::now_v7().to_string()
}

/// The terminal result of awaiting an approval hold.
///
/// Fail-closed is structural: the [`Approved`] witness rides inside the
/// `Approved` variant, constructible only inside this module (its field is
/// private), so no caller can forge an approval outcome - only
/// [`ApprovalHold::outcome`] minting one from a receiver's decision creates
/// it.  [`apply_authorized`] requires the witness, and every other variant
/// is an ordinary tool observation the coordinator replans against.
#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalOutcome {
    /// The human approved the exact proposed instance; the executor may apply.
    Approved(Approved),
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
    /// This extracts the witness a receiver's decision produced; it cannot
    /// forge one, which is what makes fail-closed structural at this seam.
    pub fn into_approved(self) -> Option<Approved> {
        match self {
            Self::Approved(approved) => Some(approved),
            _ => None,
        }
    }
}

/// The apply authorization only a receiver's approval decision can produce.
///
/// The field is private, so the type cannot be constructed outside this
/// module; [`apply_authorized`] requires it, so the approval-gated apply
/// path cannot be reached without a terminal approval.  (W3's
/// `execute_workflow` remains public for its own test contract; production
/// apply goes through this witness.  The residual is recorded on the card
/// for the U(wire-contract) gate.)
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// The identifier the receiver keys its approval row by: a fresh UUID
    /// v7 minted at construction.
    ///
    /// Distinct from `digest` by ruling: the digest binds the approval to
    /// the exact proposed bytes, while the decision id routes the row and
    /// the human's decision to it.
    decision_id: String,
    /// The human digest rendered at propose time.
    ///
    /// The receiver may echo this without re-rendering.
    rendered: String,
    /// The shim session id that created the proposal, for correlation.
    session_id: String,
}

impl ApprovalPayload {
    /// Build the payload from the validated spec, rendered digest, and
    /// session id, minting a fresh decision id.
    ///
    /// The digest is the binding: the approver authorizes the exact bytes
    /// that produced it.  The decision id is a fresh UUID v7 minted here,
    /// once per proposal, keeping routing (decision id) distinct from
    /// binding (digest) per the U(wire-contract) ruling.
    pub fn new(workflow: WorkflowSpec, rendered: String, session_id: String) -> Self {
        let digest = compute_digest(&workflow);
        let decision_id = new_decision_id();
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
    let bytes = serde_json::to_vec(workflow)
        .expect("WorkflowSpec is plain serde data: serialization cannot fail");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
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
        let ApprovalHold {
            client,
            payload,
            deadline,
        } = self;
        let decision_id = payload.decision_id().to_owned();
        // Transient-retry rule: a poll that fails with `Transport` is
        // retried at the next poll interval while budget remains - a single
        // transient hiccup during a long hold does not kill the hold.
        // `UnexpectedStatus` is terminal and returns immediately: the
        // client notified this decision id, so a receiver answering
        // off-contract is a violation polling cannot repair.
        let budget = deadline
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO);
        // The wall-clock deadline is a SystemTime; tokio's timer takes its
        // own Instant and the two do not interconvert (`Instant::from_std`
        // accepts a std Instant only), so the remaining budget is
        // re-anchored at the runtime's now.
        let deadline_at = tokio::time::Instant::now() + budget;
        let mut ticker = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECONDS));
        // A slow poll does not mint catch-up ticks: the next poll waits a
        // full interval.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            // Interval gate: the first tick fires immediately (starting
            // the first poll without a delay), later ticks pace one poll
            // per interval. Cancellation and the deadline stay selectable
            // while waiting for the tick.
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(ApprovalOutcome::Cancelled),
                () = tokio::time::sleep_until(deadline_at) => {
                    return Ok(ApprovalOutcome::TimedOut);
                }
                _ = ticker.tick() => {}
            }
            // Poll phase: the request is awaited under cancellation and
            // the deadline, so an in-flight poll can neither outlive the
            // hold budget (bounded further by the client's request
            // timeout) nor delay cancellation.
            let polled = tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(ApprovalOutcome::Cancelled),
                () = tokio::time::sleep_until(deadline_at) => {
                    return Ok(ApprovalOutcome::TimedOut);
                }
                polled = client.poll(&decision_id) => polled,
            };
            match polled {
                Ok(Some(response)) => {
                    return Ok(if response.approved {
                        ApprovalOutcome::Approved(Approved { _private: () })
                    } else {
                        ApprovalOutcome::Denied {
                            reason: response.reason,
                        }
                    });
                }
                Ok(None) => continue,
                Err(ApprovalError::Transport(_)) => continue,
                Err(other) => return Err(other),
            }
        }
    }

    /// The wall-clock deadline against which the hold is bounded.
    pub fn deadline(&self) -> SystemTime {
        self.deadline
    }
}

/// The runtime client for the approval receiver.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ApprovalClient {
    http: reqwest::Client,
    notify_url: reqwest::Url,
    status_url: reqwest::Url,
    hold_secs: u64,
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

/// Read a response body for an error diagnostic, naming a read failure
/// instead of silently replacing it with an empty body.
async fn diagnostic_body(response: reqwest::Response) -> String {
    match response.text().await {
        Ok(text) => text,
        Err(error) => format!("<body unreadable: {error}>"),
    }
}

impl ApprovalClient {
    /// Build a client from explicit notify/status URLs and a hold budget.
    ///
    /// `status_url` must end in `/`; the poll leg joins
    /// `<status_url><decision_id>/status`.  Tests use this constructor;
    /// production call sites use [`Self::for_section`].
    pub fn new(notify_url: reqwest::Url, status_url: reqwest::Url, hold_secs: u64) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
                .build()
                .expect("a timeout-only client builder is always valid"),
            notify_url,
            status_url,
            hold_secs,
        }
    }

    /// Create a client from the configured approval URL and hold budget.
    ///
    /// The configured URL IS the notify endpoint (the governance mirror's
    /// `/governance/workflow/authorize`); the status base derives from it
    /// by appending a trailing `/`, so the poll leg reads
    /// `<notify_url>/<decision_id>/status`.
    pub fn for_section(approval_url: &str, hold_secs: u64) -> Result<Self, ApprovalError> {
        let notify_url = reqwest::Url::parse(approval_url)
            .map_err(|error| ApprovalError::InvalidUrl(error.to_string()))?;
        let path = notify_url.path();
        let status_path = if path.ends_with('/') {
            path.to_owned()
        } else {
            format!("{path}/")
        };
        let mut status_url = notify_url.clone();
        status_url.set_path(&status_path);
        Ok(Self::new(notify_url, status_url, hold_secs))
    }

    /// Post the notify payload and return a hold that can be awaited.
    ///
    /// The decision identifier riding the wire was minted into the payload
    /// at its construction.  A re-POST of an already-decided id returns
    /// the existing decision; it never reopens the row.
    pub async fn notify(&self, payload: ApprovalPayload) -> Result<ApprovalHold, ApprovalError> {
        let body = serde_json::to_vec(&payload)
            .map_err(|error| ApprovalError::Transport(error.to_string()))?;
        let response = self
            .http
            .post(self.notify_url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|error| ApprovalError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let text = diagnostic_body(response).await;
            return Err(ApprovalError::UnexpectedStatus {
                status: status.as_u16(),
                body: text,
            });
        }
        Ok(ApprovalHold {
            client: self.clone(),
            payload,
            deadline: self.deadline(),
        })
    }

    /// Poll the status endpoint once.
    ///
    /// Returns the decoded decision for `200`, [`None`] for `202`/`207`
    /// pending, and an error for any other status.
    async fn poll(&self, decision_id: &str) -> Result<Option<DecisionResponse>, ApprovalError> {
        let url = self
            .status_url
            .join(&format!("{decision_id}/status"))
            .map_err(|error| ApprovalError::InvalidUrl(error.to_string()))?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|error| ApprovalError::Transport(error.to_string()))?;
        let status = response.status();
        match status {
            reqwest::StatusCode::OK => {
                let text = response
                    .text()
                    .await
                    .map_err(|error| ApprovalError::Transport(error.to_string()))?;
                serde_json::from_str::<DecisionResponse>(&text)
                    .map(Some)
                    .map_err(|error| {
                        ApprovalError::Transport(format!("decision body did not parse: {error}"))
                    })
            }
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::MULTI_STATUS => Ok(None),
            _ => {
                let body = diagnostic_body(response).await;
                Err(ApprovalError::UnexpectedStatus {
                    status: status.as_u16(),
                    body,
                })
            }
        }
    }

    /// The configured wall-clock deadline from now.
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
