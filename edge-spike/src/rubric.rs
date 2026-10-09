//! Sealed rubric v2 as jev-driver derive types.

use jev_driver::prelude::*;

use crate::state::EdgeState;

/// `evidence_presence` (Noul): gates an INSUFFICIENT_EVIDENCE verdict.
#[derive(Debug, Clone, Copy, JevNoul)]
#[jev(
    id = "evidence_presence",
    instructions = "Does `claim_evidence` plus `context_provided` contain the outputs the result's specific claims rely on?",
    yes = "The evidence needed to judge the worker output is present",
    no = "The evidence needed to judge the worker output is missing"
)]
pub struct EvidencePresence;

/// `task_fidelity` (Score, 5 levels): completeness and precision vs `plan_step.task`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JevScore)]
#[jev(
    id = "task_fidelity",
    instructions = "How completely and precisely does `worker_output` address `plan_step.task`?"
)]
pub enum TaskFidelity {
    #[jev(criteria = "Fully complete and precise")]
    Excellent,
    #[jev(criteria = "Mostly complete with minor omissions")]
    Good,
    #[jev(criteria = "Partially complete or somewhat imprecise")]
    Fair,
    #[jev(criteria = "Mostly incomplete or off-target")]
    Poor,
    #[jev(criteria = "Completely misses the task")]
    None,
}

/// `goal_alignment` (Score, 3 levels): advances `goal` vs distractor-aligned work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JevScore)]
#[jev(
    id = "goal_alignment",
    instructions = "Does `worker_output` advance `goal` or is it locally busy / distractor-aligned work?"
)]
pub enum GoalAlignment {
    #[jev(criteria = "Directly advances the goal")]
    Aligned,
    #[jev(criteria = "Tangentially related")]
    Neutral,
    #[jev(criteria = "Distractor-aligned or off-goal")]
    Misaligned,
}

/// `evidence_grounding` (Score, 4 levels): specific values backed by evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JevScore)]
#[jev(
    id = "evidence_grounding",
    instructions = "Are the specific values in `worker_output` backed by `claim_evidence` and `context_provided`?"
)]
pub enum EvidenceGrounding {
    #[jev(criteria = "Specific claims are supported by evidence")]
    Supported,
    #[jev(criteria = "Most claims are supported")]
    MostlySupported,
    #[jev(criteria = "Some claims supported, others not")]
    PartiallySupported,
    #[jev(criteria = "Unsupported or fabricated")]
    UnsupportedOrFabricated,
}

/// `handoff_integrity` (Noul): output carries exact values consumers need.
#[derive(Debug, Clone, Copy, JevNoul)]
#[jev(
    id = "handoff_integrity",
    instructions = "Does `worker_output` carry the exact values `plan_context.consumers` need to act without re-deriving them?",
    yes = "Consumer-needed values are present and exact",
    no = "Consumer-needed values are missing or would need re-derivation"
)]
pub struct HandoffIntegrity;

/// `scope_discipline` (Noul): stays inside `plan_step.task`.
#[derive(Debug, Clone, Copy, JevNoul)]
#[jev(
    id = "scope_discipline",
    instructions = "Does `worker_output` stay inside `plan_step.task` and avoid doing `plan_context.siblings` work?",
    yes = "Stays strictly within scope",
    no = "Performs sibling work or replays ancestors"
)]
pub struct ScopeDiscipline;

/// The full rubric: one request per edge, six questions over one state.
#[derive(Debug, Clone, JevQuestions)]
#[jev(state = EdgeState)]
pub struct EdgeRubric {
    pub evidence_presence: EvidencePresence,
    pub task_fidelity: TaskFidelity,
    pub goal_alignment: GoalAlignment,
    pub evidence_grounding: EvidenceGrounding,
    pub handoff_integrity: HandoffIntegrity,
    pub scope_discipline: ScopeDiscipline,
}
