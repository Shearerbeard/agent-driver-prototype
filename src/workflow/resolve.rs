//! Resolve `$from` references, capture declared exports, and parse step
//! result text as JSON.
//!
//! All resolution happens executor-side at apply time. The model declares
//! `$from` references and `min`/`max` bounds at propose time; it never
//! supplies or verifies the resolved values.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::{Map, Number, Value};

use crate::mcp_client::SidecarContent;
use crate::workflow::plan::{ArgValue, Bounds, ExportName, ExportRef, ExportSpec, StepId};

use super::error::WorkflowError;

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
        Self {
            values: BTreeMap::new(),
        }
    }

    /// Record the exports captured from one completed step.
    pub fn insert(&mut self, step: StepId, exports: BTreeMap<ExportName, Value>) {
        self.values.insert(step, exports);
    }

    /// Resolve a `$from` reference to its captured value.
    ///
    /// `None` means the named step has not completed, or completed
    /// without capturing the named export — the executor's
    /// [`ResolveError::ReferenceMissing`].
    pub fn resolve(&self, reference: &ExportRef) -> Option<&Value> {
        self.values.get(reference.step())?.get(reference.export())
    }
}

/// Parse the text returned by a successful tool call as JSON.
///
/// Non-JSON is a step failure; the missing-path error shape matches the
/// contract that a step result must be a JSON object the declared exports
/// can be lifted from. Only JSON *validity* is gated here: a well-formed
/// but non-object result parses, and surfaces as
/// [`ResolveError::ExportPathMissing`] when a declared export lifts from
/// it — the same failure shape the card names for non-JSON, reached
/// through one path-missing surface instead of a second result-shape
/// rule.
///
/// # Errors
///
/// Returns [`ResolveError::ResultNotJson`] for any body `serde_json`
/// cannot parse, including the empty text an empty sidecar pane yields.
pub fn parse_tool_result(content: &SidecarContent) -> Result<Value, ResolveError> {
    serde_json::from_str(content.as_str())
        .map_err(|error| ResolveError::ResultNotJson(error.to_string()))
}

/// Capture every declared export of one step out of its parsed result.
///
/// Each declared `$.`-path lifts through its JSON-pointer form (the
/// conversion `ExportSpec::to_json_pointer` exists for); a path that
/// names nothing in the result fails the capture, and with it the step.
///
/// # Errors
///
/// Returns [`ResolveError::ExportPathMissing`] naming the step, the
/// export, and the declared path form, for the first declared path the
/// result does not answer.
pub fn capture_exports(
    step: &StepId,
    exports: &BTreeMap<ExportName, ExportSpec>,
    result: &Value,
) -> Result<BTreeMap<ExportName, Value>, ResolveError> {
    let mut captured = BTreeMap::new();
    for (name, spec) in exports {
        let Some(value) = result.pointer(&spec.to_json_pointer()) else {
            return Err(ResolveError::ExportPathMissing {
                step: step.clone(),
                export: name.clone(),
                path: spec.as_path(),
            });
        };
        captured.insert(name.clone(), value.clone());
    }
    Ok(captured)
}

/// Classify one argument node, mapping the classification failure onto
/// this module's error. `ArgValue::parse` fails only with
/// [`WorkflowError::MalformedArgNode`], which already carries the raw
/// fragment; the fallback arm keeps the match total without re-wrapping
/// a message that would then name the node twice.
fn classify(node: &Value) -> Result<ArgValue, ResolveError> {
    ArgValue::parse(node).map_err(|error| match error {
        WorkflowError::MalformedArgNode(fragment) => ResolveError::MalformedArgNode(fragment),
        other => ResolveError::MalformedArgNode(other.to_string()),
    })
}

/// Substitute references in one argument node.
///
/// The walk mirrors plan validation's classification walk position for
/// position: a literal object's values and a literal array's elements
/// recurse (the anywhere-in-tree rule), so a reference binds at any
/// object-value or array-element position, at any depth; scalar
/// literals pass through untouched. Reference nodes resolve against the
/// environment and are replaced by the captured value verbatim; bounded
/// references additionally gate on numeric-ness and the declared
/// envelope (see [`check_bounds`] for the split between this walk and
/// that check).
fn resolve_node(node: &Value, env: &ExportEnvironment) -> Result<Value, ResolveError> {
    match classify(node)? {
        ArgValue::Literal(value) => match value {
            Value::Object(entries) => {
                let mut resolved = Map::with_capacity(entries.len());
                for (key, value) in entries {
                    resolved.insert(key, resolve_node(&value, env)?);
                }
                Ok(Value::Object(resolved))
            }
            Value::Array(items) => {
                let mut resolved = Vec::with_capacity(items.len());
                for item in items {
                    resolved.push(resolve_node(&item, env)?);
                }
                Ok(Value::Array(resolved))
            }
            scalar => Ok(scalar),
        },
        ArgValue::Reference { from } => {
            let Some(value) = env.resolve(&from) else {
                return Err(ResolveError::ReferenceMissing(from));
            };
            Ok(value.clone())
        }
        ArgValue::BoundedReference { from, bounds } => {
            let Some(value) = env.resolve(&from) else {
                return Err(ResolveError::ReferenceMissing(from));
            };
            let value = value.clone();
            if value.as_number().is_none() {
                return Err(ResolveError::NotANumber { reference: from });
            }
            check_bounds(&value, &bounds)?;
            Ok(value)
        }
    }
}

/// Resolve every `$from` reference in an argument tree against the
/// captured exports, substituting the resolved value in place and
/// checking declared bounds.
///
/// `owner` names the step whose (or whose rollback's) arguments these
/// are. The current [`ResolveError`] vocabulary attributes failures to
/// the step at the executor's run-record level, so no variant carries
/// the owner yet; the parameter is the seam for the day one does.
///
/// # Errors
///
/// Returns [`ResolveError::MalformedArgNode`] for a node the
/// classification rejects (a shape validation should have caught,
/// failing loud rather than trusted), [`ResolveError::ReferenceMissing`]
/// for a reference the environment does not answer,
/// [`ResolveError::NotANumber`] for a bounded reference that resolved
/// off the numbers, and [`ResolveError::BoundsViolation`] for a resolved
/// number outside its declared envelope.
#[expect(unused_variables)]
pub fn resolve_arguments(
    args: &Value,
    env: &ExportEnvironment,
    owner: &StepId,
) -> Result<Value, ResolveError> {
    resolve_node(args, env)
}

/// Compare two JSON numbers exactly.
///
/// Integer-integer pairs widen to `i128` (every `i64` and `u64` fits),
/// so they are exact at any magnitude. Float-float pairs compare as
/// `f64` values, which is exact between two floats. The mixed pair —
/// the one that used to lose precision — compares the float against
/// the integer through the float's floor as `i128` (clamped where
/// `i128` cannot hold it) with the fraction breaking ties, so an
/// integer beyond ±2⁵³ never rounds onto a float's value:
/// `9007199254740993i64` is greater than `9007199254740992.0`, not
/// equal to it. NaN cannot come out of `serde_json`; the fallback only
/// keeps the function total.
fn compare_numbers(left: &Number, right: &Number) -> Ordering {
    if let (Some(left), Some(right)) = (as_i128(left), as_i128(right)) {
        return left.cmp(&right);
    }
    // At least one side is a float (serde_json integers never exceed
    // i64/u64, so everything wider is a float).
    let (Some(left_f), Some(right_f)) = (left.as_f64(), right.as_f64()) else {
        return Ordering::Equal;
    };
    match (as_i128(left), as_i128(right)) {
        (Some(left_i), None) => cmp_f64_vs_i128(right_f, left_i).reverse(),
        (None, Some(right_i)) => cmp_f64_vs_i128(left_f, right_i),
        _ => left_f.partial_cmp(&right_f).unwrap_or(Ordering::Equal),
    }
}

/// A `serde_json` number as `i128` when it is an integer.
fn as_i128(number: &Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

/// Compare a float against an exact `i128` without converting the
/// integer: the float's floor decides as an integer (clamped where
/// `i128` cannot hold it), and a nonzero fraction breaks a tie upward.
#[allow(clippy::cast_possible_truncation)]
fn cmp_f64_vs_i128(value: f64, integer: i128) -> Ordering {
    if value.is_nan() {
        return Ordering::Equal;
    }
    let floor = value.floor();
    // 2^127 as f64: the first integer i128 cannot hold.
    const I128_LIMIT: f64 = 1.701_411_834_604_692_3e38;
    if floor >= I128_LIMIT {
        return Ordering::Greater;
    }
    if floor < -I128_LIMIT {
        return Ordering::Less;
    }
    // floor is integral and within i128 range, so the cast is exact.
    match (floor as i128).cmp(&integer) {
        Ordering::Equal if value > floor => Ordering::Greater,
        ord => ord,
    }
}

/// The violation error for a value and envelope, built once for the
/// three call sites below.
fn bounds_violation(value: &Value, bounds: &Bounds) -> ResolveError {
    ResolveError::BoundsViolation {
        value: value.clone(),
        bounds: bounds.clone(),
    }
}

/// Check a resolved numeric value against declared bounds.
///
/// Both sides are inclusive; either side may be undeclared. The
/// not-a-number gate lives one call up, in [`resolve_arguments`], where
/// the offending reference is still in hand —
/// [`ResolveError::NotANumber`] names it, and this function's signature
/// does not carry one. A direct caller that skips that gate gets the
/// loud rejection of [`ResolveError::BoundsViolation`], never a silent
/// pass.
///
/// # Errors
///
/// Returns [`ResolveError::BoundsViolation`] when the value sits below
/// a declared `min` or above a declared `max` — or is not a number at
/// all, which no numeric envelope can contain.
pub fn check_bounds(value: &Value, bounds: &Bounds) -> Result<(), ResolveError> {
    let Some(number) = value.as_number() else {
        return Err(bounds_violation(value, bounds));
    };
    if bounds
        .min()
        .is_some_and(|min| compare_numbers(number, min) == Ordering::Less)
    {
        return Err(bounds_violation(value, bounds));
    }
    if bounds
        .max()
        .is_some_and(|max| compare_numbers(number, max) == Ordering::Greater)
    {
        return Err(bounds_violation(value, bounds));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step_id(id: &str) -> StepId {
        StepId::parse(id.to_owned()).unwrap()
    }

    fn export_name(name: &str) -> ExportName {
        ExportName::parse(name.to_owned()).unwrap()
    }

    fn reference(text: &str) -> ExportRef {
        ExportRef::parse(text).unwrap()
    }

    fn path(declared: &str) -> ExportSpec {
        ExportSpec::parse(declared).unwrap()
    }

    fn number(value: serde_json::Value) -> Number {
        value.as_number().unwrap().clone()
    }

    fn bounds(min: Option<Number>, max: Option<Number>) -> Bounds {
        Bounds::new(min, max).unwrap()
    }

    fn environment(step: &str, export: &str, value: serde_json::Value) -> ExportEnvironment {
        let mut env = ExportEnvironment::new();
        let mut exports = BTreeMap::new();
        exports.insert(export_name(export), value);
        env.insert(step_id(step), exports);
        env
    }

    // ---- parse_tool_result ----

    #[test]
    fn a_json_body_parses_and_a_non_json_body_is_rejected() {
        let content = SidecarContent::new(r#"{"deployment":{"replicas":3}}"#.to_owned());
        let parsed = parse_tool_result(&content).unwrap();
        assert_eq!(parsed, json!({"deployment": {"replicas": 3}}));
        for body in ["", "not json", "{\"deployment\":"] {
            assert!(
                matches!(
                    parse_tool_result(&SidecarContent::new(body.to_owned())),
                    Err(ResolveError::ResultNotJson(_))
                ),
                "accepted non-JSON body {body:?}"
            );
        }
    }

    #[test]
    fn a_well_formed_non_object_result_parses_and_misses_paths_later() {
        // The input contract gates JSON validity only: a scalar or
        // array result is not rejected here, it surfaces as
        // ExportPathMissing when a declared export lifts from it —
        // the same missing-path failure shape non-JSON reaches the
        // step through.
        let scalar = SidecarContent::new("3".to_owned());
        assert_eq!(parse_tool_result(&scalar).unwrap(), json!(3));
        let array = SidecarContent::new(r#"["a","b"]"#.to_owned());
        assert_eq!(parse_tool_result(&array).unwrap(), json!(["a", "b"]));
    }

    // ---- capture_exports ----

    #[test]
    fn capture_lifts_every_declared_export_from_the_result() {
        let mut exports = BTreeMap::new();
        exports.insert(export_name("replicas"), path("$.deployment.replicas"));
        exports.insert(export_name("first"), path("$.items[0]"));
        let result = json!({"deployment": {"replicas": 3}, "items": ["a", "b"]});
        let captured = capture_exports(&step_id("state"), &exports, &result).unwrap();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[&export_name("replicas")], json!(3));
        assert_eq!(captured[&export_name("first")], json!("a"));
    }

    #[test]
    fn a_missing_path_names_the_step_export_and_declared_form() {
        let mut exports = BTreeMap::new();
        exports.insert(export_name("ghost"), path("$.deployment.missing"));
        let result = json!({"deployment": {"replicas": 3}});
        assert_eq!(
            capture_exports(&step_id("state"), &exports, &result),
            Err(ResolveError::ExportPathMissing {
                step: step_id("state"),
                export: export_name("ghost"),
                path: "$.deployment.missing".to_owned(),
            })
        );
    }

    // ---- ExportEnvironment ----

    #[test]
    fn an_environment_resolves_a_captured_reference_and_misses_others() {
        let env = environment("state", "replicas", json!(3));
        assert_eq!(env.resolve(&reference("state.replicas")), Some(&json!(3)));
        assert_eq!(env.resolve(&reference("state.other")), None);
        assert_eq!(env.resolve(&reference("other.replicas")), None);
        let empty = ExportEnvironment::new();
        assert_eq!(empty.resolve(&reference("state.replicas")), None);
    }

    // ---- resolve_arguments: literals ----

    #[test]
    fn literal_subtrees_pass_through_unchanged() {
        let env = ExportEnvironment::new();
        let args = json!({
            "app": "payments",
            "flags": {"force": true, "notes": [1, "two", null, false]},
            "count": 0
        });
        assert_eq!(
            resolve_arguments(&args, &env, &step_id("scale")).unwrap(),
            args
        );
    }

    // ---- resolve_arguments: references ----

    #[test]
    fn a_reference_substitutes_the_captured_value_in_place() {
        let env = environment("state", "replicas", json!(6));
        let args = json!({"app": "payments", "replicas": {"$from": "state.replicas"}});
        assert_eq!(
            resolve_arguments(&args, &env, &step_id("scale")).unwrap(),
            json!({"app": "payments", "replicas": 6})
        );
    }

    #[test]
    fn references_resolve_at_every_tree_position() {
        let env = environment("state", "replicas", json!(6));
        let args = json!({
            "labels": {"source": {"$from": "state.replicas"}},
            "list": [0, {"$from": "state.replicas"}]
        });
        assert_eq!(
            resolve_arguments(&args, &env, &step_id("scale")).unwrap(),
            json!({"labels": {"source": 6}, "list": [0, 6]})
        );
    }

    #[test]
    fn a_missing_reference_names_the_reference() {
        let env = environment("state", "replicas", json!(6));
        let args = json!({"replicas": {"$from": "state.missing"}});
        assert_eq!(
            resolve_arguments(&args, &env, &step_id("scale")),
            Err(ResolveError::ReferenceMissing(reference("state.missing")))
        );
    }

    #[test]
    fn a_malformed_node_is_rejected_with_its_fragment() {
        let env = ExportEnvironment::new();
        let malformed = json!({"$from": "state"});
        assert_eq!(
            resolve_arguments(&malformed, &env, &step_id("scale")),
            Err(ResolveError::MalformedArgNode(malformed.to_string()))
        );
        // Shapes ArgValue::parse rejects: a non-string $from value, and
        // a key beside $from.
        for node in [
            json!({"$from": 3}),
            json!({"$from": "state.x", "extra": true}),
        ] {
            assert!(
                matches!(
                    resolve_arguments(&node, &env, &step_id("scale")),
                    Err(ResolveError::MalformedArgNode(_))
                ),
                "accepted malformed node {node}"
            );
        }
    }

    // ---- resolve_arguments + check_bounds: envelopes ----

    #[test]
    fn a_bounded_reference_inside_its_envelope_substitutes() {
        let env = environment("state", "replicas", json!(6));
        let args = json!({"replicas": {"$from": "state.replicas", "min": 1, "max": 20}});
        let resolved = resolve_arguments(&args, &env, &step_id("scale")).unwrap();
        assert_eq!(resolved, json!({"replicas": 6}));
    }

    #[test]
    fn a_bound_violation_reports_the_value_and_envelope() {
        let envelope = bounds(Some(number(json!(1))), Some(number(json!(20))));
        let args = json!({"replicas": {"$from": "state.replicas", "min": 1, "max": 20}});
        let under = environment("state", "replicas", json!(0));
        assert_eq!(
            resolve_arguments(&args, &under, &step_id("scale")),
            Err(ResolveError::BoundsViolation {
                value: json!(0),
                bounds: envelope.clone(),
            })
        );
        let over = environment("state", "replicas", json!(21));
        assert_eq!(
            resolve_arguments(&args, &over, &step_id("scale")),
            Err(ResolveError::BoundsViolation {
                value: json!(21),
                bounds: envelope,
            })
        );
    }

    #[test]
    fn a_non_numeric_bounded_reference_is_rejected_with_the_reference() {
        let env = environment("state", "replicas", json!("six"));
        let args = json!({"replicas": {"$from": "state.replicas", "min": 1}});
        assert_eq!(
            resolve_arguments(&args, &env, &step_id("scale")),
            Err(ResolveError::NotANumber {
                reference: reference("state.replicas"),
            })
        );
    }

    #[test]
    fn integer_and_float_json_numbers_compare_exactly() {
        // A bound declared as a JSON integer against a float result,
        // and the reverse: the exactness policy spans the two widths.
        let envelope = bounds(Some(number(json!(1))), None);
        assert!(check_bounds(&json!(1.5), &envelope).is_ok());
        assert_eq!(
            check_bounds(&json!(0.5), &envelope),
            Err(ResolveError::BoundsViolation {
                value: json!(0.5),
                bounds: envelope.clone(),
            })
        );
        let float_floor = bounds(Some(number(json!(0.5))), None);
        assert!(check_bounds(&json!(1), &float_floor).is_ok());
    }

    #[test]
    fn mixed_width_beyond_two_pow_fifty_three_compares_exactly() {
        // An integer one ulp above 2^53 must not round onto the float
        // an ulp below it. The integer 9007199254740993 is greater than
        // the float 9007199254740992.0 (violates that max) and below
        // the next representable float, 9007199254740994.0 (violates
        // that min): exact comparison keeps both hairs.
        let just_above = json!(9_007_199_254_740_993_i64);
        let float_below = number(json!(9_007_199_254_740_992.0_f64));
        let float_above = number(json!(9_007_199_254_740_994.0_f64));

        let max_below = bounds(None, Some(float_below.clone()));
        assert_eq!(
            check_bounds(&just_above, &max_below),
            Err(ResolveError::BoundsViolation {
                value: just_above.clone(),
                bounds: max_below,
            }),
            "9007199254740993 > 9007199254740992.0; the f64 conversion must not erase the ulp"
        );

        let min_above = bounds(Some(float_above), None);
        assert_eq!(
            check_bounds(&just_above, &min_above),
            Err(ResolveError::BoundsViolation {
                value: just_above.clone(),
                bounds: min_above,
            }),
            "9007199254740993 is below the 9007199254740994.0 min exactly, a hair no f64 conversion may erase"
        );

        // The mirror: a float value against integer bounds astride 2^53.
        let max_int = bounds(None, Some(number(json!(9_007_199_254_740_993_i64))));
        assert!(check_bounds(&json!(9_007_199_254_740_992.0_f64), &max_int).is_ok());
        let min_int = bounds(Some(number(json!(9_007_199_254_740_993_i64))), None);
        assert_eq!(
            check_bounds(&json!(9_007_199_254_740_992.0_f64), &min_int),
            Err(ResolveError::BoundsViolation {
                value: json!(9_007_199_254_740_992.0_f64),
                bounds: min_int,
            }),
            "9007199254740992.0 is below the 9007199254740993 min; the integer must not round onto the float"
        );
    }

    #[test]
    fn check_bounds_is_inclusive_on_both_sides() {
        let envelope = bounds(Some(number(json!(1))), Some(number(json!(20))));
        assert!(check_bounds(&json!(1), &envelope).is_ok());
        assert!(check_bounds(&json!(20), &envelope).is_ok());
        assert!(check_bounds(&json!(20.0), &envelope).is_ok());
    }

    #[test]
    fn a_non_numeric_value_reaching_check_bounds_directly_fails_loud() {
        // Unreachable through resolve_arguments, which raises NotANumber
        // (it carries the reference this function's signature lacks); a
        // direct caller skipping that gate gets the loud rejection, not
        // a silent pass.
        let envelope = bounds(Some(number(json!(1))), None);
        assert_eq!(
            check_bounds(&json!("six"), &envelope),
            Err(ResolveError::BoundsViolation {
                value: json!("six"),
                bounds: envelope,
            })
        );
    }
}
