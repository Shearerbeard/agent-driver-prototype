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
    /// Smallest excerpt size to try before a region degrades to `None`. A
    /// still-smaller excerpt that fits the budget may be kept.
    pub region_floor: usize,
    pub fan_out_threshold: usize,
    /// Maximum characters of the enclosing tool call's arguments to keep as
    /// input provenance for a region.
    pub args_excerpt_cap: usize,
    /// Cluster capture hit positions within this many characters into one region.
    pub region_cluster_gap: usize,
    /// Per-region excerpt budget. A region whose hit span exceeds this is
    /// split at its widest internal hit gap until every piece fits, so a
    /// region's excerpt can cover its own hits.
    pub region_excerpt_budget: usize,
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
            region_excerpt_budget: 4_000,
        }
    }
}

fn assemble_head_truncation(edge: &CapturedEdge, config: &ArmConfig) -> anyhow::Result<EdgeState> {
    let full = edge.full_capture();
    let full_len = full.chars().count();
    let limit = config.state_budget_chars / 4;
    let excerpt = truncate_chars(&full, limit);
    let truncated = full_len > excerpt.chars().count();
    // The head arm carries one synthetic claim reference, not the worker's
    // real claims, so it has no supported hit positions to lose: excerpting
    // the capture is labeled budgeting, not verdict-driving information loss.
    let region = EvidenceRegion {
        region_id: 0,
        tool_call_id: "head".to_owned(),
        tool_name: "head_truncation".to_owned(),
        args_excerpt: String::new(),
        excerpt: Some(excerpt),
        span: (0, full_len),
        full_region_chars: full_len,
        source_truncated: edge
            .tool_calls
            .iter()
            .filter(|c| is_evidence_call(&c.tool_name))
            .any(|c| c.result.contains(" [truncated]")),
        truncated,
        excerpt_range: Some((0, full_len.min(limit))),
        uncovered_hits: 0,
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

    state.claim_evidence = build_region_evidence(&state, config, &full, &tool_index, &resolved);
    trim_regions_to_budget(&mut state, config.state_budget_chars);
    apply_region_coverage(&mut state.claim_evidence, &resolved);
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

    let mut evidence = build_region_evidence(&state, config, &full, &tool_index, &resolved);
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
    apply_region_coverage(&mut state.claim_evidence, &resolved);
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

/// Borrowed view of the evidence object, so its serialized size can be
/// measured without owning a copy of the regions and claims.
#[derive(serde::Serialize)]
struct EvidenceRef<'a> {
    regions: &'a [EvidenceRegion],
    claims: &'a [ClaimRef],
}

/// Serialized character length of one region entry, exactly as serde writes it.
fn region_entry_chars(region: &EvidenceRegion) -> usize {
    serde_json::to_string(region)
        .map(|s| s.chars().count())
        .unwrap_or(usize::MAX)
}

/// Serialized character length of the whole `claim_evidence` object.
fn evidence_chars(regions: &[EvidenceRegion], claims: &[ClaimRef]) -> usize {
    serde_json::to_string(&EvidenceRef { regions, claims })
        .map(|s| s.chars().count())
        .unwrap_or(usize::MAX)
}

/// Cluster sorted hit positions into groups where adjacent hits are within
/// `max_gap` characters. Positions are assumed sorted and deduplicated.
fn cluster_positions(positions: &[usize], max_gap: usize) -> Vec<Vec<usize>> {
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    for &pos in positions {
        match clusters.last_mut() {
            Some(cluster)
                if pos.saturating_sub(*cluster.last().expect("cluster non-empty")) <= max_gap =>
            {
                cluster.push(pos);
            }
            _ => clusters.push(vec![pos]),
        }
    }
    clusters
}

/// Cut a hit cluster at its widest internal gap, returning the two pieces.
fn split_once(mut cluster: Vec<usize>) -> (Vec<usize>, Vec<usize>) {
    let last = *cluster.last().expect("cluster non-empty");
    let mut cut = 0usize;
    for i in 1..cluster.len() - 1 {
        if cluster[i + 1] - cluster[i] > cluster[cut + 1] - cluster[cut] {
            cut = i;
        }
    }
    // Among equally wide gaps, take the one nearest the span midpoint so the
    // recursion halves the chain instead of peeling off one hit at a time.
    let widest = cluster[cut + 1] - cluster[cut];
    let mid = cluster[0] + (last - cluster[0]) / 2;
    for i in 0..cluster.len() - 1 {
        if cluster[i + 1] - cluster[i] == widest
            && cluster[i + 1].abs_diff(mid) < cluster[cut + 1].abs_diff(mid)
        {
            cut = i;
        }
    }
    let right = cluster.split_off(cut + 1);
    (cluster, right)
}

/// Split a chained hit cluster until every piece's span fits `target`, cutting
/// at the widest gap between consecutive hits. This undoes single-linkage
/// chaining while leaving genuinely tight clusters intact.
fn split_cluster(cluster: Vec<usize>, target: usize) -> Vec<Vec<usize>> {
    let last = *cluster.last().expect("cluster non-empty");
    if cluster.len() < 2 || last + 1 - cluster[0] <= target {
        return vec![cluster];
    }
    let (left, right) = split_once(cluster);
    let mut pieces = split_cluster(left, target);
    pieces.extend(split_cluster(right, target));
    pieces
}

fn build_args_excerpt(args: &serde_json::Value, cap: usize) -> String {
    let raw = serde_json::to_string(args).unwrap_or_default();
    truncate_chars(&raw, cap)
}

/// Take the bounded excerpt for a region and return it with the character
/// range it covers within `full`.
fn take_region_excerpt(
    full: &str,
    span: (usize, usize),
    target: usize,
) -> (String, (usize, usize)) {
    let (span_start, span_end) = span;
    let span_len = span_end - span_start;
    if span_len <= target {
        let text = full.chars().skip(span_start).take(span_len).collect();
        return (text, (span_start, span_end));
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
    let text = full.chars().skip(adj_start).take(end - adj_start).collect();
    (text, (adj_start, end))
}

/// Count, per region, the supported claims that reference it but whose hit
/// positions fall outside its shown excerpt.
///
/// A claim is covered when at least one of its occurrences is inside the
/// excerpt of the region it references; otherwise the judge cannot recover its
/// support. Regions with no supported claims stay at zero: excerpting them is
/// budgeting, not information loss.
fn apply_region_coverage(evidence: &mut ClaimEvidenceV2, resolved: &[ClaimResolution]) {
    for region in &mut evidence.regions {
        region.uncovered_hits = 0;
    }
    for claim_ref in &evidence.claims {
        if claim_ref.found_in != FoundIn::ToolCapture {
            continue;
        }
        let Some(region_id) = claim_ref.region_id else {
            continue;
        };
        let Some(resolution) = resolved.iter().find(|r| r.claim.text == claim_ref.claim) else {
            continue;
        };
        if resolution.positions.is_empty() {
            continue;
        }
        let Some(region) = evidence
            .regions
            .iter_mut()
            .find(|r| r.region_id == region_id)
        else {
            continue;
        };
        let covered = resolution.positions.iter().any(|pos| {
            region
                .excerpt_range
                .is_some_and(|(start, end)| *pos >= start && *pos < end)
        });
        if !covered {
            region.uncovered_hits += 1;
        }
    }
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
    resolved: &[ClaimResolution],
) -> ClaimEvidenceV2 {
    // Collect all capture hit positions.
    let mut all_positions: Vec<usize> = resolved
        .iter()
        .flat_map(|r| r.positions.iter().copied())
        .collect();
    all_positions.sort_unstable();
    all_positions.dedup();

    let clusters = cluster_positions(&all_positions, config.region_cluster_gap);
    let clusters: Vec<Vec<usize>> = clusters
        .into_iter()
        .flat_map(|cluster| split_cluster(cluster, config.region_excerpt_budget))
        .collect();
    let mut candidates: Vec<RegionCandidate> = clusters
        .into_iter()
        .enumerate()
        .map(|(id, cluster)| {
            let span = (cluster[0], cluster[cluster.len() - 1] + 1);
            RegionCandidate {
                region_id: id,
                span,
                first_pos: span.0,
                support: Vec::new(),
                tool: region_tool_attribution(tool_index, span),
            }
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

    let claims: Vec<ClaimRef> = resolved
        .iter()
        .enumerate()
        .map(|(idx, resolution)| ClaimRef {
            claim: resolution.claim.text.clone(),
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

    // Allowance for the whole `claim_evidence` object: the state budget less the
    // fixed fields, with the empty evidence skeleton swapped back in so the
    // measured object replaces it exactly.
    let empty_evidence = ClaimEvidenceV2 {
        regions: Vec::new(),
        claims: Vec::new(),
    };
    let empty_cost = serde_json::to_string(&empty_evidence)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let evidence_budget = config
        .state_budget_chars
        .saturating_sub(fixed_state.char_len())
        .saturating_add(empty_cost);

    let regions: Vec<EvidenceRegion> = if candidates.is_empty() {
        Vec::new()
    } else {
        allocate_regions(&candidates, evidence_budget, config, full, &claims)
    };

    ClaimEvidenceV2 { regions, claims }
}

/// Attribute one region's provenance to the captured tool call(s) it overlaps.
fn region_attribution(
    candidate: &RegionCandidate,
    args_excerpt_cap: usize,
) -> (String, String, String, bool) {
    match &candidate.tool {
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
    }
}

/// Build a region entry showing `target` excerpt characters, or a
/// reference-only entry when `target` is zero.
fn region_for(
    candidate: &RegionCandidate,
    target: usize,
    full: &str,
    config: &ArmConfig,
    span_len: usize,
) -> EvidenceRegion {
    let (excerpt, truncated, excerpt_range) = if target == 0 {
        (None, true, None)
    } else {
        let (text, range) = take_region_excerpt(full, candidate.span, target);
        (Some(text), target < span_len, Some(range))
    };
    let (tool_call_id, tool_name, args_excerpt, source_truncated) =
        region_attribution(candidate, config.args_excerpt_cap);
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
        excerpt_range,
        uncovered_hits: 0,
    }
}

/// Allocate excerpts to ranked regions by their real serialized cost.
///
/// Each region in rank order takes the largest power-of-two share of its span
/// that fits the remaining `claim_evidence` budget, halving down to
/// [`ArmConfig::region_floor`] before degrading to a reference-only entry. No
/// region is ever dropped: a degraded region is still named by its claims.
fn allocate_regions(
    candidates: &[RegionCandidate],
    budget: usize,
    config: &ArmConfig,
    full: &str,
    claims: &[ClaimRef],
) -> Vec<EvidenceRegion> {
    // Cost of the object with no regions; each region then adds its own entry
    // plus one separator/bracket slot.
    let empty_cost = evidence_chars(&[], claims).saturating_sub(1);
    let mut region_cost = 0usize;
    let mut regions: Vec<EvidenceRegion> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let span_len = candidate.span.1 - candidate.span.0;
        let mut target = span_len;
        loop {
            let region = region_for(candidate, target, full, config, span_len);
            let entry = region_entry_chars(&region) + 1;
            if empty_cost + region_cost + entry <= budget {
                region_cost += entry;
                regions.push(region);
                break;
            }
            if target <= config.region_floor {
                let region = region_for(candidate, 0, full, config, span_len);
                region_cost += region_entry_chars(&region) + 1;
                regions.push(region);
                break;
            }
            target /= 2;
        }
    }
    regions
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
            state.claim_evidence.regions[idx].excerpt_range = None;
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
    use crate::claim::{ClaimClass, ExtractedClaim};
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

        // Make each region artificially large so they cannot all fit: spread the
        // hits inside one cluster (gaps below `region_cluster_gap`) so the
        // region span is wide, not just the enclosing tool result.
        let edge_filler = "x".repeat(4_000);
        let gap = "x".repeat(600);
        for cluster in 0..cluster_count {
            let mut call_result = vec![edge_filler.clone()];
            for i in 0..claims_per_cluster {
                let token = format!("task_D{cluster:02}{i:03}");
                call_result.push(format!("{token} 150ms pool-{cluster:02}"));
                if i + 1 < claims_per_cluster {
                    call_result.push(gap.clone());
                }
            }
            call_result.push(edge_filler.clone());
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
    fn region_entry_chars_matches_serde_serialization() {
        let region = EvidenceRegion {
            region_id: 3,
            tool_call_id: "tc-3".to_owned(),
            tool_name: "query".to_owned(),
            args_excerpt: "{\"pool\":\"db-pool-03\"}".to_owned(),
            excerpt: Some("task_DB03 150ms db-pool-03".to_owned()),
            span: (10, 50),
            full_region_chars: 40,
            source_truncated: false,
            truncated: true,
            excerpt_range: Some((10, 40)),
            uncovered_hits: 0,
        };
        let expected = serde_json::to_string(&region)
            .expect("serialize region")
            .chars()
            .count();
        assert_eq!(region_entry_chars(&region), expected);

        let mut longer = region.clone();
        longer.excerpt = Some("x".repeat(100));
        assert!(
            region_entry_chars(&longer) > expected,
            "a longer excerpt must cost more"
        );
    }

    /// db-pool-shaped: spread hits across ~12 regions whose real entry sizes
    /// (not an inflated per-region estimate) decide the allocation.
    fn db_pool_regions_edge() -> CapturedEdge {
        let cluster_count = 12;
        let claims_per_cluster = 2;

        let mut result_lines = Vec::new();
        let mut capture_calls = Vec::new();
        for cluster in 0..cluster_count {
            for i in 0..claims_per_cluster {
                let token = format!("task_R{cluster:02}{i:03}");
                result_lines.push(format!("Node {token} latency 150ms."));
            }
        }
        let result = result_lines.join("\n");

        let edge_filler = "z".repeat(1_000);
        let gap = "w".repeat(800);
        for cluster in 0..cluster_count {
            let mut call_result = vec![edge_filler.clone()];
            for i in 0..claims_per_cluster {
                let token = format!("task_R{cluster:02}{i:03}");
                call_result.push(format!("{token} 150ms db-pool-{cluster:02}"));
                if i + 1 < claims_per_cluster {
                    call_result.push(gap.clone());
                }
            }
            call_result.push(edge_filler.clone());
            capture_calls.push(ToolCall {
                tool_call_id: format!("tc{cluster}"),
                tool_name: "query".to_owned(),
                arguments: serde_json::json!({"pool": format!("db-pool-{cluster:02}")}),
                result: call_result.join("\n"),
                success: true,
            });
        }

        CapturedEdge {
            scenario: "db-pool-regions".to_owned(),
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
    fn db_pool_shaped_regions_keep_excerpts_and_reduce_uncovered() {
        let edge = db_pool_regions_edge();
        let mut config = ArmConfig::default();
        // Pin the allowance for the whole `claim_evidence` object to the tight
        // db-pool-shaped budget the allocation must pack against.
        config.state_budget_chars = build_fixed_state(&edge, &config).char_len() + 7_700;

        let state = assemble_claim_indexed(&edge, &config).unwrap();
        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
        assert!(
            state.claim_evidence.regions.len() >= 10,
            "expected ~12 regions, got {}",
            state.claim_evidence.regions.len()
        );
        let with_excerpt = state
            .claim_evidence
            .regions
            .iter()
            .filter(|r| r.excerpt.is_some())
            .count();
        assert!(
            with_excerpt >= 2,
            "real entry sizes must let several regions keep excerpts, got {with_excerpt}"
        );
        let uncovered: usize = state
            .claim_evidence
            .regions
            .iter()
            .map(|r| r.uncovered_hits)
            .sum();
        assert!(
            uncovered < 22,
            "uncovered hits must stay below the pre-fix 22, got {uncovered}"
        );
    }

    #[test]
    fn tiny_region_excerpt_budget_degrades_to_references() {
        let edge = degradation_edge();
        let config = ArmConfig {
            state_budget_chars: 6_500,
            region_excerpt_budget: 1_500,
            ..Default::default()
        };

        let state = assemble_claim_indexed(&edge, &config)
            .expect("a tighter region split budget must not hard-fail assembly");
        assert!(
            state.char_len() <= config.state_budget_chars,
            "state {} chars exceeds budget {}",
            state.char_len(),
            config.state_budget_chars
        );
        assert!(
            state.claim_evidence.regions.len() > 5,
            "the tighter split budget must yield more regions, got {}",
            state.claim_evidence.regions.len()
        );
        assert!(
            state
                .claim_evidence
                .regions
                .iter()
                .any(|r| r.excerpt.is_none()),
            "the tail must degrade to reference-only rather than hard-fail"
        );
        for claim in &state.claim_evidence.claims {
            if claim.found_in == FoundIn::ToolCapture {
                assert!(
                    claim.region_id.is_some(),
                    "a degraded region must still be named by its claim"
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
                excerpt_range: Some((0, 11)),
                uncovered_hits: 0,
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

    fn coverage_region(
        excerpt_range: Option<(usize, usize)>,
        excerpt_len: usize,
    ) -> EvidenceRegion {
        EvidenceRegion {
            region_id: 0,
            tool_call_id: "tc".to_owned(),
            tool_name: "query".to_owned(),
            args_excerpt: String::new(),
            excerpt: Some("x".repeat(excerpt_len)),
            span: (0, 200),
            full_region_chars: 200,
            source_truncated: false,
            truncated: true,
            excerpt_range,
            uncovered_hits: 0,
        }
    }

    fn tool_claim_ref() -> ClaimRef {
        ClaimRef {
            claim: "150ms".to_owned(),
            found_in: FoundIn::ToolCapture,
            region_id: Some(0),
        }
    }

    fn resolved_at(positions: Vec<usize>) -> Vec<ClaimResolution> {
        vec![ClaimResolution {
            claim: ExtractedClaim {
                text: "150ms".to_owned(),
                class: ClaimClass::ValueWithUnit,
            },
            found_in: FoundIn::ToolCapture,
            positions,
        }]
    }

    #[test]
    fn bounded_excerpt_covering_all_hits_is_budgeting_not_loss() {
        let mut evidence = ClaimEvidenceV2 {
            regions: vec![coverage_region(Some((90, 170)), 80)],
            claims: vec![tool_claim_ref()],
        };

        apply_region_coverage(&mut evidence, &resolved_at(vec![120]));

        assert_eq!(
            evidence.regions[0].uncovered_hits, 0,
            "the referenced claim's hit is inside the shown excerpt"
        );
        assert!(
            evidence.regions[0].is_excerpted(),
            "an 80-char excerpt of a 200-char region is excerpted (informational)"
        );
    }

    #[test]
    fn any_shown_occurrence_keeps_a_claim_recoverable() {
        let mut evidence = ClaimEvidenceV2 {
            regions: vec![coverage_region(Some((90, 170)), 80)],
            claims: vec![tool_claim_ref()],
        };

        // One occurrence is dropped, but another is shown, so the judge can
        // still recover the claim.
        apply_region_coverage(&mut evidence, &resolved_at(vec![50, 120]));

        assert_eq!(evidence.regions[0].uncovered_hits, 0);
    }

    #[test]
    fn hit_outside_region_excerpt_is_real_loss() {
        let mut evidence = ClaimEvidenceV2 {
            regions: vec![coverage_region(Some((90, 170)), 80)],
            claims: vec![tool_claim_ref()],
        };

        apply_region_coverage(&mut evidence, &resolved_at(vec![50]));

        assert_eq!(
            evidence.regions[0].uncovered_hits, 1,
            "the claim's only hit at 50 falls outside the shown excerpt"
        );
    }

    #[test]
    fn region_without_supported_claims_is_budgeting_only() {
        let mut evidence = ClaimEvidenceV2 {
            regions: vec![coverage_region(None, 0)],
            claims: vec![ClaimRef {
                claim: "150ms".to_owned(),
                found_in: FoundIn::NotFound,
                region_id: None,
            }],
        };
        // The only hit lies outside the region span, so the region has no
        // supported claims and losing its excerpt is not information loss.
        let resolved = resolved_at(vec![500]);

        apply_region_coverage(&mut evidence, &resolved);

        assert_eq!(evidence.regions[0].uncovered_hits, 0);
    }

    /// Hits every ~1.2k chars over ~20k. A single repeated claim answers to
    /// every hit, so the shown excerpt only needs to keep one occurrence for
    /// the claim to stay recoverable.
    fn chained_edge() -> CapturedEdge {
        let mut capture = String::new();
        for i in 0..17 {
            capture.push_str("150ms\n");
            if i < 16 {
                capture.push_str(&"y".repeat(1_150));
            }
        }
        CapturedEdge {
            scenario: "chained".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "analyst".to_owned(),
            goal: "diagnose".to_owned(),
            plan_step: PlanStep {
                task: "find latency".to_owned(),
                worker_role: "analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "chained".to_owned(),
                result: "150ms".to_owned(),
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

    /// The same ~1.2k-spaced hits, but every line carries a distinct claim, so
    /// each region owns its claims and a degraded region is real loss.
    fn distinct_chained_edge() -> CapturedEdge {
        let mut capture = String::new();
        let mut result = String::new();
        for i in 0..17 {
            let token = format!("task_CHN{i:05}");
            result.push_str(&token);
            result.push('\n');
            capture.push_str(&token);
            capture.push_str(" 150ms\n");
            if i < 16 {
                capture.push_str(&"y".repeat(1_120));
            }
        }
        CapturedEdge {
            scenario: "chained-distinct".to_owned(),
            iteration: 0,
            task_id: 0,
            worker_id: "analyst".to_owned(),
            goal: "diagnose".to_owned(),
            plan_step: PlanStep {
                task: "find latency".to_owned(),
                worker_role: "analyst".to_owned(),
                depends_on: Vec::new(),
            },
            plan_context: PlanContext {
                consumers: Vec::new(),
                siblings: Vec::new(),
            },
            context_provided: String::new(),
            worker_output: WorkerOutput {
                summary: "chained".to_owned(),
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

    /// A single-linkage chain over ~20k must be cut into small regions, each
    /// excerpt covering the hit(s) its claims rest on. The excerpts are still
    /// bounded (budgeting), just not lossy.
    #[test]
    fn chained_regions_split_until_excerpts_cover_their_hits() {
        let edge = chained_edge();
        let config = ArmConfig {
            state_budget_chars: 14_000,
            ..Default::default()
        };
        let state = assemble_claim_indexed(&edge, &config).unwrap();

        assert!(state.char_len() <= config.state_budget_chars);
        assert!(
            state.claim_evidence.regions.len() >= 5,
            "chained hits should split into 5+ regions, got {}",
            state.claim_evidence.regions.len()
        );

        let uncovered: usize = state
            .claim_evidence
            .regions
            .iter()
            .map(|r| r.uncovered_hits)
            .sum();
        assert_eq!(
            uncovered, 0,
            "every referenced claim must keep a shown hit: {:?}",
            state.claim_evidence.regions
        );
        assert!(
            state
                .claim_evidence
                .regions
                .iter()
                .any(|r| r.is_excerpted()),
            "bounded-but-covered regions must record as excerpted"
        );
    }

    /// When the budget cannot show every region, the degraded tail is real
    /// loss only because its claims' hits are uncovered.
    #[test]
    fn over_budget_tail_degradation_flags_loss_only_when_hits_uncovered() {
        let edge = distinct_chained_edge();
        let config = ArmConfig {
            state_budget_chars: 5_000,
            ..Default::default()
        };
        let state = assemble_claim_indexed(&edge, &config).unwrap();
        assert!(state.char_len() <= config.state_budget_chars);

        let degraded = state
            .claim_evidence
            .regions
            .iter()
            .filter(|r| r.excerpt.is_none() && r.truncated)
            .count();
        assert!(
            degraded > 0,
            "expected the tail to degrade: {:?}",
            state.claim_evidence.regions
        );

        let uncovered: usize = state
            .claim_evidence
            .regions
            .iter()
            .map(|r| r.uncovered_hits)
            .sum();
        assert!(
            uncovered > 0,
            "a degraded region with supported claims is real loss"
        );
    }
}
