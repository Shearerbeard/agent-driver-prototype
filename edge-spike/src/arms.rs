//! Evidence-window arms for the offline replay.

use crate::claim::{ClaimResolution, extract_claims, resolve_claim_positions};
use crate::corpus::{CapturedEdge, ToolCallSpan, is_evidence_call};
use crate::state::{
    ClaimEvidenceV2, ClaimRef, EdgeState, EvidenceRegion, FoundIn, PlanContext, PlanStep,
    WorkerOutput,
};
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
    /// Floor for a per-region excerpt before the region degrades to `None`.
    pub region_floor: usize,
    pub fan_out_threshold: usize,
    /// Maximum characters of the enclosing tool call's arguments to keep as
    /// input provenance for a region.
    pub args_excerpt_cap: usize,
    /// Cluster capture hit positions within this many characters into one region.
    pub region_cluster_gap: usize,
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
            region_floor: 500,
            fan_out_threshold: 1_000,
            args_excerpt_cap: 400,
            region_cluster_gap: 1_500,
        }
    }
}

fn assemble_head_truncation(edge: &CapturedEdge, config: &ArmConfig) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let excerpt = truncate_chars(&full, config.state_budget_chars / 4);
    let truncated = full.chars().count() > excerpt.chars().count();
    let region = EvidenceRegion {
        region_id: 0,
        tool_call_id: "head".to_owned(),
        tool_name: "head_truncation".to_owned(),
        args_excerpt: String::new(),
        excerpt: Some(excerpt),
        span: (0, full.chars().count()),
        full_region_chars: full.chars().count(),
        source_truncated: edge
            .tool_calls
            .iter()
            .filter(|c| is_evidence_call(&c.tool_name))
            .any(|c| c.result.contains(" [truncated]")),
        truncated,
    };
    let claim_evidence = ClaimEvidenceV2 {
        regions: vec![region],
        claims: vec![ClaimRef {
            claim: "head_truncated_capture".to_owned(),
            found_in: FoundIn::ToolCapture,
            region_id: Some(0),
        }],
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
                || edge.worker_output.result.chars().count() > config.worker_output_cap,
        },
        context_provided: truncate_chars(&edge.context_provided, config.context_provided_cap),
        claim_evidence,
    };
    assert_within_budget(
        &state,
        config.state_budget_chars,
        SelectionArm::HeadTruncation,
    )?;
    Ok(state)
}

fn assemble_claim_indexed(edge: &CapturedEdge, config: &ArmConfig) -> anyhow::Result<EdgeState> {
    let (full, tool_index) = edge.full_capture_with_index();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claim_positions(&claims, &full, &edge.context_provided);

    let mut state = build_fixed_state(edge, config);
    assert_fixed_within_budget(&state, config, SelectionArm::ClaimIndexed)?;

    state.claim_evidence = build_region_evidence(&state, config, &full, &tool_index, resolved);
    trim_regions_to_budget(&mut state, config.state_budget_chars);
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
    let (full, tool_index) = edge.full_capture_with_index();
    let claims = extract_claims(&edge.worker_output.result);
    let (resolved, _) = resolve_claim_positions(&claims, &full, &edge.context_provided);

    let mut state = build_fixed_state(edge, config);
    assert_fixed_within_budget(&state, config, SelectionArm::FlashSummarization)?;

    let mut evidence = build_region_evidence(&state, config, &full, &tool_index, resolved);
    for region in &mut evidence.regions {
        if let Some(excerpt) = &region.excerpt {
            let summarized = summarizer
                .summarize(excerpt, excerpt.chars().count())
                .await?;
            region.excerpt = Some(summarized);
        }
    }
    state.claim_evidence = evidence;
    trim_regions_to_budget(&mut state, config.state_budget_chars);
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
                || edge.worker_output.result.chars().count() > config.worker_output_cap,
        },
        context_provided: truncate_chars(&edge.context_provided, config.context_provided_cap),
        claim_evidence: ClaimEvidenceV2 {
            regions: Vec::new(),
            claims: Vec::new(),
        },
    }
}

/// Fixed-size JSON cost of the `claim_evidence` object beyond the variable
/// region excerpts. Includes the wrapper, per-region metadata (including the
/// bounded `args_excerpt`), and per-claim text + punctuation.
fn evidence_v2_fixed_cost(
    candidates: &[RegionCandidate],
    resolved: &[ClaimResolution],
    args_excerpt_cap: usize,
) -> usize {
    let wrapper = 100;
    let region_meta = candidates.len() * (180 + args_excerpt_cap);
    let claims: usize = resolved
        .iter()
        .map(|r| r.claim.text.chars().count() + 55)
        .sum();
    wrapper + region_meta + claims
}

/// Cluster sorted hit positions into regions where adjacent hits are within
/// `max_gap` characters.
fn cluster_positions(positions: &[usize], max_gap: usize) -> Vec<(usize, usize)> {
    if positions.is_empty() {
        return Vec::new();
    }
    let mut regions = Vec::new();
    let mut start = positions[0];
    let mut end = positions[0];
    for &pos in positions.iter().skip(1) {
        if pos.saturating_sub(end) <= max_gap {
            end = pos;
        } else {
            regions.push((start, end));
            start = pos;
            end = pos;
        }
    }
    regions.push((start, end));
    regions
}

fn build_args_excerpt(args: &serde_json::Value, cap: usize) -> String {
    let raw = serde_json::to_string(args).unwrap_or_default();
    truncate_chars(&raw, cap)
}

fn take_region_excerpt(full: &str, span: (usize, usize), target: usize) -> String {
    let (span_start, span_end) = span;
    let span_len = span_end - span_start;
    if span_len <= target {
        return full.chars().skip(span_start).take(span_len).collect();
    }
    let mid = (span_start + span_end) / 2;
    let half = target / 2;
    let start = mid.saturating_sub(half);
    let mut end = start + target;
    if end > span_end {
        end = span_end;
    }
    let mut adj_start = start;
    if end - start < target {
        adj_start = end.saturating_sub(target);
    }
    full.chars().skip(adj_start).take(end - adj_start).collect()
}

#[derive(Debug, Clone)]
struct RegionCandidate {
    region_id: usize,
    span: (usize, usize),
    first_pos: usize,
    support: Vec<usize>, // indices of claims that hit inside this region
    tool: RegionTool,
}

/// How a region is attributed to the captured tool calls it overlaps.
#[derive(Debug, Clone)]
enum RegionTool {
    /// No captured call overlaps the region span.
    None,
    /// Exactly one call overlaps; keep its exact provenance.
    Single(ToolCallSpan),
    /// More than one call overlaps; no single call owns the region.
    Multi { source_truncated: bool },
}

/// Attribute a region span to the captured tool calls it overlaps. A span that
/// touches exactly one call keeps that call's provenance; one that touches
/// several is marked `multi` rather than mislabelled with the first call.
fn region_tool_attribution(tool_index: &[ToolCallSpan], span: (usize, usize)) -> RegionTool {
    let overlapping: Vec<&ToolCallSpan> = tool_index
        .iter()
        .filter(|t| span.0 < t.end_char && span.1 >= t.start_char)
        .collect();
    match overlapping.as_slice() {
        [] => RegionTool::None,
        [single] => RegionTool::Single((*single).clone()),
        many => RegionTool::Multi {
            source_truncated: many.iter().any(|t| t.result.contains(" [truncated]")),
        },
    }
}

/// Discover regions from claim hit positions, rank them by support count and
/// first occurrence, then build the v2 evidence object.
fn build_region_evidence(
    fixed_state: &EdgeState,
    config: &ArmConfig,
    full: &str,
    tool_index: &[ToolCallSpan],
    resolved: Vec<ClaimResolution>,
) -> ClaimEvidenceV2 {
    // Collect all capture hit positions.
    let mut all_positions: Vec<usize> = resolved
        .iter()
        .flat_map(|r| r.positions.iter().copied())
        .collect();
    all_positions.sort_unstable();
    all_positions.dedup();

    let clusters = cluster_positions(&all_positions, config.region_cluster_gap);
    let mut candidates: Vec<RegionCandidate> = clusters
        .into_iter()
        .enumerate()
        .map(|(id, cluster)| RegionCandidate {
            region_id: id,
            span: cluster,
            first_pos: cluster.0,
            support: Vec::new(),
            tool: region_tool_attribution(tool_index, cluster),
        })
        .collect();

    // Compute support: a claim supports every region whose span contains one
    // of its hit positions.
    for (claim_idx, resolution) in resolved.iter().enumerate() {
        for pos in &resolution.positions {
            for candidate in &mut candidates {
                if *pos >= candidate.span.0 && *pos < candidate.span.1 {
                    candidate.support.push(claim_idx);
                }
            }
        }
    }
    for candidate in &mut candidates {
        candidate.support.sort_unstable();
        candidate.support.dedup();
    }

    // Rank by support count descending, then by first occurrence ascending.
    candidates.sort_by(|a, b| {
        b.support
            .len()
            .cmp(&a.support.len())
            .then_with(|| a.first_pos.cmp(&b.first_pos))
    });

    // Re-assign stable region ids after ranking.
    for (new_id, candidate) in candidates.iter_mut().enumerate() {
        candidate.region_id = new_id;
    }

    // Map each claim to the highest-ranked region that contains it.
    let mut claim_to_region: Vec<Option<usize>> = vec![None; resolved.len()];
    for (claim_idx, resolution) in resolved.iter().enumerate() {
        if resolution.found_in != FoundIn::ToolCapture {
            continue;
        }
        for candidate in &candidates {
            if resolution
                .positions
                .iter()
                .any(|pos| *pos >= candidate.span.0 && *pos < candidate.span.1)
            {
                claim_to_region[claim_idx] = Some(candidate.region_id);
                break;
            }
        }
    }

    let fixed_cost = evidence_v2_fixed_cost(&candidates, &resolved, config.args_excerpt_cap);
    let budget = config
        .state_budget_chars
        .saturating_sub(fixed_state.char_len())
        .saturating_sub(fixed_cost);

    let regions: Vec<EvidenceRegion> = if candidates.is_empty() {
        Vec::new()
    } else {
        allocate_regions(
            &candidates,
            budget,
            config.region_floor,
            config.args_excerpt_cap,
            full,
        )
    };

    // If the budget was zero, all capture-backed claims degrade to references.
    let claims: Vec<ClaimRef> = resolved
        .into_iter()
        .enumerate()
        .map(|(idx, resolution)| ClaimRef {
            claim: resolution.claim.text,
            found_in: resolution.found_in,
            region_id: if resolution.found_in == FoundIn::ToolCapture {
                claim_to_region[idx].or_else(|| {
                    // Fallback to the highest-ranked region if clustering missed
                    // the exact position but the claim was still found.
                    candidates.first().map(|c| c.region_id)
                })
            } else {
                None
            },
        })
        .collect();

    ClaimEvidenceV2 { regions, claims }
}

/// Allocate excerpts to ranked regions under the evidence budget.
fn allocate_regions(
    candidates: &[RegionCandidate],
    budget: usize,
    floor: usize,
    args_excerpt_cap: usize,
    full: &str,
) -> Vec<EvidenceRegion> {
    let mut active: Vec<usize> = (0..candidates.len()).collect();
    let mut allocations: Vec<Option<usize>> = vec![None; candidates.len()];

    loop {
        if active.is_empty() {
            break;
        }
        let n = active.len().max(1);
        let share = budget / n;
        if share < floor {
            // Degrade the tail region.
            let tail = *active.last().expect("active non-empty");
            allocations[tail] = None;
            active.pop();
            continue;
        }

        // Compute desired size for each active region.
        let mut total = 0usize;
        for &i in &active {
            let span_len = candidates[i].span.1 - candidates[i].span.0;
            let want = share.min(span_len).max(floor);
            total += want;
        }

        if total <= budget {
            for &i in &active {
                let span_len = candidates[i].span.1 - candidates[i].span.0;
                allocations[i] = Some(share.min(span_len));
            }
            break;
        }

        // Proportional shrink.
        let factor = budget as f64 / total as f64;
        let mut below_floor = Vec::new();
        for &i in &active {
            let span_len = candidates[i].span.1 - candidates[i].span.0;
            let target = (span_len.max(floor) as f64 * factor) as usize;
            if target < floor {
                below_floor.push(i);
            }
        }

        if below_floor.is_empty() {
            for &i in &active {
                let span_len = candidates[i].span.1 - candidates[i].span.0;
                let target = ((span_len.max(floor) as f64 * factor) as usize)
                    .min(span_len)
                    .max(floor);
                allocations[i] = Some(target);
            }
            break;
        }

        // Degrade the tail active region that fell below the floor.
        let tail = *below_floor.last().expect("below_floor non-empty");
        allocations[tail] = None;
        active.retain(|i| *i != tail);
    }

    candidates
        .iter()
        .enumerate()
        .map(|(idx, candidate)| {
            let span_len = candidate.span.1 - candidate.span.0;
            let (excerpt, truncated) = match allocations[idx] {
                Some(0) | None => (None, true),
                Some(target) => {
                    let excerpt = take_region_excerpt(full, candidate.span, target);
                    (Some(excerpt), target < span_len)
                }
            };
            let (tool_call_id, tool_name, args_excerpt, source_truncated) = match &candidate.tool {
                RegionTool::Single(t) => (
                    t.tool_call_id.clone(),
                    t.tool_name.clone(),
                    build_args_excerpt(&t.arguments, args_excerpt_cap),
                    t.result.contains(" [truncated]"),
                ),
                RegionTool::Multi { source_truncated } => (
                    String::new(),
                    "multi".to_owned(),
                    String::new(),
                    *source_truncated,
                ),
                RegionTool::None => (String::new(), String::new(), String::new(), false),
            };
            EvidenceRegion {
                region_id: candidate.region_id,
                tool_call_id,
                tool_name,
                args_excerpt,
                excerpt,
                span: candidate.span,
                full_region_chars: span_len,
                source_truncated,
                truncated,
            }
        })
        .collect()
}

/// If the assembled state still exceeds the budget, degrade tail regions
/// until it fits. This is a deterministic safety valve; the by-construction
/// packing above should rarely need it.
fn trim_regions_to_budget(state: &mut EdgeState, budget: usize) {
    while state.char_len() > budget {
        if let Some(idx) = state
            .claim_evidence
            .regions
            .iter()
            .rposition(|r| r.excerpt.is_some())
        {
            state.claim_evidence.regions[idx].excerpt = None;
            state.claim_evidence.regions[idx].truncated = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim::ClaimClass;
    use crate::corpus::{CapturedEdge, ToolCall};
    use crate::state::{FoundIn, PlanContext, PlanStep, WorkerOutput};

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
    fn claim_indexed_fits_budget_and_keeps_region_excerpts() {
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
            state.claim_evidence.claims.len() >= 80,
            "expected at least 80 claims, got {}",
            state.claim_evidence.claims.len()
        );
        assert!(
            !state.claim_evidence.regions.is_empty(),
            "expected at least one region"
        );
        assert!(
            state
                .claim_evidence
                .regions
                .iter()
                .any(|r| r.excerpt.is_some()),
            "expected some regions to keep excerpts"
        );
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

    /// 100 short claims spread across 5 tight clusters in the capture. Region
    /// discovery should collapse them to ~5 regions, every capture-backed
    /// claim should reference a region, and the state should fit budget.
    fn clustered_edge() -> CapturedEdge {
        let cluster_count = 5;
        let claims_per_cluster = 20;

        let mut result_lines = Vec::new();
        let mut capture_calls = Vec::new();
        for cluster in 0..cluster_count {
            for i in 0..claims_per_cluster {
                let token = format!("task_H{cluster:02}{i:03}");
                result_lines.push(format!("Node {token} latency 150ms."));
            }
        }
        let result = result_lines.join("\n");

        let gap_filler = "y".repeat(2_000);
        for cluster in 0..cluster_count {
            let mut call_result = vec![gap_filler.clone()];
            for i in 0..claims_per_cluster {
                let token = format!("task_H{cluster:02}{i:03}");
                call_result.push(format!("{token} 150ms db-pool-{cluster:02}"));
            }
            capture_calls.push(ToolCall {
                tool_call_id: format!("tc{cluster}"),
                tool_name: "query".to_owned(),
                arguments: serde_json::json!({"pool": format!("db-pool-{cluster:02}")}),
                result: call_result.join("\n"),
                success: true,
            });
        }

        CapturedEdge {
            scenario: "db-pool-shaped".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "db-analyst".to_owned(),
            goal: "diagnose db latency".to_owned(),
            plan_step: PlanStep {
                task: "find slow pools".to_owned(),
                worker_role: "db-analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "db findings".to_owned(),
                result,
                truncated: false,
            },
            tool_calls: capture_calls,
            confidence: None,
        }
    }

    #[test]
    fn clustered_claims_collapse_to_five_regions() {
        let edge = clustered_edge();
        let config = ArmConfig::default();
        let state = assemble_claim_indexed(&edge, &config).unwrap();

        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
        assert!(
            state.claim_evidence.claims.len() >= 90,
            "expected ~100 extracted claims, got {}",
            state.claim_evidence.claims.len()
        );
        assert_eq!(
            state.claim_evidence.regions.len(),
            5,
            "expected 5 clustered regions, got {}",
            state.claim_evidence.regions.len()
        );

        let referenced_regions: std::collections::HashSet<usize> = state
            .claim_evidence
            .claims
            .iter()
            .filter(|c| c.found_in == FoundIn::ToolCapture)
            .filter_map(|c| c.region_id)
            .collect();
        assert_eq!(
            referenced_regions.len(),
            5,
            "all capture-backed claims should reference one of the 5 regions"
        );

        let degraded = state
            .claim_evidence
            .regions
            .iter()
            .filter(|r| r.excerpt.is_none())
            .count();
        let with_excerpt = state.claim_evidence.regions.len().saturating_sub(degraded);
        assert!(
            with_excerpt >= 3,
            "expected most regions to carry excerpts, only {with_excerpt} did"
        );
    }

    /// db-pool-shaped edge: the worker reports percentages (`51.1%`) while
    /// the tool JSON stores the same values as bare numbers (`"pct": 51.1`).
    fn db_pool_pct_edge() -> CapturedEdge {
        let result = "Broad discovery: payments 51.1%, catalog 28.6%, checkout 15.0%. Node \
                      task_DBPOOL00001 reported the onset."
            .to_owned();
        let capture = "{\n  \"buckets\": [\n    {\"count\": 8481, \"pct\": 51.1, \"value\": \
                       \"payments\"},\n    {\"count\": 3656, \"pct\": 28.6, \"value\": \
                       \"catalog\"},\n    {\"count\": 2496, \"pct\": 15.0, \"value\": \
                       \"checkout\"}\n  ]\n}";
        CapturedEdge {
            scenario: "db-pool-pct".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "log-analyst".to_owned(),
            goal: "diagnose db pool exhaustion".to_owned(),
            plan_step: PlanStep {
                task: "find the slow pool".to_owned(),
                worker_role: "log-analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "pct findings".to_owned(),
                result,
                truncated: false,
            },
            tool_calls: vec![ToolCall {
                tool_call_id: "tc1".to_owned(),
                tool_name: "group_logs_by_field".to_owned(),
                arguments: serde_json::json!({"field": "_app"}),
                result: capture.to_owned(),
                success: true,
            }],
            confidence: None,
        }
    }

    #[test]
    fn db_pool_pct_claims_resolve_and_state_fits_budget() {
        let edge = db_pool_pct_edge();
        let full = edge.full_capture();
        let claims = extract_claims(&edge.worker_output.result);
        let (resolved, stats) = resolve_claim_positions(&claims, &full, &edge.context_provided);

        let pct_claims: Vec<&ClaimResolution> = resolved
            .iter()
            .filter(|r| r.claim.class == ClaimClass::ValueWithUnit && r.claim.text.ends_with('%'))
            .collect();
        assert!(
            pct_claims.len() >= 3,
            "fixture must extract the percentage claims, got {}",
            pct_claims.len()
        );
        for resolution in &pct_claims {
            assert_eq!(
                resolution.found_in,
                FoundIn::ToolCapture,
                "{} should resolve via value-equivalence",
                resolution.claim.text
            );
            assert!(
                !resolution.positions.is_empty(),
                "{} must keep a usable position",
                resolution.claim.text
            );
        }
        assert_eq!(stats.normalized_matches, pct_claims.len());

        let config = ArmConfig::default();
        let state = assemble_claim_indexed(&edge, &config).unwrap();
        assert!(
            !state.claim_evidence.regions.is_empty(),
            "normalized hits must still produce evidence regions"
        );
        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
    }

    /// A worker whose result text appears only in its own `submit_result`
    /// echo, never in real tool output. The echo must not be evidence.
    fn self_echo_edge() -> CapturedEdge {
        let echoed = "The incident task_ECHO00001 started at 2026-07-17T00:00:00Z.".to_owned();
        CapturedEdge {
            scenario: "self-echo".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "analyst".to_owned(),
            goal: "diagnose".to_owned(),
            plan_step: PlanStep {
                task: "find the cause".to_owned(),
                worker_role: "analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "findings".to_owned(),
                result: echoed.clone(),
                truncated: false,
            },
            tool_calls: vec![
                ToolCall {
                    tool_call_id: "tc-real".to_owned(),
                    tool_name: "query".to_owned(),
                    arguments: serde_json::json!({"q": "incident"}),
                    result: "no matching records".to_owned(),
                    success: true,
                },
                ToolCall {
                    tool_call_id: "tc-submit".to_owned(),
                    tool_name: "submit_result".to_owned(),
                    arguments: serde_json::json!({"result": echoed, "confidence": "high"}),
                    result: "submitted".to_owned(),
                    success: true,
                },
            ],
            confidence: None,
        }
    }

    #[test]
    fn self_echo_claims_resolve_not_found() {
        let edge = self_echo_edge();
        let full = edge.full_capture();
        assert!(
            !full.contains("task_ECHO00001"),
            "submit_result echo must not be part of the evidence capture"
        );

        let claims = extract_claims(&edge.worker_output.result);
        assert!(
            claims.iter().any(|c| c.text == "task_ECHO00001"),
            "the fixture must extract the echoed claim"
        );
        let (resolved, stats) = resolve_claim_positions(&claims, &full, &edge.context_provided);
        assert_eq!(
            stats.found_in_tool_capture, 0,
            "claims present only in the submit_result echo must not be found"
        );
        assert!(
            resolved.iter().all(|r| r.found_in == FoundIn::NotFound),
            "every self-echo claim must resolve not_found"
        );
    }

    /// Two real calls whose claims land close enough to cluster into one region
    /// spanning both calls.
    fn multi_call_region_edge() -> CapturedEdge {
        CapturedEdge {
            scenario: "multi-call".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "analyst".to_owned(),
            goal: "diagnose".to_owned(),
            plan_step: PlanStep {
                task: "find slow nodes".to_owned(),
                worker_role: "analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "findings".to_owned(),
                result: "Node task_MULTI00001 slow; node task_MULTI00002 slow.".to_owned(),
                truncated: false,
            },
            tool_calls: vec![
                ToolCall {
                    tool_call_id: "tc-a".to_owned(),
                    tool_name: "query".to_owned(),
                    arguments: serde_json::json!({"node": "a"}),
                    result: "task_MULTI00001 150ms".to_owned(),
                    success: true,
                },
                ToolCall {
                    tool_call_id: "tc-b".to_owned(),
                    tool_name: "query".to_owned(),
                    arguments: serde_json::json!({"node": "b"}),
                    result: "task_MULTI00002 150ms".to_owned(),
                    success: true,
                },
            ],
            confidence: None,
        }
    }

    #[test]
    fn multi_call_region_is_attributed_multi() {
        let edge = multi_call_region_edge();
        let config = ArmConfig::default();
        let state = assemble_claim_indexed(&edge, &config).unwrap();

        assert_eq!(
            state.claim_evidence.regions.len(),
            1,
            "the two near claims should cluster into one region"
        );
        let region = &state.claim_evidence.regions[0];
        assert_eq!(
            region.tool_name, "multi",
            "a region spanning two calls must not be attributed to one call"
        );
        assert!(
            region.tool_call_id.is_empty(),
            "a multi-call region has no single tool_call_id"
        );
    }

    /// db-pool-shaped edge: the worker's `submit_result` echoes the whole
    /// result. The echo must not create an evidence region.
    fn clustered_edge_with_submit_echo() -> CapturedEdge {
        let mut edge = clustered_edge();
        let echoed = edge.worker_output.result.clone();
        let summary = edge.worker_output.summary.clone();
        edge.tool_calls.push(ToolCall {
            tool_call_id: "tc-submit".to_owned(),
            tool_name: "submit_result".to_owned(),
            arguments: serde_json::json!({
                "summary": summary,
                "result": echoed,
                "confidence": "high",
            }),
            result: "submitted".to_owned(),
            success: true,
        });
        edge
    }

    #[test]
    fn submit_result_echo_is_not_an_evidence_region() {
        let edge = clustered_edge_with_submit_echo();
        let config = ArmConfig::default();
        let state = assemble_claim_indexed(&edge, &config).unwrap();

        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
        assert_eq!(
            state.claim_evidence.regions.len(),
            5,
            "the submit_result echo must not add regions: {:?}",
            state
                .claim_evidence
                .regions
                .iter()
                .map(|r| r.tool_name.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            state
                .claim_evidence
                .regions
                .iter()
                .all(|r| r.tool_name != "submit_result"),
            "no region may be attributed to the worker's own submission"
        );
        assert!(
            state.claim_evidence.claims.len() >= 90,
            "claims should still be extracted, got {}",
            state.claim_evidence.claims.len()
        );
    }

    /// A smaller fixture where the evidence budget is tight enough that the
    /// lowest-ranked regions degrade while claims still reference them.
    fn degradation_edge() -> CapturedEdge {
        let cluster_count = 5;
        let claims_per_cluster = 5;

        let mut result_lines = Vec::new();
        let mut capture_calls = Vec::new();
        for cluster in 0..cluster_count {
            for i in 0..claims_per_cluster {
                let token = format!("task_D{cluster:02}{i:03}");
                result_lines.push(format!("Node {token} latency 150ms."));
            }
        }
        let result = result_lines.join("\n");

        // Make each region artificially large so they cannot all fit.
        let filler = "x".repeat(4_000);
        for cluster in 0..cluster_count {
            let mut call_result = vec![filler.clone()];
            for i in 0..claims_per_cluster {
                let token = format!("task_D{cluster:02}{i:03}");
                call_result.push(format!("{token} 150ms pool-{cluster:02}"));
            }
            call_result.push(filler.clone());
            capture_calls.push(ToolCall {
                tool_call_id: format!("tc{cluster}"),
                tool_name: "query".to_owned(),
                arguments: serde_json::json!({"pool": format!("pool-{cluster:02}")}),
                result: call_result.join("\n"),
                success: true,
            });
        }

        CapturedEdge {
            scenario: "degradation".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "db-analyst".to_owned(),
            goal: "diagnose db latency".to_owned(),
            plan_step: PlanStep {
                task: "find slow pools".to_owned(),
                worker_role: "db-analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "db findings".to_owned(),
                result,
                truncated: false,
            },
            tool_calls: capture_calls,
            confidence: None,
        }
    }

    #[test]
    fn tiny_evidence_budget_degrades_tail_regions_by_support() {
        let edge = degradation_edge();
        let config = ArmConfig {
            state_budget_chars: 7_500,
            ..Default::default()
        };

        let state = assemble_claim_indexed(&edge, &config).unwrap();
        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );

        let degraded = state
            .claim_evidence
            .regions
            .iter()
            .filter(|r| r.excerpt.is_none() && r.truncated)
            .count();
        assert!(degraded > 0, "expected tail regions to degrade");

        for claim in &state.claim_evidence.claims {
            if claim.found_in == FoundIn::ToolCapture {
                assert!(
                    claim.region_id.is_some(),
                    "degraded regions must still be referenced by claims"
                );
            }
        }
    }

    #[test]
    fn claim_evidence_v2_round_trips_through_serde() {
        let evidence = ClaimEvidenceV2 {
            regions: vec![EvidenceRegion {
                region_id: 0,
                tool_call_id: "tc1".to_owned(),
                tool_name: "query".to_owned(),
                args_excerpt: "{}".to_owned(),
                excerpt: Some("result text".to_owned()),
                span: (0, 11),
                full_region_chars: 11,
                source_truncated: false,
                truncated: false,
            }],
            claims: vec![ClaimRef {
                claim: "host-001".to_owned(),
                found_in: FoundIn::ToolCapture,
                region_id: Some(0),
            }],
        };

        let json = serde_json::to_string(&evidence).expect("serialize");
        let parsed: ClaimEvidenceV2 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.regions.len(), 1);
        assert_eq!(parsed.claims.len(), 1);
        assert_eq!(parsed.regions[0].region_id, 0);
        assert_eq!(parsed.claims[0].region_id, Some(0));
    }
}
