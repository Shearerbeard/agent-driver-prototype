//! Resolve `$from` references, capture declared exports, and parse step
//! result text as JSON.
//!
//! All resolution happens executor-side at apply time. The model declares
//! `$from` references and `min`/`max` bounds at propose time; it never
//! supplies or verifies the resolved values.

#![expect(dead_code, unused_imports, unused_variables)]

use std::collections::BTreeMap;

use serde_json::Value;

use crate::mcp_client::SidecarContent;
use crate::workflow::plan::{ArgValue, Bounds, ExportName, ExportRef, ExportSpec, StepId};

/// Why resolving a reference or capturing an export failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ResolveError {
    /// The tool result text was not valid JSON.
    #[error("step result is not valid JSON: {0}")]
    ResultNotJson(String),
    /// A declared export path did not point at a value in the parsed
    /// result.
    #[error("export '{export}' of step '{step}' is not present at path '{path}'")]
    ExportPathMissing {
        step: StepId,
        export: ExportName,
        path: String,
    },
    /// A `$from` reference named a step/export pair that has not been
    /// captured.
    #[error("reference {0} resolves to a step or export that does not exist")]
    ReferenceMissing(ExportRef),
    /// A resolved numeric value violated the declared bounds.
    #[error("resolved value {value} violates declared bounds")]
    BoundsViolation { value: Value, bounds: Bounds },
    /// A bounded reference resolved to a value that is not a number.
    #[error("resolved value for {reference} is not a number, so bounds cannot be checked")]
    NotANumber { reference: ExportRef },
    /// An argument node was malformed after validation.
    #[error("argument node is malformed after validation: {0}")]
    MalformedArgNode(String),
}

/// The binding environment: every completed step's captured exports.
///
/// This is the executor-side store that `$from` references resolve
/// against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportEnvironment {
    values: BTreeMap<StepId, BTreeMap<ExportName, Value>>,
}

impl ExportEnvironment {
    /// An empty environment.
    pub fn new() -> Self {
        todo!()
    }

    /// Record the exports captured from one completed step.
    pub fn insert(&mut self, step: StepId, exports: BTreeMap<ExportName, Value>) {
        todo!()
    }

    /// Resolve a `$from` reference to its captured value.
    pub fn resolve(&self, reference: &ExportRef) -> Option<&Value> {
        todo!()
    }
}

/// Parse the text returned by a successful tool call as JSON.
///
/// Non-JSON is a step failure; the missing-path error shape matches the
/// contract that a step result must be a JSON object the declared exports
/// can be lifted from.
pub fn parse_tool_result(content: &SidecarContent) -> Result<Value, ResolveError> {
    todo!()
}

/// Capture every declared export of one step out of its parsed result.
pub fn capture_exports(
    step: &StepId,
    exports: &BTreeMap<ExportName, ExportSpec>,
    result: &Value,
) -> Result<BTreeMap<ExportName, Value>, ResolveError> {
    todo!()
}

/// Resolve every `$from` reference in an argument tree against the
/// captured exports, substituting the resolved value in place and
/// checking declared bounds.
pub fn resolve_arguments(
    args: &Value,
    env: &ExportEnvironment,
    owner: &StepId,
) -> Result<Value, ResolveError> {
    todo!()
}

/// Check a resolved numeric value against declared bounds.
pub fn check_bounds(value: &Value, bounds: &Bounds) -> Result<(), ResolveError> {
    todo!()
}
