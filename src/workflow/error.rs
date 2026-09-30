//! Rejections produced by the workflow plan types' parsing constructors
//! and validator.
//!
//! Variants name the rule that was violated; the caller still holds the
//! offending input, so no variant repeats it except where the message
//! needs the offending fragment to be actionable (a malformed path or
//! reference names itself, because the model must be able to find it in
//! its own proposal).

/// A value the workflow plan layer refused.
///
/// Every rejection below reaches the model as a tool observation it can
/// revise against in W2; nothing here is a run-ending error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowError {
    /// A step id must be a non-empty string; an empty one names no step.
    #[error("step id is empty")]
    EmptyStepId,
    /// An export name must be non-empty; an empty one cannot be bound to.
    #[error("export name is empty")]
    EmptyExportName,
    /// An export path is outside the `$.a.b[0]` subset: dotted keys and
    /// bracketed array indices only, leading `$.`, no wildcards, no
    /// arithmetic.
    #[error("result path '{0}' is not in the $.a.b[0] subset")]
    MalformedResultPath(String),
    /// A `$from` value is not a `step.export` name: exactly one dot,
    /// both halves non-empty.
    #[error("reference '{0}' is not a step.export name")]
    MalformedExportRef(String),
    /// Bounds exist to constrain a resolved value; a bound that sets
    /// neither `min` nor `max` constrains nothing and only mimics the
    /// reference-with-bounds wire shape.
    #[error("bounds must set min, max, or both")]
    EmptyBounds,
    /// An argument node that is neither a reference-free literal nor a
    /// well-formed reference: the recorded shape is the fragment that
    /// failed, so the model can find it in its own proposal. This is
    /// the fail-loud answer to a half-specified reference (a `$from`
    /// object carrying a stray `min` without a `max`), which silent
    /// deserialization would truncate into an unbounded reference.
    #[error("argument node is neither literal nor well-formed reference: {0}")]
    MalformedArgNode(String),
    /// Two steps share one id, so a dependency or reference names either
    /// ambiguously.
    #[error("two steps share the id '{id}'")]
    DuplicateStepId { id: String },
    /// A step depends on a step declared at the same position or later.
    /// Declaration order is load bearing: earlier-declared alone already
    /// forbids every cycle (a cycle needs a back-edge to a later or
    /// equal position), so this one rule carries the card's "acyclic and
    /// earlier-declared" pair.
    #[error("step '{step}' depends on '{dependency}', which is not declared earlier")]
    ForwardDependency { step: String, dependency: String },
    /// A step binds a reference that is not an export of a step in its
    /// dependencies-closure. A reference to a step the executor may not
    /// have run yet would resolve against a result that does not exist.
    #[error(
        "step '{step}' binds {from}, which is not an export of a step in its dependencies-closure"
    )]
    ReferenceOutsideClosure { step: String, from: String },
    /// A rollback binds a reference that is neither an earlier step's
    /// export nor the owning step's own. The owning step's own exports
    /// are legal exactly here: the step completed if its rollback runs.
    #[error(
        "rollback of step '{step}' binds {from}, which is neither an earlier export nor the step's own"
    )]
    RollbackReferenceOutsideOwner { step: String, from: String },
    /// A named tool is not in the discovered inventory. The available
    /// list lets the model revise against what the sidecar actually
    /// offers.
    #[error("no tool named '{tool}' is discovered; available: {available}")]
    UnknownTool { tool: String, available: String },
    /// A step's arguments fail the discovered tool's `inputSchema`, so
    /// the approver would be authorizing a schema-invalid instance
    /// (K3 finding 2).
    #[error("step '{step}' args do not satisfy the inputSchema of '{tool}': {message}")]
    ArgsFailSchema {
        step: String,
        tool: String,
        message: String,
    },
    /// A discovered tool's schema leans on a validation-relevant keyword
    /// the in-tree subset validator does not implement. Failing loud
    /// here is deliberate: silently passing an unvalidated schema would
    /// hand the approver unearned confidence, the worst outcome of the
    /// three.
    #[error("inputSchema of '{tool}' uses a keyword this validator does not implement: {keyword}")]
    UnsupportedSchemaKeyword { tool: String, keyword: String },
}
