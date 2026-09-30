//! A deliberately minimal JSON-Schema validator: the `inputSchema`
//! subset the rig's discovered tools actually declare, and nothing
//! more.
//!
//! W1 narrowing, recorded in `DESIGN.md`: the card's scope names
//! `src/workflow/` only, so adding a schema crate to `Cargo.toml` is a
//! stop-and-report rather than a quiet new dependency. The subset below
//! covers the validation-relevant keywords the sidecar tools use —
//! `type` (with `string`, `number`, `integer`, `boolean`, `object`,
//! `array`, `null`), `properties`, `required`, `items`, and `enum`.
//! Metadata keywords (`title`, `description`, `$schema`, `$id`,
//! `default`, `examples`) are ignored. Any other validation-relevant
//! keyword fails loud as [`SchemaCheck::Unsupported`] rather than
//! passing silently: the approver-authorization rule (K3 finding 2)
//! makes unearned validation confidence the worst outcome of the three.
//!
//! Reference nodes are structural here: a `{"$from": ...}` object
//! occupies the position its property names, and its bounds (when
//! present) must be numbers, but its *type* is checked at resolve time
//! by the W3 executor against the value the referenced export actually
//! produced. Propose time checks what is knowable at propose time.

use serde_json::Value;

/// Why an inputSchema check refused.
///
/// The two cases map to different [`WorkflowError`](super::WorkflowError)
/// variants at the caller: an unsupported keyword indicts the *schema*
/// (the tool's declaration exceeds what this validator can honestly
/// check), while a mismatch indicts the *instance* (the model's
/// arguments). Conflating them would let a tool-limitation read as a
/// model mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    dead_code,
    reason = "W1 Layer 1: used when validate_step_args is filled"
)]
pub(super) enum SchemaCheck {
    /// The schema leans on a validation-relevant keyword outside the
    /// subset; `keyword` names it.
    Unsupported { keyword: String },
    /// The instance broke the rule `message` describes.
    Mismatch { message: String },
}

/// Validate `instance` against `schema`, both as raw JSON.
///
/// # Errors
///
/// Returns the first refusal per [`SchemaCheck`]: the caller (the spec
/// validator) owns the step and tool names needed to phrase either case
/// as a `WorkflowError`.
#[expect(
    dead_code,
    reason = "W1 Layer 1: called when validate_step_args is filled"
)]
pub(super) fn validate_instance(_schema: &Value, _instance: &Value) -> Result<(), SchemaCheck> {
    todo!()
}

/// Whether an argument node is a reference (`{"$from": ...}`), which
/// occupies a structural position in the instance tree without being
/// type-checked at propose time.
#[expect(
    dead_code,
    reason = "W1 Layer 1: called when the reference walk is filled"
)]
pub(super) fn is_reference_node(_node: &Value) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn type_keywords_accept_matching_scalars() {
        let schema = json!({"type": "string"});
        assert!(validate_instance(&schema, &json!("x")).is_ok());
        assert!(validate_instance(&schema, &json!(3)).is_err());

        let schema = json!({"type": "integer"});
        assert!(validate_instance(&schema, &json!(3)).is_ok());
        assert!(validate_instance(&schema, &json!(3.5)).is_err());
        assert!(validate_instance(&schema, &json!("3")).is_err());

        let schema = json!({"type": "number"});
        assert!(validate_instance(&schema, &json!(3.5)).is_ok());

        let schema = json!({"type": "boolean"});
        assert!(validate_instance(&schema, &json!(true)).is_ok());
    }

    #[test]
    fn objects_check_properties_and_required() {
        let schema = json!({
            "type": "object",
            "properties": {"app": {"type": "string"}, "replicas": {"type": "integer"}},
            "required": ["app", "replicas"]
        });
        assert!(validate_instance(&schema, &json!({"app": "x", "replicas": 2})).is_ok());
        assert!(validate_instance(&schema, &json!({"app": "x"})).is_err());
        assert!(validate_instance(&schema, &json!({"app": 1, "replicas": 2})).is_err());
    }

    #[test]
    fn arrays_check_items() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        assert!(validate_instance(&schema, &json!(["a", "b"])).is_ok());
        assert!(validate_instance(&schema, &json!(["a", 1])).is_err());
    }

    #[test]
    fn enums_accept_only_their_members() {
        let schema = json!({"enum": ["red", "green"]});
        assert!(validate_instance(&schema, &json!("red")).is_ok());
        assert!(validate_instance(&schema, &json!("blue")).is_err());
    }

    #[test]
    fn metadata_keywords_are_ignored() {
        let schema = json!({
            "title": "scale",
            "description": "scales an app",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "default": {},
            "examples": [],
            "type": "object"
        });
        assert!(validate_instance(&schema, &json!({})).is_ok());
    }

    #[test]
    fn validation_relevant_keywords_outside_the_subset_refuse() {
        for keyword in [
            "pattern",
            "minimum",
            "maximum",
            "minLength",
            "maxLength",
            "additionalProperties",
            "patternProperties",
            "allOf",
            "anyOf",
            "oneOf",
            "not",
            "format",
            "minItems",
            "maxItems",
            "uniqueItems",
            "multipleOf",
            "minProperties",
            "if",
        ] {
            let mut schema = json!({"type": "object"});
            schema[keyword] = json!(true);
            let refusal = validate_instance(&schema, &json!({})).unwrap_err();
            match refusal {
                SchemaCheck::Unsupported { keyword: found } => assert_eq!(found, keyword),
                SchemaCheck::Mismatch { message } => {
                    panic!("{keyword} was read as a mismatch instead of refusing: {message}")
                }
            }
        }
    }

    #[test]
    fn reference_nodes_are_structural() {
        assert!(is_reference_node(&json!({"$from": "state.x"})));
        assert!(is_reference_node(&json!({
            "$from": "state.x", "min": 1, "max": 2
        })));
        assert!(!is_reference_node(&json!("x")));
        assert!(!is_reference_node(&json!({"plain": 1})));
        assert!(!is_reference_node(&json!({"min": 1})));
    }
}
