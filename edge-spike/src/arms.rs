//! Evidence-window arms for the offline replay.

use crate::claim::{ExtractedClaim, extract_claims, resolve_claims};
use crate::corpus::CapturedEdge;
use crate::state::{ClaimEvidence, EdgeState, FoundIn, PlanContext, PlanStep, WorkerOutput};
use crate::summarizer::FlashSummarizer;

/// Selection arm identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionArm {
    HeadTruncation,
    ClaimIndexed,
    FlashSummarization,
}

impl SelectionArm {
    /// All arms in experiment order.
    #[must_use]
    pub fn all() -> &'static [SelectionArm] {
        &[
            SelectionArm::HeadTruncation,
            SelectionArm::ClaimIndexed,
            SelectionArm::FlashSummarization,
        ]
    }

    /// Human-readable name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SelectionArm::HeadTruncation => "head_truncation",
            SelectionArm::ClaimIndexed => "claim_indexed",
            SelectionArm::FlashSummarization => "flash_summarization",
        }
    }
}

/// Build an `EdgeState` for the given arm.
pub async fn assemble_state(
    edge: &CapturedEdge,
    arm: SelectionArm,
    config: &ArmConfig,
    summarizer: &dyn FlashSummarizer,
) -> anyhow::Result<EdgeState> {
    let state = match arm {
        SelectionArm::HeadTruncation => assemble_head_truncation(edge, config)?,
        SelectionArm::ClaimIndexed => assemble_claim_indexed(edge, config)?,
        SelectionArm::FlashSummarization => {
            assemble_flash_summary(edge, config, summarizer).await?
        }
    };
    Ok(state)
}

/// Configuration constants for the arms.
#[derive(Debug, Clone, Copy)]
pub struct ArmConfig {
    pub state_budget_chars: usize,
    pub goal_cap: usize,
    pub plan_step_cap: usize,
    pub plan_context_cap: usize,
    pub worker_output_cap: usize,
    pub context_provided_cap: usize,
    pub window_floor: usize,
    pub per_claim_floor: usize,
    pub fan_out_threshold: usize,
}

impl Default for ArmConfig {
    fn default() -> Self {
        Self {
            state_budget_chars: 24_000,
            goal_cap: 2_000,
            plan_step_cap: 2_000,
            plan_context_cap: 5_000,
            worker_output_cap: 7_000,
            context_provided_cap: 4_000,
            window_floor: 4_000,
            per_claim_floor: 500,
            fan_out_threshold: 1_000,
        }
    }
}

fn assemble_head_truncation(edge: &CapturedEdge, config: &ArmConfig) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let evidence = ClaimEvidence {
        claim: "head_truncated_capture".to_owned(),
        found_in: FoundIn::ToolCapture,
        window: Some(truncate_chars(&full, config.state_budget_chars / 4)),
        truncated: full.len() > config.state_budget_chars / 4,
    };
    let state = EdgeState {
        goal: truncate_chars(&edge.goal, config.goal_cap),
        plan_step: PlanStep {
            task: truncate_chars(&edge.plan_step.task, config.plan_step_cap),
            worker_role: edge.plan_step.worker_role.clone(),
            depends_on: edge.plan_step.depends_on.clone(),
        },
        plan_context: truncate_plan_context(&edge.plan_context, config.plan_context_cap),
        worker_output: WorkerOutput {
            summary: edge.worker_output.summary.clone(),
            result: truncate_chars(&edge.worker_output.result, config.worker_output_cap),
            truncated: edge.worker_output.truncated
                || edge.worker_output.result.len() > config.worker_output_cap,
        },
        context_provided: truncate_chars(&edge.context_provided, config.context_provided_cap),
        claim_evidence: vec![evidence],
    };
    assert_within_budget(
        &state,
        config.state_budget_chars,
        SelectionArm::HeadTruncation,
    )?;
    Ok(state)
}

fn assemble_claim_indexed(edge: &CapturedEdge, config: &ArmConfig) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claims(&claims, &full, &edge.context_provided);

    let mut state = build_fixed_state(edge, config);
    assert_fixed_within_budget(&state, config, SelectionArm::ClaimIndexed)?;

    state.claim_evidence = fit_claim_evidence(&state, config, &full, resolved);
    trim_claim_evidence_to_budget(&mut state, config.state_budget_chars);
    assert_within_budget(
        &state,
        config.state_budget_chars,
        SelectionArm::ClaimIndexed,
    )?;
    Ok(state)
}

async fn assemble_flash_summary(
    edge: &CapturedEdge,
    config: &ArmConfig,
    summarizer: &dyn FlashSummarizer,
) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claims(&claims, &full, &edge.context_provided);

    let mut state = build_fixed_state(edge, config);
    assert_fixed_within_budget(&state, config, SelectionArm::FlashSummarization)?;

    let fitted = fit_claim_evidence(&state, config, &full, resolved);
    let mut claim_evidence = Vec::with_capacity(fitted.len());
    for ev in fitted {
        let window = match ev.window {
            Some(w) => Some(summarizer.summarize(&w, w.len()).await?),
            None => None,
        };
        claim_evidence.push(ClaimEvidence {
            claim: ev.claim,
            found_in: ev.found_in,
            window,
            truncated: ev.truncated,
        });
    }
    state.claim_evidence = claim_evidence;
    trim_claim_evidence_to_budget(&mut state, config.state_budget_chars);
    assert_within_budget(
        &state,
        config.state_budget_chars,
        SelectionArm::FlashSummarization,
    )?;
    Ok(state)
}

/// Build the fixed fields of the state, before any claim evidence is added.
fn build_fixed_state(edge: &CapturedEdge, config: &ArmConfig) -> EdgeState {
    EdgeState {
        goal: truncate_chars(&edge.goal, config.goal_cap),
        plan_step: PlanStep {
            task: truncate_chars(&edge.plan_step.task, config.plan_step_cap),
            worker_role: edge.plan_step.worker_role.clone(),
            depends_on: edge.plan_step.depends_on.clone(),
        },
        plan_context: truncate_plan_context(&edge.plan_context, config.plan_context_cap),
        worker_output: WorkerOutput {
            summary: edge.worker_output.summary.clone(),
            result: truncate_chars(&edge.worker_output.result, config.worker_output_cap),
            truncated: edge.worker_output.truncated
                || edge.worker_output.result.len() > config.worker_output_cap,
        },
        context_provided: truncate_chars(&edge.context_provided, config.context_provided_cap),
        claim_evidence: Vec::new(),
    }
}

/// Approximate JSON overhead for the `claim_evidence` array beyond the
/// variable claim/window text.
fn evidence_overhead(claim_count: usize) -> usize {
    // Array brackets, commas, object punctuation, keys, and enum values.
    200 + claim_count * 80
}

/// Fit claim windows collectively into the remaining budget.
///
/// Deterministic first-seen order. Each claim starts with an equal share of
/// the evidence budget. Windows shrink proportionally until the total fits;
/// claims whose allocated window falls below the floor are degraded to a
/// `found_in` marker, starting from the tail so early evidence keeps windows.
fn fit_claim_evidence(
    fixed_state: &EdgeState,
    config: &ArmConfig,
    full: &str,
    resolved: Vec<(ExtractedClaim, FoundIn)>,
) -> Vec<ClaimEvidence> {
    let budget = config
        .state_budget_chars
        .saturating_sub(fixed_state.char_len())
        .saturating_sub(evidence_overhead(resolved.len()));

    if budget == 0 {
        return resolved
            .into_iter()
            .map(|(claim, found_in)| ClaimEvidence {
                claim: claim.text,
                found_in,
                window: None,
                truncated: !matches!(found_in, FoundIn::NotFound),
            })
            .collect();
    }

    let floor = config.per_claim_floor;
    let n = resolved.len().max(1);
    let share = (budget / n).max(floor);

    struct Candidate {
        claim: ExtractedClaim,
        found_in: FoundIn,
        window: Option<String>,
        len: usize,
    }

    let mut candidates: Vec<Candidate> = resolved
        .into_iter()
        .map(|(claim, found_in)| {
            let window = if matches!(found_in, FoundIn::NotFound) {
                None
            } else {
                find_window(full, &claim.text, share)
            };
            let len = window.as_ref().map(|w| w.len()).unwrap_or(0);
            Candidate {
                claim,
                found_in,
                window,
                len,
            }
        })
        .collect();

    // Active indices = claims that currently have a window.
    let mut active: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| if c.window.is_some() { Some(i) } else { None })
        .collect();

    loop {
        let total: usize = active.iter().map(|i| candidates[*i].len).sum();
        if active.is_empty() || total <= budget {
            break;
        }

        let factor = budget as f64 / total as f64;
        let mut below_floor: Vec<(usize, usize)> = active
            .iter()
            .map(|&i| {
                let target = (candidates[i].len as f64 * factor) as usize;
                (i, target)
            })
            .filter(|(_, target)| *target < floor)
            .collect();

        if below_floor.is_empty() {
            // All active claims stay above the floor after shrinking.
            for &i in &active {
                let target = (candidates[i].len as f64 * factor) as usize;
                let target = target.max(floor);
                candidates[i].window = find_window(full, &candidates[i].claim.text, target);
                candidates[i].len = candidates[i].window.as_ref().map(|w| w.len()).unwrap_or(0);
            }
            break;
        }

        // Degrade the tail claim that fell below the floor.
        below_floor.sort_by_key(|(i, _)| *i);
        let tail = below_floor.last().map(|(i, _)| *i).unwrap();
        candidates[tail].window = None;
        candidates[tail].len = 0;
        active.retain(|i| *i != tail);
    }

    candidates
        .into_iter()
        .map(|c| {
            let truncated = c.window.is_none() && !matches!(c.found_in, FoundIn::NotFound);
            ClaimEvidence {
                claim: c.claim.text,
                found_in: c.found_in,
                window: c.window,
                truncated,
            }
        })
        .collect()
}

/// If the assembled state still exceeds the budget, degrade tail windows
/// until it fits. This is a deterministic safety valve; the by-construction
/// packing above should rarely need it.
fn trim_claim_evidence_to_budget(state: &mut EdgeState, budget: usize) {
    while state.char_len() > budget {
        if let Some(idx) = state
            .claim_evidence
            .iter()
            .rposition(|c| c.window.is_some())
        {
            state.claim_evidence[idx].window = None;
            state.claim_evidence[idx].truncated =
                !matches!(state.claim_evidence[idx].found_in, FoundIn::NotFound);
        } else {
            break;
        }
    }
}

fn assert_within_budget(state: &EdgeState, budget: usize, arm: SelectionArm) -> anyhow::Result<()> {
    let len = state.char_len();
    if len > budget {
        anyhow::bail!(
            "assembled {} state exceeds state_budget_chars: {len} > {budget}",
            arm.name()
        );
    }
    Ok(())
}

fn assert_fixed_within_budget(
    state: &EdgeState,
    config: &ArmConfig,
    arm: SelectionArm,
) -> anyhow::Result<()> {
    let len = state.char_len();
    if len > config.state_budget_chars {
        anyhow::bail!(
            "{} fixed fields exceed state_budget_chars: {len} > {}",
            arm.name(),
            config.state_budget_chars
        );
    }
    Ok(())
}

fn truncate_chars(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        s.to_owned()
    } else {
        let mut out = s.chars().take(limit).collect::<String>();
        out.push_str("... [truncated]");
        out
    }
}

fn truncate_plan_context(ctx: &PlanContext, limit: usize) -> PlanContext {
    let mut out = PlanContext {
        consumers: Vec::new(),
        siblings: Vec::new(),
    };
    let mut used = 0usize;
    for t in &ctx.consumers {
        let len = t.task.chars().count();
        if used + len > limit {
            break;
        }
        used += len;
        out.consumers.push(t.clone());
    }
    for t in &ctx.siblings {
        let len = t.task.chars().count();
        if used + len > limit {
            break;
        }
        used += len;
        out.siblings.push(t.clone());
    }
    out
}

fn find_window(full: &str, needle: &str, window_size: usize) -> Option<String> {
    let chars: Vec<char> = full.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    let char_pos = chars
        .windows(needle_chars.len())
        .position(|window| window == needle_chars.as_slice())?;
    let total_chars = chars.len();
    let start_char = char_pos.saturating_sub(window_size / 2);
    let end_char = (char_pos + needle_chars.len() + window_size / 2).min(total_chars);
    Some(chars[start_char..end_char].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{CapturedEdge, ToolCall};
    use crate::state::{PlanContext, PlanStep, WorkerOutput};

    fn big_edge() -> CapturedEdge {
        let mut result_lines = Vec::new();
        for i in 0..50 {
            result_lines.push(format!(
                "The host 'host-{i:03}.prod.internal' reported latency 150ms at timestamp 2026-07-17T22:{i:02}:00Z."
            ));
        }
        let result = result_lines.join("\n");

        let mut capture_lines = Vec::new();
        for i in 0..50 {
            capture_lines.push(format!(
                "tool: query\nhost-{i:03}.prod.internal 150ms 2026-07-17T22:{i:02}:00Z"
            ));
        }
        let capture = capture_lines.join("\n---\n");

        CapturedEdge {
            scenario: "packing-test".to_owned(),
            iteration: 0,
            task_id: 1,
            worker_id: "log-analyst".to_owned(),
            goal: "diagnose incident".to_owned(),
            plan_step: PlanStep {
                task: "find latency spikes".to_owned(),
                worker_role: "log-analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "latency findings".to_owned(),
                result,
                truncated: false,
            },
            tool_calls: vec![ToolCall {
                tool_call_id: "tc1".to_owned(),
                tool_name: "query".to_owned(),
                arguments: serde_json::json!({}),
                result: capture,
                success: true,
            }],
            confidence: None,
        }
    }

    #[test]
    fn claim_indexed_fits_budget_with_tail_degradation() {
        let edge = big_edge();
        let config = ArmConfig::default();
        let state = assemble_claim_indexed(&edge, &config).unwrap();

        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
        assert!(
            state.claim_evidence.len() >= 80,
            "expected at least 80 claims, got {}",
            state.claim_evidence.len()
        );

        let windowed = state
            .claim_evidence
            .iter()
            .filter(|c| c.window.is_some())
            .count();
        let degraded = state
            .claim_evidence
            .iter()
            .filter(|c| c.window.is_none() && c.truncated)
            .count();
        assert!(windowed > 0, "expected some claims to keep windows");
        assert!(degraded > 0, "expected tail claims to degrade to markers");
    }

    #[test]
    fn tiny_budget_fails_loud_before_send() {
        let edge = big_edge();
        let config = ArmConfig {
            state_budget_chars: 50,
            ..Default::default()
        };

        let err = assemble_claim_indexed(&edge, &config)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("claim_indexed"),
            "error should name the variant: {err}"
        );
        assert!(
            err.contains("fixed fields exceed state_budget_chars"),
            "error should explain the pre-send guard: {err}"
        );
    }
}
