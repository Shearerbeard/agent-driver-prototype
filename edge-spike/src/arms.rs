//! Evidence-window arms for the offline replay.

use crate::claim::{extract_claims, resolve_claims};
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
    match arm {
        SelectionArm::HeadTruncation => Ok(assemble_head_truncation(edge, config)),
        SelectionArm::ClaimIndexed => Ok(assemble_claim_indexed(edge, config)),
        SelectionArm::FlashSummarization => assemble_flash_summary(edge, config, summarizer).await,
    }
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

fn assemble_head_truncation(edge: &CapturedEdge, config: &ArmConfig) -> EdgeState {
    let full = edge.full_capture();
    let evidence = ClaimEvidence {
        claim: "head_truncated_capture".to_owned(),
        found_in: FoundIn::ToolCapture,
        window: Some(truncate_chars(&full, config.state_budget_chars / 4)),
        truncated: full.len() > config.state_budget_chars / 4,
    };
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
        claim_evidence: vec![evidence],
    }
}

fn assemble_claim_indexed(edge: &CapturedEdge, config: &ArmConfig) -> EdgeState {
    let full = edge.full_capture();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claims(&claims, &full, &edge.context_provided);

    let fixed = fixed_field_size(edge, config);
    let overhead = 200;
    let evidence_budget = config.state_budget_chars.saturating_sub(fixed + overhead);

    let mut claim_evidence = Vec::new();
    if evidence_budget >= config.window_floor {
        let per_claim = (evidence_budget / resolved.len().max(1)).max(config.per_claim_floor);
        for (claim, found_in) in resolved {
            let window = match found_in {
                FoundIn::NotFound => None,
                FoundIn::ToolCapture | FoundIn::ContextProvided => {
                    find_window(&full, &claim.text, per_claim)
                }
            };
            claim_evidence.push(ClaimEvidence {
                claim: claim.text,
                found_in,
                window,
                truncated: false,
            });
        }
    } else {
        // The evidence budget fell below the window floor: supported claims
        // get no window at all. Record that as assembly truncation so the
        // arm's loss is visible; unsupported claims have no window regardless.
        for (claim, found_in) in resolved {
            claim_evidence.push(ClaimEvidence {
                claim: claim.text,
                found_in,
                window: None,
                truncated: !matches!(found_in, FoundIn::NotFound),
            });
        }
    }

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
        claim_evidence,
    }
}

async fn assemble_flash_summary(
    edge: &CapturedEdge,
    config: &ArmConfig,
    summarizer: &dyn FlashSummarizer,
) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claims(&claims, &full, &edge.context_provided);

    let fixed = fixed_field_size(edge, config);
    let overhead = 200;
    let evidence_budget = config.state_budget_chars.saturating_sub(fixed + overhead);

    let mut claim_evidence = Vec::new();
    if evidence_budget >= config.window_floor {
        let per_claim = (evidence_budget / resolved.len().max(1)).max(config.per_claim_floor);
        for (claim, found_in) in resolved {
            let window = match found_in {
                FoundIn::NotFound => None,
                FoundIn::ToolCapture | FoundIn::ContextProvided => {
                    match find_window(&full, &claim.text, per_claim) {
                        Some(w) => Some(summarizer.summarize(&w, per_claim).await?),
                        None => None,
                    }
                }
            };
            claim_evidence.push(ClaimEvidence {
                claim: claim.text,
                found_in,
                window,
                truncated: false,
            });
        }
    } else {
        // The evidence budget fell below the window floor: supported claims
        // get no window at all. Record that as assembly truncation so the
        // arm's loss is visible; unsupported claims have no window regardless.
        for (claim, found_in) in resolved {
            claim_evidence.push(ClaimEvidence {
                claim: claim.text,
                found_in,
                window: None,
                truncated: !matches!(found_in, FoundIn::NotFound),
            });
        }
    }

    Ok(EdgeState {
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
        claim_evidence,
    })
}

fn fixed_field_size(edge: &CapturedEdge, config: &ArmConfig) -> usize {
    edge.goal.len().min(config.goal_cap)
        + edge.plan_step.task.len().min(config.plan_step_cap)
        + edge
            .plan_context
            .consumers
            .iter()
            .map(|t| t.task.len())
            .sum::<usize>()
            .min(config.plan_context_cap)
        + edge
            .plan_context
            .siblings
            .iter()
            .map(|t| t.task.len())
            .sum::<usize>()
            .min(config.plan_context_cap)
        + edge
            .worker_output
            .result
            .len()
            .min(config.worker_output_cap)
        + edge.context_provided.len().min(config.context_provided_cap)
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
