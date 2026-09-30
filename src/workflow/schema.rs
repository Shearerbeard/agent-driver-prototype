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

/// Keywords this validator recognizes without enforcing anything: they
/// annotate the schema rather than constrain the instance.
const METADATA_KEYWORDS: &[&str] = &[
    "title",
    "description",
    "$schema",
    "$id",
    "default",
    "examples",
];

/// The validation-relevant keywords the W1 narrowing admits; any other
/// keyword refuses as [`SchemaCheck::Unsupported`].
const SUBSET_KEYWORDS: &[&str] = &["type", "enum", "required", "properties", "items"];

/// The `type` names the subset admits.
const TYPE_NAMES: &[&str] = &[
    "string", "number", "integer", "boolean", "object", "array", "null",
];

/// Why an inputSchema check refused.
///
/// The two cases map to different [`WorkflowError`](super::WorkflowError)
/// variants at the caller: an unsupported keyword indicts the *schema*
/// (the tool's declaration exceeds what this validator can honestly
/// check), while a mismatch indicts the *instance* (the model's
/// arguments). Conflating them would let a tool-limitation read as a
/// model mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
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
pub(super) fn validate_instance(schema: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    // The schema is refused in full before the instance is examined:
    // an unsupported keyword indicts the declaration, so it must
    // surface even when this instance never reaches the subschema
    // holding it (an undeclared property, an empty array).
    refuse_unsupported_schema(schema)?;
    enforce_subset(schema, instance)
}

/// Whether an argument node is a reference (`{"$from": ...}`), which
/// occupies a structural position in the instance tree without being
/// type-checked at propose time.
pub(super) fn is_reference_node(node: &Value) -> bool {
    match node {
        Value::Object(node) => {
            node.contains_key("$from")
                && node
                    .keys()
                    .all(|key| matches!(key.as_str(), "$from" | "min" | "max"))
        }
        _ => false,
    }
}

/// Refuse any schema tree leaning on a keyword — or a keyword shape —
/// outside the subset, without regard to the instance.
fn refuse_unsupported_schema(schema: &Value) -> Result<(), SchemaCheck> {
    let Some(keywords) = schema.as_object() else {
        // A non-object schema (a bare `true`/`false` boolean schema, a
        // string, a number) is a shape this subset does not admit; the
        // literal fragment names it in the refusal.
        return Err(SchemaCheck::Unsupported {
            keyword: schema.to_string(),
        });
    };
    for (keyword, value) in keywords {
        if METADATA_KEYWORDS.contains(&keyword.as_str()) {
            continue;
        }
        if !SUBSET_KEYWORDS.contains(&keyword.as_str()) {
            return Err(SchemaCheck::Unsupported {
                keyword: keyword.clone(),
            });
        }
        match keyword.as_str() {
            "type" => {
                let admitted = value
                    .as_str()
                    .is_some_and(|name| TYPE_NAMES.contains(&name));
                if !admitted {
                    return Err(SchemaCheck::Unsupported {
                        keyword: keyword.clone(),
                    });
                }
            }
            "enum" => {
                if !value.is_array() {
                    return Err(SchemaCheck::Unsupported {
                        keyword: keyword.clone(),
                    });
                }
            }
            "required" => {
                let named = value
                    .as_array()
                    .is_some_and(|names| names.iter().all(|name| name.as_str().is_some()));
                if !named {
                    return Err(SchemaCheck::Unsupported {
                        keyword: keyword.clone(),
                    });
                }
            }
            "properties" => {
                let declared = value.as_object().ok_or_else(|| SchemaCheck::Unsupported {
                    keyword: keyword.clone(),
                })?;
                for property_schema in declared.values() {
                    refuse_unsupported_schema(property_schema)?;
                }
            }
            "items" => {
                if !value.is_object() {
                    return Err(SchemaCheck::Unsupported {
                        keyword: keyword.clone(),
                    });
                }
                refuse_unsupported_schema(value)?;
            }
            _ => {} // the remaining subset keywords hold no subschemas
        }
    }
    Ok(())
}

/// Enforce the subset keywords of an already-vetted schema against the
/// instance. Only instance faults refuse here; every schema fault was
/// refused by [`refuse_unsupported_schema`].
fn enforce_subset(schema: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(keywords) = schema.as_object() else {
        return Err(SchemaCheck::Unsupported {
            keyword: schema.to_string(),
        });
    };
    for (keyword, value) in keywords {
        match keyword.as_str() {
            "type" => enforce_type(value, instance)?,
            "enum" => enforce_enum(value, instance)?,
            "required" => enforce_required(value, instance)?,
            "properties" => enforce_properties(value, instance)?,
            "items" => enforce_items(value, instance)?,
            _ => {} // metadata, already vetted
        }
    }
    Ok(())
}

fn enforce_type(expected: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(expected) = expected.as_str() else {
        return Err(SchemaCheck::Unsupported {
            keyword: "type".to_string(),
        });
    };
    let matched = match expected {
        "string" => instance.is_string(),
        "number" => instance.is_number(),
        "integer" => instance.is_i64() || instance.is_u64(),
        "boolean" => instance.is_boolean(),
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "null" => instance.is_null(),
        _ => {
            return Err(SchemaCheck::Unsupported {
                keyword: "type".to_string(),
            });
        }
    };
    if matched {
        return Ok(());
    }
    Err(SchemaCheck::Mismatch {
        message: format!("expected {expected}, found {}", json_kind(instance)),
    })
}

fn enforce_enum(members: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(members) = members.as_array() else {
        return Err(SchemaCheck::Unsupported {
            keyword: "enum".to_string(),
        });
    };
    if members.contains(instance) {
        return Ok(());
    }
    Err(SchemaCheck::Mismatch {
        message: format!("value is not one of the enumerated members: {instance}"),
    })
}

fn enforce_required(names: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(names) = names.as_array() else {
        return Err(SchemaCheck::Unsupported {
            keyword: "required".to_string(),
        });
    };
    let Some(object) = instance.as_object() else {
        return Err(SchemaCheck::Mismatch {
            message: format!("expected an object, found {}", json_kind(instance)),
        });
    };
    for name in names {
        let Some(name) = name.as_str() else {
            return Err(SchemaCheck::Unsupported {
                keyword: "required".to_string(),
            });
        };
        if !object.contains_key(name) {
            return Err(SchemaCheck::Mismatch {
                message: format!("missing required property {name:?}"),
            });
        }
    }
    Ok(())
}

fn enforce_properties(declared: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(declared) = declared.as_object() else {
        return Err(SchemaCheck::Unsupported {
            keyword: "properties".to_string(),
        });
    };
    let Some(object) = instance.as_object() else {
        return Err(SchemaCheck::Mismatch {
            message: format!("expected an object, found {}", json_kind(instance)),
        });
    };
    // Open world: a property without a declared subschema passes, and
    // a reference node occupies its position structurally — its type
    // is checked at resolve time (W3), not here.
    for (name, property_schema) in declared {
        let Some(property_value) = object.get(name) else {
            continue;
        };
        if is_reference_node(property_value) {
            continue; // structural: type-checked at resolve time (W3)
        }
        enforce_subset(property_schema, property_value)?;
    }
    Ok(())
}

fn enforce_items(items: &Value, instance: &Value) -> Result<(), SchemaCheck> {
    let Some(elements) = instance.as_array() else {
        return Err(SchemaCheck::Mismatch {
            message: format!("expected an array, found {}", json_kind(instance)),
        });
    };
    for element in elements {
        if is_reference_node(element) {
            continue; // structural, like any reference node position
        }
        enforce_subset(items, element)?;
    }
    Ok(())
}

/// The JSON kind of `instance`, phrased the way `type` names it, for
/// mismatch messages.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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
