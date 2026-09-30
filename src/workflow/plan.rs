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
/// This is the wire shape as well as the validated form's container:
/// [`WorkflowSpec::validate`] against the discovered tool inventory is
/// the parse step, and nothing downstream accepts a spec that has not
/// passed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    /// What this workflow is for; rendered to the approver verbatim.
    pub goal: String,
    /// The steps, in declaration order. Declaration order is load
    /// bearing: a step's dependencies must be declared earlier in this
    /// list (see [`ForwardDependency`]).
    ///
    /// [`ForwardDependency`]: super::WorkflowError::ForwardDependency
    pub steps: Vec<WorkflowStep>,
}

impl WorkflowSpec {
    /// Validate this spec against the discovered tool inventory.
    ///
    /// The rules, in the order they run, each failing with its own
    /// variant of [`WorkflowError`]:
    ///
    /// 1. step ids are unique;
    /// 2. every dependency names a step declared *earlier* — which
    ///    subsumes acyclicity, since a cycle needs a back-edge to a
    ///    later or equal position;
    /// 3. every `$from` reference in step args names a declared export
    ///    of a step in that step's dependencies-closure; a step's own
    ///    rollback may additionally reference the owning step's own
    ///    exports;
    /// 4. every tool name — step tools and rollback tools — exists in
    ///    the discovered inventory;
    /// 5. every step's literal arguments satisfy the discovered tool's
    ///    `inputSchema` (K3 finding 2: the approver never authorizes a
    ///    schema-invalid instance).
    ///
    /// # Errors
    ///
    /// Returns the first rule the spec breaks; the caller still holds
    /// the spec, and in W2 every rejection reaches the model as a tool
    /// observation it can revise against.
    pub fn validate(&self, tools: &[SidecarTool]) -> Result<(), WorkflowError> {
        self.validate_unique_ids()?;
        self.validate_dependencies()?;
        self.validate_references()?;
        self.validate_tool_inventory(tools)?;
        self.validate_step_args(tools)?;
        Ok(())
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
    fn validate_tool_inventory(&self, _tools: &[SidecarTool]) -> Result<(), WorkflowError> {
        todo!()
    }

    /// Rule 5: literal step arguments satisfy the tool's inputSchema.
    fn validate_step_args(&self, _tools: &[SidecarTool]) -> Result<(), WorkflowError> {
        todo!()
    }
}

/// One step of a workflow: a tool call, its arguments, what it exports
/// of its result, and how to undo it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
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
/// rollback's `args` may bind `$from` references to earlier steps'
/// exports, and additionally to the owning step's own exports — the
/// owning step completed if its rollback runs.
///
/// Forbidden invalid state: a mutating step with nothing to declare is
/// represented honestly as `rollback: null` on the step ([`None`]), so
/// no rollback spec exists that names no tool or undoes nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
/// Forbidden invalid state: an empty id, which names no step and cannot
/// be depended on or rendered to an approver; [`StepId::parse`]
/// rejects it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StepId(String);

impl StepId {
    /// Parse a step id from its wire string.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyStepId`] for an empty id.
    pub fn parse(id: String) -> Result<Self, WorkflowError> {
        if id.is_empty() {
            return Err(WorkflowError::EmptyStepId);
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
/// Forbidden invalid state: an empty name, which nothing could bind to;
/// [`ExportName::parse`] rejects it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExportName(String);

impl ExportName {
    /// Parse an export name from its wire string.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyExportName`] for an empty name.
    pub fn parse(name: String) -> Result<Self, WorkflowError> {
        if name.is_empty() {
            return Err(WorkflowError::EmptyExportName);
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
    pub fn parse(_path: &str) -> Result<Self, WorkflowError> {
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
    pub fn parse(_text: &str) -> Result<Self, WorkflowError> {
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
/// Forbidden invalid state: a bound that constrains nothing (neither
/// `min` nor `max`), which only mimics the reference-with-bounds wire
/// shape; [`Bounds::new`] rejects it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// The inclusive lower bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Number>,
    /// The inclusive upper bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Number>,
}

impl Bounds {
    /// Declare a bound.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyBounds`] when both halves are
    /// `None`.
    pub fn new(_min: Option<Number>, _max: Option<Number>) -> Result<Self, WorkflowError> {
        todo!()
    }
}

/// One classified node of a step's argument tree.
///
/// The wire contract (custom serde, filled in Layer 2 and pinned by its
/// tests):
///
/// - `{"$from": "step.export"}` is [`ArgValue::Reference`];
/// - `{"$from": "step.export", "min": 1, "max": 20}` is
///   [`ArgValue::BoundedReference`];
/// - anything else is [`ArgValue::Literal`] — and a literal that
///   contains a nested `$from` object is rejected by [`ArgValue::parse`]
///   rather than silently kept as an opaque literal, because a dropped
///   reference resolves to nothing at apply time and the failure would
///   surface three cards later as a missing path.
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
    /// A reference to an earlier step's export.
    Reference { from: ExportRef },
    /// A reference with resolve-time numeric bounds.
    BoundedReference { from: ExportRef, bounds: Bounds },
}

impl ArgValue {
    /// Classify one argument node against the wire contract above.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::MalformedArgNode`] for the half-specified
    /// reference shapes a silent parse would truncate (a `$from` object
    /// carrying a stray `min` without a `max`, or extra keys beside a
    /// well-formed reference).
    pub fn parse(_node: &Value) -> Result<Self, WorkflowError> {
        todo!()
    }
}

impl Serialize for ArgValue {
    fn serialize<S: Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
        todo!()
    }
}

impl<'de> Deserialize<'de> for ArgValue {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
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
