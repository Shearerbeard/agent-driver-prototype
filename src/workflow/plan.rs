//! The workflow plan types: what a proposed workflow is, and the rules
//! a valid one satisfies before a human ever sees it.
//!
//! W1 is the types only. The wire shape is JSON (the model authors it
//! through the W2 tool's arguments), and approval binds the digest over
//! `serde_json::to_vec(&workflow)` — serde struct field order is
//! stable, so the field order below is part of the approval contract.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};

use crate::mcp_client::{SidecarTool, SidecarToolName};

use super::error::WorkflowError;

/// A proposed workflow: what it is for, and the steps that achieve it.
///
/// Business rule: a workflow is what a human approves in one act — the
/// goal names the intent the authorization covers, and the steps name
/// every tool call it covers. Nothing outside `steps` executes.
///
/// This is the wire shape. It is deliberately unvalidated on its own:
/// [`WorkflowSpec::validate`] against the discovered tool inventory is
/// the parse step, and it is also the *only* constructor of
/// [`ValidatedWorkflowSpec`] — the type the digest, the render, and the
/// executor accept. Nothing downstream takes a bare `WorkflowSpec`, so
/// an unvetted instance cannot be approved (panel finding 1, both
/// seats).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    /// What this workflow is for; rendered to the approver verbatim.
    pub goal: String,
    /// The steps, in declaration order. Declaration order carries the
    /// ordering rule: a step's dependencies must be declared earlier in
    /// this list (see [`ForwardDependency`]).
    ///
    /// [`ForwardDependency`]: WorkflowError::ForwardDependency
    pub steps: Vec<WorkflowStep>,
}

/// A workflow that passed [`WorkflowSpec::validate`].
///
/// Business rule: approval authorizes exactly what validation vetted
/// (K3 findings 1 and 2 both reduce to this). The wrapper is the type
/// witness: `validate` is its only constructor, so a digest, a render,
/// or an execution reaches only a spec whose ids, ordering, references,
/// tool names, and argument schemas were all checked.
///
/// Serializes identically to the [`WorkflowSpec`] it wraps (same bytes,
/// same field order), so the approval digest contract is unchanged.
///
/// Forbidden invalid state: construction from anything but `validate`.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedWorkflowSpec(WorkflowSpec);

impl ValidatedWorkflowSpec {
    /// The validated workflow's spec.
    pub fn spec(&self) -> &WorkflowSpec {
        &self.0
    }

    /// Consume the wrapper, yielding the validated spec.
    pub fn into_spec(self) -> WorkflowSpec {
        self.0
    }
}

impl Serialize for ValidatedWorkflowSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl WorkflowSpec {
    /// Validate this spec against the discovered tool inventory,
    /// yielding the capability type everything downstream requires.
    ///
    /// The rules, in the order they run, each failing with its own
    /// variant of [`WorkflowError`]:
    ///
    /// 0. the goal is non-empty and there is at least one step (panel
    ///    ruling on R3: an unnamed intent or a no-op authorization is
    ///    not a meaningful proposal, and the capability type must not
    ///    be able to wrap one);
    /// 1. step ids are unique;
    /// 2. every dependency names a step declared *earlier* — which
    ///    subsumes acyclicity, since a cycle needs a back-edge to a
    ///    later or equal position;
    /// 3. every `$from` reference in an argument tree names a declared
    ///    export of a step in that step's dependencies-closure; a
    ///    step's own rollback may additionally reference the owning
    ///    step's own exports;
    /// 4. every tool name — step tools and rollback tools — exists in
    ///    the discovered inventory;
    /// 5. every literal argument node satisfies the discovered tool's
    ///    `inputSchema` (K3 finding 2: the approver never authorizes a
    ///    schema-invalid instance). Step args and rollback args both:
    ///    reference nodes are structural here (they resolve at apply
    ///    time), literals are checked (panel finding: rollback literals
    ///    are as knowable at propose time as step literals).
    ///
    /// # Errors
    ///
    /// Returns the first rule the spec breaks. `validate` consumes the
    /// spec, so on rejection the caller re-parses from the wire JSON it
    /// still holds; in W2 every rejection reaches the model as a tool
    /// observation it can revise against.
    pub fn validate(self, tools: &[SidecarTool]) -> Result<ValidatedWorkflowSpec, WorkflowError> {
        self.validate_shape()?;
        self.validate_unique_ids()?;
        self.validate_dependencies()?;
        self.validate_references()?;
        self.validate_tool_inventory(tools)?;
        self.validate_argument_schemas(tools)?;
        Ok(ValidatedWorkflowSpec(self))
    }

    /// Rule 0: a nameable goal and at least one step.
    fn validate_shape(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 1: unique step ids.
    fn validate_unique_ids(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 2: every dependency is declared earlier than its step.
    fn validate_dependencies(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 3: references resolve inside the dependencies-closure, and
    /// a rollback may additionally reach its own step's exports.
    fn validate_references(&self) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 4: every named tool, step or rollback, is discovered.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn validate_tool_inventory(&self, tools: &[SidecarTool]) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 5: literal argument nodes — step args and rollback args —
    /// satisfy the named tool's inputSchema.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn validate_argument_schemas(&self, tools: &[SidecarTool]) -> Result<(), WorkflowError> {
        todo!()
    }
}

/// One step of a workflow: a tool call, its arguments, what it exports
/// of its result, and how to undo it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowStep {
    /// The step's identity, unique within its spec.
    pub id: StepId,
    /// The steps that must complete before this one, by id — the
    /// `Task.dependencies` vocabulary, string-keyed: workflow steps
    /// carry model-authored string ids where the DAG's tasks carry
    /// positional numeric ones, but the meaning is the same list of
    /// ids-of-steps-that-must-complete-first.
    #[serde(default)]
    pub dependencies: Vec<StepId>,
    /// The tool this step calls, as discovered from the sidecar's
    /// `tools/list` inventory.
    #[serde(with = "tool_name_serde")]
    pub tool: SidecarToolName,
    /// The argument tree. Literal nodes are checked against the tool's
    /// `inputSchema` at propose time; reference nodes (see
    /// [`ArgValue`]) are structural at propose time and resolve at
    /// apply time.
    pub args: Value,
    /// Named `$.`-paths over this step's JSON result. Later steps — and
    /// this step's own rollback — may bind to them by name.
    ///
    /// Deserialized through a duplicate-rejecting map reader: a wire
    /// object naming one export twice is a rejection, not a silent
    /// last-wins collapse (panel ruling on R1 — the raw-JSON ingress is
    /// the only place the duplicate is still visible, and no schema
    /// downstream of it can express the rule).
    #[serde(default, with = "exports_map_serde")]
    pub exports: BTreeMap<ExportName, ExportSpec>,
    /// The compensating call run on unwind, or `None` for a read-only
    /// step — rendered to the approver as an honest "nothing to undo",
    /// never invented.
    #[serde(default)]
    pub rollback: Option<RollbackSpec>,
}

/// A step's compensating call.
///
/// Business rule: a mutating step declares how to undo itself. The
/// rollback's `args` may bind `$from` references to exports of steps in
/// the owning step's dependencies-closure, and additionally to the
/// owning step's own exports — the owning step completed if its
/// rollback runs.
///
/// Forbidden invalid state: a mutating step with nothing to declare is
/// represented honestly as `rollback: null` on the step ([`None`]), so
/// no rollback spec exists that names no tool or undoes nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackSpec {
    /// The compensating tool, as discovered from the sidecar inventory.
    #[serde(with = "tool_name_serde")]
    pub tool: SidecarToolName,
    /// The rollback's argument tree; may bind `$from` references per
    /// the rule above.
    pub args: Value,
}

/// A workflow step's identity, unique within its spec.
///
/// Forbidden invalid states: an empty or whitespace-only id (the
/// `SidecarToolName` rule, mirrored), and an id containing `.`, which
/// the `step.export` reference form reserves as its separator — an id
/// carrying one could be declared but never referenced unambiguously
/// (panel finding: the component and reference grammars must agree).
/// [`StepId::parse`] rejects both.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StepId(String);

impl StepId {
    /// Parse a step id from its wire string.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyStepId`] for an empty or
    /// whitespace-only id, and [`WorkflowError::MalformedStepId`] for
    /// one containing the reference separator `.`.
    pub fn parse(id: String) -> Result<Self, WorkflowError> {
        if id.trim().is_empty() {
            return Err(WorkflowError::EmptyStepId);
        }
        if id.contains('.') {
            return Err(WorkflowError::MalformedStepId(id));
        }
        Ok(Self(id))
    }

    /// The id as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StepId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for StepId {
    type Error = WorkflowError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::parse(id)
    }
}

impl From<StepId> for String {
    fn from(id: StepId) -> Self {
        id.0
    }
}

/// An export's name within its step.
///
/// Forbidden invalid states: an empty or whitespace-only name (nothing
/// could bind to it), and a name containing `.` — the same separator
/// reservation as [`StepId`], so the `step.export` reference grammar
/// stays unambiguous. [`ExportName::parse`] rejects both.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExportName(String);

impl ExportName {
    /// Parse an export name from its wire string.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyExportName`] for an empty or
    /// whitespace-only name, and [`WorkflowError::MalformedExportName`]
    /// for one containing the reference separator `.`.
    pub fn parse(name: String) -> Result<Self, WorkflowError> {
        if name.trim().is_empty() {
            return Err(WorkflowError::EmptyExportName);
        }
        if name.contains('.') {
            return Err(WorkflowError::MalformedExportName(name));
        }
        Ok(Self(name))
    }

    /// The name as it appears on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExportName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ExportName {
    type Error = WorkflowError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::parse(name)
    }
}

impl From<ExportName> for String {
    fn from(name: ExportName) -> Self {
        name.0
    }
}

/// A named `$.`-path over a step's JSON result: the subset
/// `$.a.b[0]` — dotted keys and bracketed array indices, nothing else.
/// No wildcards, no arithmetic, no filters.
///
/// Forbidden invalid state: a path this vocabulary cannot express, which
/// either names nothing or means something the executor never agreed
/// to resolve; [`ExportSpec::parse`] rejects it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExportSpec {
    segments: Vec<PathSegment>,
}

/// One segment of an [`ExportSpec`] path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[expect(
    dead_code,
    reason = "W1 Layer 1: constructed when ExportSpec::parse is filled"
)]
enum PathSegment {
    /// An object key.
    Key(String),
    /// An array index.
    Index(usize),
}

impl ExportSpec {
    /// Parse a `$.a.b[0]` result path.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::MalformedResultPath`] for anything
    /// outside the subset: a missing leading `$.`, an empty key, an
    /// empty or non-numeric index, a stray `[` or `]`, a trailing dot.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    pub fn parse(path: &str) -> Result<Self, WorkflowError> {
        todo!()
    }

    /// The path's serde_json pointer form: `$.a.b[0]` becomes `/a/b/0`.
    ///
    /// This is the conversion the W3 executor uses to lift a declared
    /// export out of a step's JSON result via
    /// `serde_json::Pointer`. Key segments are not escaped (the subset
    /// excludes `/`, `~`, and `.`, so a key segment needs no JSON-pointer
    /// escaping); an index segment renders as its decimal form.
    pub fn to_json_pointer(&self) -> String {
        todo!()
    }

    /// The path in its declared `$.a.b[0]` form.
    pub fn as_path(&self) -> String {
        todo!()
    }
}

impl fmt::Display for ExportSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_path())
    }
}

impl Serialize for ExportSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_path())
    }
}

impl<'de> Deserialize<'de> for ExportSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let path = String::deserialize(deserializer)?;
        Self::parse(&path).map_err(D::Error::custom)
    }
}

/// A `$from` reference: a named export of an earlier step, written
/// `step.export`.
///
/// Forbidden invalid state: a reference that names no step or no export
/// half (an empty side, or more than one dot); [`ExportRef::parse`]
/// rejects it. Whether the named step and export are actually declared
/// is a spec-level rule, checked by
/// [`WorkflowSpec::validate`](WorkflowSpec::validate), because only the
/// whole spec knows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExportRef {
    step: StepId,
    export: ExportName,
}

impl ExportRef {
    /// Assemble a reference from its halves.
    pub fn new(step: StepId, export: ExportName) -> Self {
        Self { step, export }
    }

    /// Parse a `step.export` reference string.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::MalformedExportRef`] unless the string
    /// is exactly two non-empty halves around exactly one dot.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    pub fn parse(text: &str) -> Result<Self, WorkflowError> {
        todo!()
    }

    /// The referenced step's id.
    pub fn step(&self) -> &StepId {
        &self.step
    }

    /// The referenced export's name.
    pub fn export(&self) -> &ExportName {
        &self.export
    }
}

impl fmt::Display for ExportRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.step, self.export)
    }
}

impl Serialize for ExportRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ExportRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(D::Error::custom)
    }
}

/// Resolve-time numeric bounds riding on a reference.
///
/// Business rule: the model declares the envelope, the executor checks
/// the resolved value against it at resolve time — the model never
/// supplies or verifies bound values (AURA argument-model rule). A
/// violation on step N's resolve is a step-N failure and unwinds like
/// any other.
///
/// One-sided bounds are legal (panel ruling, R2): a `min`-only or
/// `max`-only envelope is a real constraint. Forbidden invalid state: a
/// bound that constrains nothing (neither `min` nor `max`), which only
/// mimics the bounded-reference wire shape; [`Bounds::new`] rejects it,
/// and construction is gated exactly like [`StepId`]: private fields,
/// and serde routed through `new`, so no path bypasses the rejection
/// (panel finding 2, both seats).
#[derive(Clone, Debug, PartialEq)]
pub struct Bounds {
    min: Option<Number>,
    max: Option<Number>,
}

impl Bounds {
    /// Declare a bound.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyBounds`] when both halves are
    /// `None`.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    pub fn new(min: Option<Number>, max: Option<Number>) -> Result<Self, WorkflowError> {
        todo!()
    }

    /// The inclusive lower bound, when declared.
    pub fn min(&self) -> Option<&Number> {
        self.min.as_ref()
    }

    /// The inclusive upper bound, when declared.
    pub fn max(&self) -> Option<&Number> {
        self.max.as_ref()
    }
}

impl Serialize for Bounds {
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!()
    }
}

impl<'de> Deserialize<'de> for Bounds {
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        todo!()
    }
}

/// One classified node of a step's argument tree.
///
/// The wire contract (custom serde, filled in Layer 2 and pinned by its
/// tests):
///
/// - `{"$from": "step.export"}` is [`ArgValue::Reference`];
/// - `{"$from": "step.export", "min": 1}`, `{"$from": "step.export",
///   "max": 20}`, or both together are [`ArgValue::BoundedReference`] —
///   one-sided bounds are legal (panel ruling, R2);
/// - anything else is [`ArgValue::Literal`].
///
/// References bind inline anywhere in the argument tree (panel ruling
/// on R5): a reference object is recognized as the value of any object
/// key and as any array element, at any depth, and the classification
/// walk visits every such position — so a literal is reference-free by
/// construction, not by trust, and a malformed `$from` shape at any
/// depth is a rejection rather than a silently swallowed node.
///
/// This classifies nodes; the argument *tree* stays a raw [`Value`]
/// end-to-end, because both consumers need the wire shape — the
/// inputSchema check validates the literal structure, and the W3
/// resolver substitutes references in place. Validation walks the tree
/// with [`ArgValue::parse`] to find and check every reference node.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgValue {
    /// A reference-free JSON node.
    Literal(Value),
    /// A reference to an export of a step in the dependencies-closure.
    Reference { from: ExportRef },
    /// A reference with resolve-time numeric bounds; either side may be
    /// declared alone, never both absent.
    BoundedReference { from: ExportRef, bounds: Bounds },
}

impl ArgValue {
    /// Classify one argument node against the wire contract above.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::MalformedArgNode`] for the shapes that
    /// are neither literal nor well-formed reference: a `$from` whose
    /// value is not a `step.export` string, keys beside
    /// `$from`/`min`/`max`, or `min`/`max` values that are not numbers.
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    pub fn parse(node: &Value) -> Result<Self, WorkflowError> {
        todo!()
    }
}

impl Serialize for ArgValue {
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        todo!()
    }
}

impl<'de> Deserialize<'de> for ArgValue {
    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        todo!()
    }
}

/// Serde bridge for [`SidecarToolName`], which carries no serde derives
/// of its own.
///
/// The non-empty rule stays where it already lives
/// (`SidecarToolName::new`); this module only carries the string across
/// the serde boundary, so no second authority on tool-name validity can
/// drift from the first.
mod tool_name_serde {
    use super::*;

    pub fn serialize<S: Serializer>(
        name: &SidecarToolName,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(name.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SidecarToolName, D::Error> {
        let text = String::deserialize(deserializer)?;
        SidecarToolName::new(&text).map_err(D::Error::custom)
    }
}

/// Duplicate-rejecting serde for a step's `exports` map (panel ruling
/// on R1: the raw-JSON ingress is the only place a duplicate export
/// name is still visible; a plain `BTreeMap` deserialization collapses
/// it last-wins, and no schema downstream of the parse can express key
/// uniqueness).
///
/// Serializes as the plain map (the validated form cannot hold a
/// duplicate, so serialization needs no guard); deserialization reads
/// the raw map entries and rejects a repeated name with
/// [`WorkflowError::DuplicateExportName`] before any collapse can
/// happen.
mod exports_map_serde {
    use super::*;

    pub fn serialize<S: Serializer>(
        exports: &BTreeMap<ExportName, ExportSpec>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        exports.serialize(serializer)
    }

    #[expect(unused_variables, reason = "W1 Layer 1: used when the fill lands")]
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<ExportName, ExportSpec>, D::Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- StepId / ExportName ----

    #[test]
    fn step_id_accepts_a_plain_name() {
        let id = StepId::parse("state".into()).unwrap();
        assert_eq!(id.as_str(), "state");
        assert_eq!(id.to_string(), "state");
    }

    #[test]
    fn step_id_rejects_empty_and_whitespace_only() {
        assert_eq!(
            StepId::parse(String::new()),
            Err(WorkflowError::EmptyStepId)
        );
        assert_eq!(StepId::parse("   ".into()), Err(WorkflowError::EmptyStepId));
    }

    #[test]
    fn step_id_rejects_the_reference_separator() {
        assert_eq!(
            StepId::parse("a.b".into()),
            Err(WorkflowError::MalformedStepId("a.b".into()))
        );
    }

    #[test]
    fn step_id_serde_rejects_what_parse_rejects() {
        let bad: Result<StepId, _> = serde_json::from_str("\"\"");
        assert!(bad.is_err());
        let ok: StepId = serde_json::from_str("\"scale\"").unwrap();
        assert_eq!(ok.as_str(), "scale");
        assert_eq!(serde_json::to_string(&ok).unwrap(), "\"scale\"");
    }

    #[test]
    fn export_name_rejects_empty_whitespace_and_separator() {
        assert_eq!(
            ExportName::parse(String::new()),
            Err(WorkflowError::EmptyExportName)
        );
        assert_eq!(
            ExportName::parse(" ".into()),
            Err(WorkflowError::EmptyExportName)
        );
        assert_eq!(
            ExportName::parse("a.b".into()),
            Err(WorkflowError::MalformedExportName("a.b".into()))
        );
    }

    // ---- ExportSpec (the $.-path subset) ----

    #[test]
    fn result_path_parses_keys_and_indices() {
        let path = ExportSpec::parse("$.deployment.replicas").unwrap();
        assert_eq!(path.to_json_pointer(), "/deployment/replicas");
        let path = ExportSpec::parse("$.a.b[0]").unwrap();
        assert_eq!(path.to_json_pointer(), "/a/b/0");
    }

    #[test]
    fn result_path_round_trips_through_its_declared_form() {
        for declared in ["$.a", "$.a.b[0]", "$.a[0][1].b[2].c"] {
            let path = ExportSpec::parse(declared).unwrap();
            assert_eq!(path.as_path(), declared);
            assert_eq!(&path.to_string(), declared);
        }
    }

    #[test]
    fn result_path_lifts_the_declared_value_from_a_json_result() {
        let path = ExportSpec::parse("$.deployment.replicas").unwrap();
        let result = json!({"deployment": {"replicas": 3}});
        let lifted = result.pointer(&path.to_json_pointer()).unwrap();
        assert_eq!(lifted, &json!(3));
    }

    #[test]
    fn result_path_rejects_shapes_outside_the_subset() {
        for bad in [
            "a.b",         // no leading $.
            "$",           // no segments
            "$.",          // empty key
            "$.a.",        // trailing dot
            "$.a..b",      // empty interior key
            "$.a[b]",      // non-numeric index
            "$.a[-1]",     // negative index
            "$.a[0",       // unclosed bracket
            "$.a[0][",     // unclosed second bracket
            "$.a]b[",      // stray bracket
            "$.a.b[0]c",   // key glued to a closed bracket
            "$.*",         // wildcard
            "$.a[+0]",     // arithmetic
            "$.a.b[0]..x", // empty key after index
        ] {
            assert!(ExportSpec::parse(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn result_path_rejects_pointer_escape_and_separator_characters_in_keys() {
        for bad in ["$.a/b", "$.a~b", "$.a.b[0].c[d]"] {
            assert!(ExportSpec::parse(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn result_path_serde_round_trips() {
        let path: ExportSpec = serde_json::from_str("\"$.a.b[0]\"").unwrap();
        assert_eq!(serde_json::to_string(&path).unwrap(), "\"$.a.b[0]\"");
        let bad: Result<ExportSpec, _> = serde_json::from_str("\"a.b\"");
        assert!(bad.is_err());
    }

    // ---- ExportRef ----

    #[test]
    fn export_ref_parses_exactly_one_dot() {
        let r = ExportRef::parse("state.current_replicas").unwrap();
        assert_eq!(r.step().as_str(), "state");
        assert_eq!(r.export().as_str(), "current_replicas");
        assert_eq!(r.to_string(), "state.current_replicas");
    }

    #[test]
    fn export_ref_rejects_zero_two_or_empty_halves() {
        for bad in ["state", "a.b.c", ".b", "a.", ""] {
            assert!(ExportRef::parse(bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn export_ref_serde_round_trips() {
        let r: ExportRef = serde_json::from_str("\"state.x\"").unwrap();
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"state.x\"");
        let bad: Result<ExportRef, _> = serde_json::from_str("\"state\"");
        assert!(bad.is_err());
    }

    // ---- Bounds ----

    #[test]
    fn bounds_require_at_least_one_side() {
        assert_eq!(Bounds::new(None, None), Err(WorkflowError::EmptyBounds));
        assert!(Bounds::new(Some(json!(1).as_number().unwrap().clone()), None).is_ok());
        assert!(Bounds::new(None, Some(json!(20).as_number().unwrap().clone())).is_ok());
    }

    #[test]
    fn bounds_accessors_expose_the_declared_sides() {
        let min = json!(1).as_number().unwrap().clone();
        let max = json!(20).as_number().unwrap().clone();
        let b = Bounds::new(Some(min.clone()), Some(max.clone())).unwrap();
        assert_eq!(b.min(), Some(&min));
        assert_eq!(b.max(), Some(&max));
    }

    #[test]
    fn bounds_serde_routes_through_new() {
        let one_sided: Result<Bounds, _> = serde_json::from_str("{\"min\":1}");
        assert!(one_sided.is_ok());
        let empty: Result<Bounds, _> = serde_json::from_str("{}");
        assert!(empty.is_err());
    }

    // ---- ArgValue wire contract ----

    #[test]
    fn arg_nodes_classify_reference_bounded_reference_and_literal() {
        let r = ArgValue::parse(&json!({"$from": "state.x"})).unwrap();
        assert_eq!(
            r,
            ArgValue::Reference {
                from: ExportRef::parse("state.x").unwrap()
            }
        );
        let b = ArgValue::parse(&json!({"$from": "state.x", "min": 1, "max": 20})).unwrap();
        assert!(matches!(b, ArgValue::BoundedReference { .. }));
        let l = ArgValue::parse(&json!("payments")).unwrap();
        assert_eq!(l, ArgValue::Literal(json!("payments")));
    }

    #[test]
    fn one_sided_bounds_are_well_formed_references() {
        for node in [
            json!({"$from": "state.x", "min": 1}),
            json!({"$from": "state.x", "max": 20}),
        ] {
            assert!(
                matches!(
                    ArgValue::parse(&node).unwrap(),
                    ArgValue::BoundedReference { .. }
                ),
                "rejected one-sided bounds in {node}"
            );
        }
    }

    #[test]
    fn malformed_reference_shapes_are_rejected_not_truncated() {
        for node in [
            json!({"$from": "state"}),                  // not step.export
            json!({"$from": 3}),                        // not a string
            json!({"$from": "state.x", "extra": true}), // stray key
            json!({"$from": "state.x", "min": "high"}), // non-numeric bound
            json!({"min": 1, "max": 2}),                // no $from at all
        ] {
            assert!(
                ArgValue::parse(&node).is_err(),
                "accepted malformed reference {node}"
            );
        }
    }

    #[test]
    fn arg_value_serde_round_trips_all_three_cases() {
        for node in [
            json!({"$from": "state.x"}),
            json!({"$from": "state.x", "min": 1}),
            json!({"$from": "state.x", "min": 1, "max": 20}),
            json!("literal"),
            json!({"plain": [1, 2]}),
        ] {
            let parsed = ArgValue::parse(&node).unwrap();
            let wire = serde_json::to_value(&parsed).unwrap();
            let back: ArgValue = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(back, parsed, "round-trip failed through {wire}");
        }
    }

    // ---- WorkflowSpec wire shape ----

    fn demo_wire() -> serde_json::Value {
        json!({
            "goal": "Mitigate the payments db-primary connection pool exhaustion",
            "steps": [
                {
                    "id": "state",
                    "dependencies": [],
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "payments"},
                    "exports": {"current_replicas": "$.deployment.replicas"},
                    "rollback": null
                },
                {
                    "id": "scale",
                    "dependencies": ["state"],
                    "tool": "ops_scale_app",
                    "args": {"app": "payments", "replicas": 6},
                    "exports": {},
                    "rollback": {
                        "tool": "ops_scale_app",
                        "args": {"app": "payments",
                                 "replicas": {"$from": "state.current_replicas", "min": 1, "max": 20}}
                    }
                }
            ]
        })
    }

    #[test]
    fn the_demo_wire_shape_parses_into_the_typed_spec() {
        let spec: WorkflowSpec = serde_json::from_value(demo_wire()).unwrap();
        assert_eq!(spec.steps.len(), 2);
        assert_eq!(spec.steps[0].id.as_str(), "state");
        assert!(spec.steps[0].rollback.is_none());
        let rollback = spec.steps[1].rollback.as_ref().unwrap();
        assert_eq!(rollback.tool.as_str(), "ops_scale_app");
        assert_eq!(
            spec.steps[1].exports.keys().next().map(|k| k.as_str()),
            None
        );
        let _ = spec.steps[0]
            .exports
            .get(&ExportName::parse("current_replicas".into()).unwrap())
            .unwrap();
    }

    #[test]
    fn unknown_fields_on_the_wire_structs_are_rejected() {
        let mut extra_top = demo_wire();
        extra_top["priority"] = json!(1);
        assert!(serde_json::from_value::<WorkflowSpec>(extra_top).is_err());

        let mut extra_step = demo_wire();
        extra_step["steps"][0]["urgency"] = json!("high");
        assert!(serde_json::from_value::<WorkflowSpec>(extra_step).is_err());

        let mut extra_rollback = demo_wire();
        extra_rollback["steps"][1]["rollback"]["reason"] = json!("why");
        assert!(serde_json::from_value::<WorkflowSpec>(extra_rollback).is_err());
    }

    #[test]
    fn a_duplicate_export_name_is_rejected_at_the_ingress() {
        let raw = r#"{
            "goal": "g",
            "steps": [{
                "id": "s",
                "tool": "t",
                "args": {},
                "exports": {"x": "$.a", "x": "$.b"}
            }]
        }"#;
        let parsed: Result<WorkflowSpec, _> = serde_json::from_str(raw);
        let err = parsed.err().expect("duplicate export name accepted");
        assert!(
            err.to_string().contains("declared twice"),
            "wrong rejection: {err}"
        );
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use crate::mcp_client::{SidecarTool, SidecarToolName};
    use serde_json::json;

    fn tool(name: &str, schema: serde_json::Value) -> SidecarTool {
        SidecarTool::new(SidecarToolName::new(name).unwrap(), String::new(), schema)
    }

    fn scale_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "app": {"type": "string"},
                "replicas": {"type": "integer"}
            },
            "required": ["app", "replicas"]
        })
    }

    fn read_schema() -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"app": {"type": "string"}},
            "required": ["app"]
        })
    }

    fn inventory() -> Vec<SidecarTool> {
        vec![
            tool("ops_get_cluster_state", read_schema()),
            tool("ops_scale_app", scale_schema()),
        ]
    }

    fn spec(wire: serde_json::Value) -> WorkflowSpec {
        serde_json::from_value(wire).unwrap()
    }

    fn step(id: &str, tool: &str, args: serde_json::Value) -> serde_json::Value {
        json!({
            "id": id,
            "tool": tool,
            "args": args,
            "exports": {},
            "rollback": null
        })
    }

    fn valid_two_step() -> serde_json::Value {
        json!({
            "goal": "mitigate",
            "steps": [
                {
                    "id": "state",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "payments"},
                    "exports": {"replicas": "$.deployment.replicas"},
                    "rollback": null
                },
                {
                    "id": "scale",
                    "dependencies": ["state"],
                    "tool": "ops_scale_app",
                    "args": {"app": "payments",
                             "replicas": {"$from": "state.replicas", "min": 1, "max": 20}},
                    "exports": {},
                    "rollback": {
                        "tool": "ops_scale_app",
                        "args": {"app": "payments",
                                 "replicas": {"$from": "state.replicas", "min": 1}}
                    }
                }
            ]
        })
    }

    #[test]
    fn the_valid_two_step_workflow_passes_and_yields_the_capability() {
        let validated = spec(valid_two_step()).validate(&inventory()).unwrap();
        assert_eq!(validated.spec().goal, "mitigate");
        let inner = validated.into_spec();
        assert_eq!(inner.steps.len(), 2);
    }

    #[test]
    fn the_validated_wrapper_serializes_identically_to_the_spec() {
        let wire = valid_two_step();
        let spec = serde_json::from_value::<WorkflowSpec>(wire.clone()).unwrap();
        let validated = spec.clone().validate(&inventory()).unwrap();
        assert_eq!(
            serde_json::to_value(&validated).unwrap(),
            serde_json::to_value(&spec).unwrap()
        );
    }

    // ---- rule 0: shape ----

    #[test]
    fn an_empty_goal_is_rejected() {
        let mut wire = valid_two_step();
        wire["goal"] = json!("  ");
        assert_eq!(
            spec(wire).validate(&inventory()),
            Err(WorkflowError::EmptyGoal)
        );
    }

    #[test]
    fn a_workflow_with_no_steps_is_rejected() {
        let wire = json!({"goal": "g", "steps": []});
        assert_eq!(
            spec(wire).validate(&inventory()),
            Err(WorkflowError::EmptySteps)
        );
    }

    // ---- rule 1: unique ids ----

    #[test]
    fn duplicate_step_ids_are_rejected() {
        let wire = json!({
            "goal": "g",
            "steps": [
                step("a", "ops_get_cluster_state", json!({"app": "x"})),
                step("a", "ops_get_cluster_state", json!({"app": "x"}))
            ]
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::DuplicateStepId { .. }),
            "{err}"
        );
    }

    // ---- rule 2: dependencies earlier-declared ----

    #[test]
    fn a_forward_dependency_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [
                step("a", "ops_get_cluster_state", json!({"app": "x"})),
                step("b", "ops_get_cluster_state", json!({"app": "x"}))
            ]
        });
        wire["steps"][0]["dependencies"] = json!(["b"]); // b is declared later
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::ForwardDependency { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_self_dependency_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_get_cluster_state", json!({"app": "x"}))]
        });
        wire["steps"][0]["dependencies"] = json!(["a"]);
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::ForwardDependency { .. }),
            "{err}"
        );
    }

    #[test]
    fn an_unknown_dependency_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_get_cluster_state", json!({"app": "x"}))]
        });
        wire["steps"][0]["dependencies"] = json!(["ghost"]);
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::ForwardDependency { .. }),
            "{err}"
        );
    }

    // ---- rule 3: references in the closure ----

    #[test]
    fn a_reference_outside_the_closure_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [
                {
                    "id": "solo",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "x"},
                    "exports": {"out": "$.deployment.replicas"},
                    "rollback": null
                },
                {
                    "id": "other",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "x"},
                    "exports": {},
                    "rollback": null
                },
                {
                    "id": "late",
                    "dependencies": ["other"],
                    "tool": "ops_get_cluster_state",
                    "args": {"app": {"$from": "solo.out"}},
                    "exports": {},
                    "rollback": null
                }
            ]
        });
        let _ = &mut wire;
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::ReferenceOutsideClosure { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_reference_to_an_undeclared_export_is_rejected() {
        let mut wire = valid_two_step();
        wire["steps"][1]["args"] = json!({"app": {"$from": "state.missing"}, "replicas": 6});
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::ReferenceOutsideClosure { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_rollback_may_reference_the_owning_step_exports() {
        let mut wire = valid_two_step();
        wire["steps"][1]["exports"] = json!({"chosen": "$.deployment.replicas"});
        wire["steps"][1]["rollback"] = json!({
            "tool": "ops_scale_app",
            "args": {"app": "payments", "replicas": {"$from": "scale.chosen"}}
        });
        assert!(spec(wire).validate(&inventory()).is_ok());
    }

    #[test]
    fn a_rollback_reference_outside_closure_and_own_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [
                {
                    "id": "a",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "x"},
                    "exports": {"out": "$.deployment.replicas"},
                    "rollback": null
                },
                {
                    "id": "b",
                    "tool": "ops_get_cluster_state",
                    "args": {"app": "x"},
                    "exports": {},
                    "rollback": null
                },
                {
                    "id": "c",
                    "dependencies": ["a"],
                    "tool": "ops_scale_app",
                    "args": {"app": "x", "replicas": 2},
                    "exports": {},
                    "rollback": {
                        "tool": "ops_scale_app",
                        "args": {"app": "x", "replicas": {"$from": "b.out"}}
                    }
                }
            ]
        });
        let _ = &mut wire;
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(
            matches!(err, WorkflowError::RollbackReferenceOutsideOwner { .. }),
            "{err}"
        );
    }

    #[test]
    fn references_bind_anywhere_in_the_argument_tree() {
        let mut wire = valid_two_step();
        wire["steps"][1]["args"] = json!({
            "app": "payments",
            "replicas": {"$from": "state.replicas", "min": 1, "max": 20},
            "labels": {"source": {"$from": "state.replicas"}, "extras": [1, {"$from": "state.replicas"}]}
        });
        let err = spec(wire.clone()).validate(&inventory()).unwrap_err();
        // The nested object/array positions are outside the scale schema's
        // declared properties, so this specific wire fails rule 5, not
        // rule 3: the references themselves must be accepted. Use a
        // permissive schema to prove the depth reading alone.
        assert!(
            matches!(err, WorkflowError::ArgsFailSchema { .. }),
            "depth reading regressed to a rule-3 rejection: {err}"
        );

        let permissive = vec![
            tool("ops_get_cluster_state", read_schema()),
            tool("ops_scale_app", json!({"type": "object"})),
        ];
        assert!(
            spec(wire).validate(&permissive).is_ok(),
            "nested or array-element references were rejected"
        );
    }

    // ---- rule 4: inventory ----

    #[test]
    fn an_undiscovered_step_tool_is_rejected_with_the_available_list() {
        let wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_no_such_tool", json!({}))]
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        match err {
            WorkflowError::UnknownTool { tool, available } => {
                assert_eq!(tool, "ops_no_such_tool");
                assert!(available.contains("ops_scale_app"), "{available}");
            }
            other => panic!("wrong rejection: {other}"),
        }
    }

    #[test]
    fn an_undiscovered_rollback_tool_is_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_get_cluster_state", json!({"app": "x"}))]
        });
        wire["steps"][0]["rollback"] = json!({
            "tool": "ops_no_such_tool",
            "args": {}
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(matches!(err, WorkflowError::UnknownTool { .. }), "{err}");
    }

    // ---- rule 5: argument schemas ----

    #[test]
    fn schema_invalid_step_args_are_rejected() {
        let wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_scale_app", json!({"app": "payments", "replicas": "six"}))]
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        match err {
            WorkflowError::ArgsFailSchema { step, tool, .. } => {
                assert_eq!(step, "a");
                assert_eq!(tool, "ops_scale_app");
            }
            other => panic!("wrong rejection: {other}"),
        }
    }

    #[test]
    fn a_missing_required_property_is_rejected() {
        let wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_scale_app", json!({"app": "payments"}))]
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(matches!(err, WorkflowError::ArgsFailSchema { .. }), "{err}");
    }

    #[test]
    fn reference_nodes_are_structural_not_type_checked_at_propose_time() {
        let mut wire = valid_two_step();
        // The reference occupies `replicas`, where the schema wants an
        // integer; propose time accepts it, resolve time (W3) checks it.
        assert!(spec(wire.clone()).validate(&inventory()).is_ok());
        wire["steps"][1]["args"]["replicas"] = json!({"$from": "state.replicas"});
        assert!(spec(wire).validate(&inventory()).is_ok());
    }

    #[test]
    fn schema_invalid_rollback_literal_args_are_rejected() {
        let mut wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_get_cluster_state", json!({"app": "x"}))]
        });
        wire["steps"][0]["rollback"] = json!({
            "tool": "ops_scale_app",
            "args": {"app": "payments", "replicas": "six"}
        });
        let err = spec(wire).validate(&inventory()).unwrap_err();
        assert!(matches!(err, WorkflowError::ArgsFailSchema { .. }), "{err}");
    }

    #[test]
    fn a_schema_keyword_outside_the_subset_fails_loud() {
        let fancy = vec![
            tool("ops_get_cluster_state", read_schema()),
            tool(
                "ops_scale_app",
                json!({
                    "type": "object",
                    "properties": {"app": {"type": "string", "pattern": "^pay"}},
                }),
            ),
        ];
        let wire = json!({
            "goal": "g",
            "steps": [step("a", "ops_scale_app", json!({"app": "payments"}))]
        });
        let err = spec(wire).validate(&fancy).unwrap_err();
        match err {
            WorkflowError::UnsupportedSchemaKeyword { tool, keyword } => {
                assert_eq!(tool, "ops_scale_app");
                assert_eq!(keyword, "pattern");
            }
            other => panic!("wrong rejection: {other}"),
        }
    }
}
